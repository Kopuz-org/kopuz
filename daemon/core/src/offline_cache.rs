//! The offline playback cache: songs from sources across the internet, kept from their own stream and evicted least recently played first past the cap.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use api::ApiError;
use server::playback_ref::ResolvedStreamRef;
use server::stream::stream_buffer::{Captured, CompleteSink};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, watch};

/// How long a prefetch may take before it is given up.
const PREFETCH_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Where the cache keeps its songs: the data dir's `offline-cache`, or `offline-cache-debug` in a debug build, as the database splits.
pub fn default_dir() -> PathBuf {
    let name = if cfg!(debug_assertions) {
        "offline-cache-debug"
    } else {
        "offline-cache"
    };
    #[cfg(target_os = "android")]
    {
        db::config_dir().join(name)
    }
    #[cfg(not(target_os = "android"))]
    {
        config::flavor::project_dirs()
            .map(|dirs| dirs.data_dir().join(name))
            .unwrap_or_else(|| std::env::temp_dir().join(format!("kopuz-{name}")))
    }
}

/// One song the cache holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedCopy {
    pub source: String,
    pub path: PathBuf,
    pub size: u64,
    pub content_type: String,
    pub cached_at: i64,
    pub last_played_at: i64,
}

impl CachedCopy {
    fn of(row: db::CachedTrackRow) -> (String, Self) {
        (
            row.track_key,
            Self {
                source: row.source,
                path: PathBuf::from(row.path),
                size: row.size,
                content_type: row.content_type,
                cached_at: row.cached_at,
                last_played_at: row.last_played_at,
            },
        )
    }

    fn row(&self, track_key: &str) -> db::CachedTrackRow {
        db::CachedTrackRow {
            track_key: track_key.to_string(),
            source: self.source.clone(),
            path: self.path.to_string_lossy().into_owned(),
            size: self.size,
            content_type: self.content_type.clone(),
            cached_at: self.cached_at,
            last_played_at: self.last_played_at,
        }
    }
}

/// The next song to keep ahead of time, fetched at the stream quality the playing one used.
#[derive(Clone)]
pub struct PrefetchRequest {
    pub item_id: String,
    pub source_id: String,
    pub source: Option<server::source::ActiveSource>,
    pub quality: config::StreamQuality,
}

/// How a prefetch gets a song's whole file; tests stand in for the network.
#[async_trait::async_trait]
pub trait Fetch: Send + Sync {
    async fn fetch(&self, request: &PrefetchRequest) -> Result<Captured, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Limits {
    enabled: bool,
    max_bytes: u64,
}

impl Limits {
    fn of(settings: &config::Settings) -> Self {
        Self {
            enabled: settings.library.offline_cache_enabled,
            max_bytes: settings.library.offline_cache_max_bytes,
        }
    }
}

struct Inner {
    /// `None` keeps nothing, as a session built without a home for its cache has.
    dir: Option<PathBuf>,
    db: Option<db::Db>,
    runtime: Option<tokio::runtime::Handle>,
    entries: RwLock<HashMap<String, CachedCopy>>,
    limits: RwLock<Limits>,
    /// Songs a stream or a prefetch is filling a copy of now, so none is fetched twice.
    filling: Mutex<HashSet<String>>,
    /// How many of those are the playing song's own stream, which a prefetch waits out.
    foreground: watch::Sender<usize>,
    prefetch: Mutex<Option<(String, tokio::task::JoinHandle<()>)>>,
    fetcher: Arc<dyn Fetch>,
    /// Held across a row write and the map update that follows it.
    writer: tokio::sync::Mutex<()>,
    changed: Arc<Notify>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            dir: None,
            db: None,
            runtime: None,
            entries: RwLock::default(),
            limits: RwLock::new(Limits::of(&config::Settings::default())),
            filling: Mutex::default(),
            foreground: watch::channel(0).0,
            prefetch: Mutex::default(),
            fetcher: Arc::new(StreamFetch),
            writer: tokio::sync::Mutex::default(),
            changed: Arc::default(),
        }
    }
}

#[derive(Clone, Default)]
pub struct OfflineCache {
    inner: Arc<Inner>,
}

/// Keeps a song marked as filling until it is stored or given up.
struct Filling {
    cache: OfflineCache,
    item_id: String,
    foreground: bool,
}

impl Drop for Filling {
    fn drop(&mut self) {
        lock(&self.cache.inner.filling).remove(&self.item_id);
        if self.foreground {
            self.cache
                .inner
                .foreground
                .send_modify(|streams| *streams = streams.saturating_sub(1));
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn file_name(item_id: &str, extension: &str) -> String {
    let mut name = String::with_capacity(72);
    for byte in Sha256::digest(item_id.as_bytes()) {
        let _ = write!(name, "{byte:02x}");
    }
    name.push('.');
    name.push_str(extension);
    name
}

impl OfflineCache {
    /// The cache index as stored, less any song whose file is gone; strays in `dir` the index does not name are deleted.
    pub async fn load(
        db: &db::Db,
        dir: PathBuf,
        settings: &config::Settings,
    ) -> Result<Self, db::DbError> {
        Self::load_with(db, dir, settings, Arc::new(StreamFetch)).await
    }

    pub async fn load_with(
        db: &db::Db,
        dir: PathBuf,
        settings: &config::Settings,
        fetcher: Arc<dyn Fetch>,
    ) -> Result<Self, db::DbError> {
        let mut entries = HashMap::new();
        let mut gone = Vec::new();
        for row in db.offline_cache().await? {
            let (key, copy) = CachedCopy::of(row);
            match copy.path.is_file() {
                true => {
                    entries.insert(key, copy);
                }
                false => gone.push(key),
            }
        }
        if !gone.is_empty() {
            db.remove_offline_cache(&gone).await?;
            tracing::info!(
                count = gone.len(),
                "forgot cached songs whose files are gone"
            );
        }
        sweep_strays(&dir, &entries);
        let cache = Self {
            inner: Arc::new(Inner {
                dir: Some(dir),
                db: Some(db.clone()),
                runtime: tokio::runtime::Handle::try_current().ok(),
                entries: RwLock::new(entries),
                limits: RwLock::new(Limits::of(settings)),
                fetcher,
                ..Inner::default()
            }),
        };
        cache.evict().await;
        Ok(cache)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, CachedCopy>> {
        self.inner
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, CachedCopy>> {
        self.inner
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn limits(&self) -> Limits {
        *self
            .inner
            .limits
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Signalled whenever a song joins or leaves the cache, for whoever re-announces the listings.
    pub fn changed(&self) -> Arc<Notify> {
        self.inner.changed.clone()
    }

    /// Follow the settings: turning the cache off only stops it adding songs, and a lower cap evicts at once.
    pub fn set_limits(&self, settings: &config::Settings) {
        let next = Limits::of(settings);
        let previous = std::mem::replace(
            &mut *self
                .inner
                .limits
                .write()
                .unwrap_or_else(PoisonError::into_inner),
            next,
        );
        if next.max_bytes < previous.max_bytes
            && let Some(runtime) = self.inner.runtime.clone()
        {
            let cache = self.clone();
            runtime.spawn(async move { cache.evict().await });
        }
    }

    /// Whether new songs are kept at all.
    pub fn enabled(&self) -> bool {
        self.inner.dir.is_some() && self.limits().enabled
    }

    pub fn status(&self) -> api::OfflineCacheStatus {
        let limits = self.limits();
        let entries = self.read();
        api::OfflineCacheStatus {
            enabled: limits.enabled,
            max_bytes: limits.max_bytes,
            used_bytes: entries.values().map(|copy| copy.size).sum(),
            tracks: entries.len() as u32,
        }
    }

    pub fn contains(&self, item_id: &str) -> bool {
        self.read().contains_key(item_id)
    }

    /// The cached file of `item_id` if it is still there; one that is gone is forgotten.
    pub fn path(&self, item_id: &str) -> Option<PathBuf> {
        let path = self.read().get(item_id)?.path.clone();
        if path.is_file() {
            return Some(path);
        }
        if let Some(runtime) = self.inner.runtime.clone() {
            let cache = self.clone();
            let item_id = item_id.to_string();
            runtime.spawn(async move { cache.forget(&[item_id]).await });
        }
        None
    }

    /// A cached song is playing, which keeps it from eviction longest.
    pub fn played(&self, item_id: &str) {
        self.played_at(item_id, unix_now());
    }

    fn played_at(&self, item_id: &str, at: i64) {
        match self.write().get_mut(item_id) {
            Some(copy) => copy.last_played_at = at,
            None => return,
        }
        let (Some(db), Some(runtime)) = (self.inner.db.clone(), self.inner.runtime.clone()) else {
            return;
        };
        let item_id = item_id.to_string();
        runtime.spawn(async move {
            if let Err(error) = db.touch_offline_cache(&item_id, at).await {
                tracing::warn!(%error, "stamping a cached song as played failed");
            }
        });
    }

    fn begin_fill(&self, item_id: &str, foreground: bool) -> Option<Filling> {
        if !lock(&self.inner.filling).insert(item_id.to_string()) {
            return None;
        }
        if foreground {
            self.inner.foreground.send_modify(|streams| *streams += 1);
        }
        Some(Filling {
            cache: self.clone(),
            item_id: item_id.to_string(),
            foreground,
        })
    }

    /// A sink for the playing song's stream that keeps its whole file once it has arrived; `None` when there is nothing to keep it for.
    pub fn capture(&self, item_id: &str, source_id: &str) -> Option<CompleteSink> {
        if !self.enabled() || self.contains(item_id) {
            return None;
        }
        let runtime = self.inner.runtime.clone()?;
        let filling = self.begin_fill(item_id, true)?;
        let source_id = source_id.to_string();
        Some(Box::new(move |captured: Captured| {
            runtime.spawn(async move {
                let cache = filling.cache.clone();
                if let Err(error) = cache.store(&filling.item_id, &source_id, captured).await {
                    tracing::warn!(%error, "keeping a streamed song for offline listening failed");
                }
                drop(filling);
            });
        }))
    }

    /// Keep `captured` as `item_id`'s copy, then evict past the cap.
    pub async fn store(
        &self,
        item_id: &str,
        source_id: &str,
        captured: Captured,
    ) -> Result<(), ApiError> {
        self.store_at(item_id, source_id, captured, unix_now())
            .await
    }

    async fn store_at(
        &self,
        item_id: &str,
        source_id: &str,
        captured: Captured,
        at: i64,
    ) -> Result<(), ApiError> {
        let Some(dir) = self.inner.dir.clone() else {
            return Ok(());
        };
        if !self.limits().enabled || captured.bytes.is_empty() {
            return Ok(());
        }
        let content_type = captured
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let extension = crate::downloads::content_type_to_ext(&content_type).unwrap_or("bin");
        let path = dir.join(file_name(item_id, extension));
        let size = captured.bytes.len() as u64;
        let written = path.clone();
        tokio::task::spawn_blocking(move || publish(&written, &captured.bytes))
            .await
            .map_err(|error| ApiError::internal(format!("cache write task failed: {error}")))?
            .map_err(|error| ApiError::internal(format!("offline cache io: {error}")))?;
        let copy = CachedCopy {
            source: source_id.to_string(),
            path,
            size,
            content_type,
            cached_at: at,
            last_played_at: at,
        };
        let replaced = {
            let _writing = self.inner.writer.lock().await;
            if let Some(db) = &self.inner.db {
                db.put_offline_cache(&copy.row(item_id))
                    .await
                    .map_err(|error| {
                        ApiError::internal(format!("cache registration failed: {error}"))
                    })?;
            }
            self.write().insert(item_id.to_string(), copy.clone())
        };
        if let Some(old) = replaced.filter(|old| old.path != copy.path) {
            remove_file(&dir, &old.path);
        }
        tracing::info!(
            item = item_id,
            bytes = size,
            "kept a song for offline listening"
        );
        self.evict().await;
        self.inner.changed.notify_one();
        Ok(())
    }

    /// Delete the least recently played songs until what is left fits the cap.
    async fn evict(&self) {
        let max_bytes = self.limits().max_bytes;
        let victims: Vec<String> = {
            let entries = self.read();
            let mut used: u64 = entries.values().map(|copy| copy.size).sum();
            if used <= max_bytes {
                return;
            }
            let mut order: Vec<(&String, &CachedCopy)> = entries.iter().collect();
            order.sort_by(|(a_key, a), (b_key, b)| {
                (a.last_played_at, a.cached_at, a_key).cmp(&(b.last_played_at, b.cached_at, b_key))
            });
            let mut victims = Vec::new();
            for (key, copy) in order {
                if used <= max_bytes {
                    break;
                }
                used = used.saturating_sub(copy.size);
                victims.push(key.clone());
            }
            victims
        };
        tracing::info!(
            count = victims.len(),
            "evicted the least recently played cached songs past the cap"
        );
        self.forget(&victims).await;
    }

    /// Drop these songs from the index and delete their files.
    async fn forget(&self, item_ids: &[String]) {
        if item_ids.is_empty() {
            return;
        }
        let removed: Vec<CachedCopy> = {
            let _writing = self.inner.writer.lock().await;
            if let Some(db) = &self.inner.db
                && let Err(error) = db.remove_offline_cache(item_ids).await
            {
                tracing::warn!(%error, "forgetting cached songs failed");
                return;
            }
            let mut entries = self.write();
            item_ids
                .iter()
                .filter_map(|item_id| entries.remove(item_id))
                .collect()
        };
        if let Some(dir) = &self.inner.dir {
            for copy in &removed {
                remove_file(dir, &copy.path);
            }
        }
        self.inner.changed.notify_one();
    }

    /// Delete every cached song; downloads are another store and stay.
    pub async fn clear(&self) -> Result<(), ApiError> {
        let all: Vec<String> = self.read().keys().cloned().collect();
        self.forget(&all).await;
        if self.read().is_empty() {
            return Ok(());
        }
        Err(ApiError::internal("the offline cache could not be cleared"))
    }

    /// Every song kept from `source_id`, for a source that is being deleted.
    pub async fn forget_source(&self, source_id: &str) {
        let theirs: Vec<String> = self
            .read()
            .iter()
            .filter(|(_, copy)| copy.source == source_id)
            .map(|(key, _)| key.clone())
            .collect();
        self.forget(&theirs).await;
    }

    /// Hand `item_id`'s copy over, as a download of it does: the index forgets it and the file is the caller's to move.
    pub async fn take(&self, item_id: &str) -> Option<CachedCopy> {
        let copy = self.read().get(item_id).cloned()?;
        if !copy.path.is_file() {
            self.forget(&[item_id.to_string()]).await;
            return None;
        }
        {
            let _writing = self.inner.writer.lock().await;
            if let Some(db) = &self.inner.db
                && let Err(error) = db.remove_offline_cache(&[item_id.to_string()]).await
            {
                tracing::warn!(%error, "handing a cached song over failed");
                return None;
            }
            self.write().remove(item_id);
        }
        self.inner.changed.notify_one();
        Some(copy)
    }

    /// Fetch the next song into the cache once the playing one's stream is in; a newer request replaces one still waiting.
    pub fn prefetch(&self, request: PrefetchRequest) {
        if !self.enabled() || self.contains(&request.item_id) {
            return;
        }
        let Some(runtime) = self.inner.runtime.clone() else {
            return;
        };
        let mut slot = lock(&self.inner.prefetch);
        if let Some((item_id, task)) = slot.as_ref() {
            if *item_id == request.item_id && !task.is_finished() {
                return;
            }
            task.abort();
        }
        let Some(filling) = self.begin_fill(&request.item_id, false) else {
            return;
        };
        let cache = self.clone();
        let item_id = request.item_id.clone();
        let task = runtime.spawn(async move {
            let mut streams = cache.inner.foreground.subscribe();
            if streams.wait_for(|streams| *streams == 0).await.is_err() {
                return;
            }
            tracing::debug!(item = %request.item_id, "prefetching the next song");
            let fetched =
                tokio::time::timeout(PREFETCH_TIMEOUT, cache.inner.fetcher.fetch(&request)).await;
            match fetched {
                Ok(Ok(captured)) => {
                    if let Err(error) = cache
                        .store(&request.item_id, &request.source_id, captured)
                        .await
                    {
                        tracing::warn!(%error, "keeping a prefetched song failed");
                    }
                }
                Ok(Err(error)) => tracing::debug!(%error, "prefetching the next song failed"),
                Err(_) => tracing::debug!("prefetching the next song timed out"),
            }
            drop(filling);
        });
        *slot = Some((item_id, task));
    }

    /// The song a prefetch was last asked for, whether or not it has landed.
    pub fn prefetching(&self) -> Option<String> {
        lock(&self.inner.prefetch)
            .as_ref()
            .map(|(item_id, _)| item_id.clone())
    }
}

/// Write through a staging file and rename, so a crash cannot leave a truncated copy that reads back as whole.
fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let staging = path.with_extension(format!("part-{}", uuid::Uuid::new_v4()));
    if let Err(error) = std::fs::write(&staging, bytes) {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }
    std::fs::rename(&staging, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staging);
    })
}

/// Delete a file only when it sits in the cache's own directory.
fn remove_file(dir: &Path, path: &Path) {
    let inside = path.parent().is_some_and(|parent| {
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
        canonical(parent) == canonical(dir)
    });
    if !inside {
        tracing::warn!(path = %path.display(), "not deleting a cached file outside the cache");
        return;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "deleting a cached file failed")
        }
    }
}

/// Files the index does not name are left from a crash or a cleared index, and take room nothing counts.
fn sweep_strays(dir: &Path, entries: &HashMap<String, CachedCopy>) {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    let kept: HashSet<&std::ffi::OsStr> = entries
        .values()
        .filter_map(|copy| copy.path.file_name())
        .collect();
    for entry in listing.flatten() {
        let path = entry.path();
        if path.is_file() && !kept.contains(entry.file_name().as_os_str()) {
            remove_file(dir, &path);
        }
    }
}

/// The real fetch: the same stream a play would open, downloaded whole.
struct StreamFetch;

#[async_trait::async_trait]
impl Fetch for StreamFetch {
    async fn fetch(&self, request: &PrefetchRequest) -> Result<Captured, String> {
        let source = request
            .source
            .clone()
            .ok_or_else(|| "no active source to fetch from".to_string())?;
        match source.download_track(&request.item_id, None).await {
            Ok(bytes) => {
                return Ok(Captured {
                    bytes,
                    content_type: Some("audio/mp4".into()),
                });
            }
            Err(server::source::SourceError::Unsupported(_)) => {}
            Err(error) => return Err(error.to_string()),
        }
        let info = source
            .resolve_stream(&request.item_id, request.quality)
            .await
            .map_err(|error| error.to_string())?;
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || whole_stream(info, runtime))
            .await
            .map_err(|error| error.to_string())?
    }
}

/// Download a resolved stream whole, by range where the server allows it and progressively where it does not.
fn whole_stream(
    info: server::source::StreamInfo,
    runtime: tokio::runtime::Handle,
) -> Result<Captured, String> {
    let url = match ResolvedStreamRef::parse(&info.url) {
        ResolvedStreamRef::SoundCloudHls(url) => {
            let bytes = server::stream::hls_source::assemble(url, info.user_agent.as_deref())
                .map_err(|error| error.to_string())?;
            return Ok(Captured {
                bytes,
                content_type: Some("audio/mp4".into()),
            });
        }
        ResolvedStreamRef::Direct(url) => url.to_string(),
        ResolvedStreamRef::Pending(_) | ResolvedStreamRef::AppleMusicFmp4(_) => {
            return Err("this stream cannot be fetched whole".into());
        }
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let sink: CompleteSink = Box::new(move |captured| {
        let _ = tx.send(captured);
    });
    let ranged = match info.format {
        Some((_, false)) => None,
        _ => server::stream::range_source::RangeStreamSource::new(
            url.clone(),
            info.user_agent.clone(),
        )
        .and_then(|range| {
            if info.format.is_some() {
                range.check_tail()?;
            }
            Ok(range)
        })
        .ok(),
    };
    let wait = |rx: std::sync::mpsc::Receiver<Captured>| {
        rx.recv_timeout(PREFETCH_TIMEOUT)
            .map_err(|_| "the stream stopped before it was whole".to_string())
    };
    match ranged {
        Some(mut range) => {
            range.capture(sink);
            let captured = wait(rx);
            drop(range);
            captured
        }
        None => {
            let stream = server::stream::stream_buffer::StreamBuffer::capturing(
                url,
                server::stream::stream_buffer::stream_client(info.user_agent),
                runtime,
                None,
                Some(sink),
            );
            let captured = wait(rx);
            drop(stream);
            captured
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;

    fn settings(enabled: bool, max_bytes: u64) -> config::Settings {
        let mut settings = config::Settings::default();
        settings.library.offline_cache_enabled = enabled;
        settings.library.offline_cache_max_bytes = max_bytes;
        settings
    }

    fn song(bytes: usize) -> Captured {
        Captured {
            bytes: vec![7; bytes],
            content_type: Some("audio/webm".into()),
        }
    }

    struct Fixture {
        db: db::Db,
        dir: tempfile::TempDir,
        cache: OfflineCache,
    }

    impl Fixture {
        fn cache_dir(&self) -> PathBuf {
            self.dir.path().join("offline-cache")
        }

        fn files(&self) -> usize {
            std::fs::read_dir(self.cache_dir())
                .map(|listing| listing.count())
                .unwrap_or(0)
        }

        async fn keys(&self) -> Vec<String> {
            let mut keys: Vec<String> = self
                .db
                .offline_cache()
                .await
                .expect("rows")
                .into_iter()
                .map(|row| row.track_key)
                .collect();
            keys.sort();
            keys
        }
    }

    async fn fixture(settings: config::Settings, fetcher: Arc<dyn Fetch>) -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = db::init(&dir.path().join("cache.db")).await.expect("db");
        let cache =
            OfflineCache::load_with(&db, dir.path().join("offline-cache"), &settings, fetcher)
                .await
                .expect("cache");
        Fixture { db, dir, cache }
    }

    struct NoFetch;

    #[async_trait::async_trait]
    impl Fetch for NoFetch {
        async fn fetch(&self, _: &PrefetchRequest) -> Result<Captured, String> {
            Err("offline".into())
        }
    }

    #[tokio::test]
    async fn past_the_cap_the_least_recently_played_song_goes_first() {
        let fixture = fixture(settings(true, 3 * MB), Arc::new(NoFetch)).await;
        let cache = &fixture.cache;
        for (at, key) in [(10, "a"), (20, "b"), (30, "c")] {
            cache
                .store_at(key, "yt", song(MB as usize), at)
                .await
                .expect("kept");
        }
        cache.played_at("a", 40);

        cache
            .store_at("d", "yt", song(MB as usize), 50)
            .await
            .expect("kept");

        assert_eq!(
            fixture.keys().await,
            ["a", "c", "d"],
            "b was played longest ago"
        );
        assert_eq!(fixture.files(), 3);
        let status = cache.status();
        assert_eq!((status.used_bytes, status.tracks), (3 * MB, 3));
        assert!(status.enabled);
    }

    #[tokio::test]
    async fn downloads_are_neither_counted_nor_evicted() {
        let fixture = fixture(settings(true, 2 * MB), Arc::new(NoFetch)).await;
        let download = fixture.dir.path().join("downloaded.webm");
        std::fs::write(&download, vec![1; 5 * MB as usize]).expect("a download");
        fixture
            .db
            .set_offline_track("kept", Some(&download.to_string_lossy()))
            .await
            .expect("registered");

        for (at, key) in [(10, "a"), (20, "b"), (30, "c")] {
            fixture
                .cache
                .store_at(key, "yt", song(MB as usize), at)
                .await
                .expect("kept");
        }

        assert_eq!(fixture.cache.status().used_bytes, 2 * MB);
        assert_eq!(fixture.keys().await, ["b", "c"]);
        assert!(download.is_file());
        assert!(
            fixture
                .db
                .offline_tracks()
                .await
                .expect("rows")
                .contains_key("kept")
        );
    }

    #[tokio::test]
    async fn turned_off_it_adds_nothing_and_keeps_what_it_holds() {
        let fixture = fixture(settings(true, 10 * MB), Arc::new(NoFetch)).await;
        fixture
            .cache
            .store("a", "yt", song(1000))
            .await
            .expect("kept");

        fixture.cache.set_limits(&settings(false, 10 * MB));
        fixture
            .cache
            .store("b", "yt", song(1000))
            .await
            .expect("nothing to do");

        assert!(fixture.cache.capture("c", "yt").is_none());
        assert_eq!(fixture.keys().await, ["a"]);
        assert!(fixture.cache.path("a").is_some(), "still plays");
        assert!(!fixture.cache.status().enabled);
    }

    #[tokio::test]
    async fn clearing_deletes_every_song_and_leaves_downloads() {
        let fixture = fixture(settings(true, 10 * MB), Arc::new(NoFetch)).await;
        for key in ["a", "b"] {
            fixture
                .cache
                .store(key, "yt", song(1000))
                .await
                .expect("kept");
        }
        let download = fixture.dir.path().join("downloaded.webm");
        std::fs::write(&download, b"audio").expect("a download");

        fixture.cache.clear().await.expect("cleared");

        assert!(fixture.keys().await.is_empty());
        assert_eq!(fixture.files(), 0);
        assert_eq!(fixture.cache.status().tracks, 0);
        assert!(download.is_file());
    }

    #[tokio::test]
    async fn a_song_handed_to_the_downloads_leaves_the_index_with_its_file() {
        let fixture = fixture(settings(true, 10 * MB), Arc::new(NoFetch)).await;
        fixture
            .cache
            .store("a", "yt", song(1000))
            .await
            .expect("kept");

        let copy = fixture.cache.take("a").await.expect("cached");

        assert!(copy.path.is_file(), "the file is the caller's now");
        assert!(!fixture.cache.contains("a"));
        assert!(fixture.keys().await.is_empty());
        assert!(fixture.cache.take("a").await.is_none());
    }

    #[tokio::test]
    async fn a_stream_cut_short_keeps_nothing_and_frees_the_song() {
        let fixture = fixture(settings(true, 10 * MB), Arc::new(NoFetch)).await;

        let sink = fixture.cache.capture("a", "yt").expect("a sink");
        assert!(
            fixture.cache.capture("a", "yt").is_none(),
            "one fill at a time"
        );
        drop(sink);

        assert!(fixture.keys().await.is_empty());
        let again = fixture.cache.capture("a", "yt").expect("free again");
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let changed = fixture.cache.changed();
        tokio::spawn(async move {
            changed.notified().await;
            let _ = done_tx.send(());
        });
        again(song(1000));
        done_rx.await.expect("stored");
        assert_eq!(fixture.keys().await, ["a"]);
        let cached = fixture.cache.path("a").expect("a file");
        assert_eq!(std::fs::read(cached).expect("read"), vec![7; 1000]);
    }

    #[tokio::test]
    async fn the_index_drops_missing_files_and_the_directory_drops_strays_at_load() {
        let fixture = fixture(settings(true, 10 * MB), Arc::new(NoFetch)).await;
        fixture
            .cache
            .store("a", "yt", song(1000))
            .await
            .expect("kept");
        fixture
            .cache
            .store("b", "yt", song(1000))
            .await
            .expect("kept");
        let gone = fixture.cache.path("b").expect("b");
        std::fs::remove_file(gone).expect("delete");
        let stray = fixture.cache_dir().join("left-over.part-1");
        std::fs::write(&stray, b"half").expect("stray");

        let reloaded =
            OfflineCache::load(&fixture.db, fixture.cache_dir(), &settings(true, 10 * MB))
                .await
                .expect("reload");

        assert!(reloaded.contains("a"));
        assert!(!reloaded.contains("b"));
        assert_eq!(fixture.keys().await, ["a"]);
        assert!(!stray.exists());
    }

    struct Recorded(Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl Fetch for Recorded {
        async fn fetch(&self, request: &PrefetchRequest) -> Result<Captured, String> {
            lock(&self.0).push(request.item_id.clone());
            Ok(song(500))
        }
    }

    #[tokio::test]
    async fn a_prefetch_waits_for_the_playing_stream_then_keeps_the_song() {
        let fetcher = Arc::new(Recorded(Mutex::default()));
        let fixture = fixture(settings(true, 10 * MB), fetcher.clone()).await;
        let playing = fixture.cache.capture("now", "yt").expect("a sink");
        let changed = fixture.cache.changed();

        fixture.cache.prefetch(PrefetchRequest {
            item_id: "next".into(),
            source_id: "yt".into(),
            source: None,
            quality: config::StreamQuality::High,
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(lock(&fetcher.0).is_empty(), "the playing song comes first");

        playing(song(800));
        tokio::time::timeout(Duration::from_secs(10), async {
            while !(fixture.cache.contains("now") && fixture.cache.contains("next")) {
                changed.notified().await;
            }
        })
        .await
        .expect("both kept");
        assert_eq!(*lock(&fetcher.0), ["next"]);
        assert_eq!(fixture.cache.prefetching().as_deref(), Some("next"));
    }
}
