use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{ArtworkRef, ArtworkRequest, ArtworkTarget};
use gpui_kit::{App, Asset, Image, ImageCacheError, ImageFormat, ImageSource, Window};
use link::Link;

/// One cover at one size; `version` changes exactly when the picture would, so a hit caches forever.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtworkKey {
    pub target: ArtworkTarget,
    pub version: u64,
    pub hq: bool,
}

impl ArtworkKey {
    pub fn new(artwork: &ArtworkRef, hq: bool) -> Self {
        Self {
            target: artwork.target.clone(),
            version: artwork.version,
            hq,
        }
    }
}

/// Fetches cover bytes through the daemon; GPUI's asset cache keys, dedupes and decodes them.
pub enum ArtworkBytes {}

impl Asset for ArtworkBytes {
    type Source = ArtworkKey;
    type Output = Result<Arc<Image>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let hq = source.hq;
        let call = Link::global(cx).call(move |api| async move {
            api.artwork(ArtworkRequest {
                target: source.target,
                hq: source.hq,
            })
            .await
        });
        async move {
            let span = tracing::debug_span!("artwork.load", hq);
            let _entered = span.enter();
            match call.await {
                Ok(data) => {
                    // Counted here: this is the one place a cover enters the process.
                    let (width, height) = probe_size(&data.bytes).unwrap_or((0, 0));
                    profile::artwork::loaded(data.bytes.len(), width, height);
                    Ok(Arc::new(Image::from_bytes(
                        image_format(&data.content_type),
                        data.bytes,
                    )))
                }
                Err(error) => Err(ImageCacheError::Asset(error.to_string().into())),
            }
        }
    }
}

/// A cover's encoded thumbnail: the copy a view already holds if there is one, else a fetch of it.
pub(crate) fn thumbnail_bytes(
    key: &ArtworkKey,
    cx: &mut App,
) -> impl Future<Output = Option<Vec<u8>>> + Send + 'static + use<> {
    let resident = cx
        .fetch_asset::<ArtworkBytes>(key)
        .and_then(Result::ok)
        .map(|image| image.bytes.clone());
    let call = resident.is_none().then(|| {
        let target = key.target.clone();
        Link::global(cx)
            .call(move |api| async move { api.artwork(ArtworkRequest { target, hq: false }).await })
    });
    async move {
        match resident {
            Some(bytes) => Some(bytes),
            None => call?.await.ok().map(|data| data.bytes),
        }
    }
}

/// The decoded dimensions, read from the header rather than by decoding the whole picture.
fn probe_size(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// An `img()` source for a row's artwork; `hq` asks for the full-size picture rather than a thumbnail.
pub fn artwork_source(artwork: &ArtworkRef, hq: bool) -> ImageSource {
    let key = ArtworkKey::new(artwork, hq);
    ImageSource::Custom(Arc::new(move |window: &mut Window, cx: &mut App| {
        let result = match window.use_asset::<ArtworkBytes>(&key, cx) {
            Some(Ok(image)) => {
                cx.default_global::<Residents>().failed.remove(&key);
                if let Some(rendered) = image.clone().use_render_image(window, cx) {
                    if Residents::touch(&key, &image, cx) {
                        // The cover fading this in learns it was drawn on the next frame, so ask for one.
                        window.request_animation_frame();
                    }
                    return Some(Ok(rendered));
                }
                None
            }
            Some(Err(error)) => {
                if cx.default_global::<Residents>().failed.insert(key.clone()) {
                    // The cover's fallback shows once its frame learns the picture failed.
                    window.request_animation_frame();
                }
                Some(Err(error))
            }
            None => None,
        };
        // Keep the last decoded cover until its replacement has both fetched and decoded.
        if let Some((previous, image)) = cx.default_global::<Residents>().previous(&key) {
            Residents::touch(&previous, &image, cx);
            if let Some(rendered) = image.use_render_image(window, cx) {
                return Some(Ok(rendered));
            }
        }
        result
    }))
}

/// Long past any cover's fade-in, after which its arrival is forgotten and it counts as settled.
const ARRIVAL_KEPT: Duration = Duration::from_secs(2);

/// Where a cover stands in its first appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// Nothing drawable yet.
    Waiting,
    /// First drawn at this instant.
    Since(Instant),
    /// Drawn long enough ago, failed, or standing in for an older version: nothing to fade.
    Settled,
}

/// How many decoded covers to keep. A screenful is a few dozen; this is several screens of scrollback.
const BUDGET: usize = 256;

/// `KOPUZ_ARTWORK_BUDGET` overrides it, which is how the sweep gets tested without scrolling a library.
fn budget() -> usize {
    std::env::var("KOPUZ_ARTWORK_BUDGET")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(BUDGET)
}

/// The covers currently decoded, so the oldest can be released once the budget is passed.
///
/// GPUI caches an image in three places at once and frees none of them on its own: the fetched
/// bytes in the asset cache, the decoded RGBA in the image cache, and the texture in the window's
/// sprite atlas. Dropping the element does not drop any of them.
#[derive(Default)]
pub struct Residents {
    live: HashMap<ArtworkKey, Resident>,
    // Per target and size, the version drawn last: what stands in while a newer one loads.
    latest: HashMap<(ArtworkTarget, bool), ArtworkKey>,
    failed: HashSet<ArtworkKey>,
    // When each cover was first drawn, while that is recent enough to still be fading in.
    arrived: HashMap<ArtworkKey, Instant>,
    clock: u64,
}

struct Resident {
    image: Arc<Image>,
    used: u64,
}

impl gpui_kit::Global for Residents {}

impl Residents {
    fn previous(&self, key: &ArtworkKey) -> Option<(ArtworkKey, Arc<Image>)> {
        let old = self
            .latest
            .get(&(key.target.clone(), key.hq))
            .filter(|old| *old != key)?;
        let resident = self.live.get(old)?;
        Some((old.clone(), resident.image.clone()))
    }

    /// Whether `key` is drawn for the first time, which is when it starts fading in.
    fn record(&mut self, key: &ArtworkKey, image: &Arc<Image>, now: Instant) -> bool {
        self.clock += 1;
        let clock = self.clock;
        let first = !self.live.contains_key(key);
        // A new version takes over from the old one on screen, so it has nothing to fade in from.
        let replaces = self.previous(key).is_some();
        if first && !replaces {
            self.arrived
                .retain(|_, at| now.saturating_duration_since(*at) < ARRIVAL_KEPT);
            self.arrived.insert(key.clone(), now);
        }
        self.live
            .entry(key.clone())
            .and_modify(|resident| resident.used = clock)
            .or_insert_with(|| Resident {
                image: image.clone(),
                used: clock,
            });
        self.latest
            .insert((key.target.clone(), key.hq), key.clone());
        first
    }

    fn forget(&mut self, key: &ArtworkKey) -> Option<Resident> {
        let slot = (key.target.clone(), key.hq);
        if self.latest.get(&slot) == Some(key) {
            self.latest.remove(&slot);
        }
        self.arrived.remove(key);
        self.live.remove(key)
    }

    fn arrival_of(&self, key: &ArtworkKey) -> Arrival {
        if let Some(at) = self.arrived.get(key) {
            return Arrival::Since(*at);
        }
        if self.live.contains_key(key) || self.failed.contains(key) || self.previous(key).is_some()
        {
            return Arrival::Settled;
        }
        Arrival::Waiting
    }

    /// How far into its first appearance `artwork` is, for a cover that fades in rather than pops.
    pub fn arrival(artwork: &ArtworkRef, hq: bool, cx: &App) -> Arrival {
        cx.try_global::<Self>()
            .map_or(Arrival::Waiting, |residents| {
                residents.arrival_of(&ArtworkKey::new(artwork, hq))
            })
    }

    pub(crate) fn retry_failed(cx: &mut App) {
        let failed = std::mem::take(&mut cx.default_global::<Self>().failed);
        for key in &failed {
            cx.remove_asset::<ArtworkBytes>(key);
        }
        if !failed.is_empty() {
            cx.refresh_windows();
        }
    }

    /// Records that `key` was drawn this frame, and says whether it was the first.
    fn touch(key: &ArtworkKey, image: &Arc<Image>, cx: &mut App) -> bool {
        cx.default_global::<Self>()
            .record(key, image, Instant::now())
    }

    /// Releases the least recently drawn covers until the budget is met.
    pub fn sweep(window: &mut Window, cx: &mut App) {
        let held = cx.default_global::<Self>().live.len();
        profile::artwork::tracked(held);
        let over = held.saturating_sub(budget());
        if over == 0 {
            return;
        }
        // Oldest drawn first, which leaves the screenful at the young end untouched so long as the
        // budget stays well above it. Do not filter by "drawn recently": under a continuous scroll
        // every cover was drawn recently, and the sweep stops exactly when it is needed most.
        let residents = cx.default_global::<Self>();
        let mut ages: Vec<(u64, ArtworkKey)> = residents
            .live
            .iter()
            .map(|(key, resident)| (resident.used, key.clone()))
            .collect();
        ages.sort_unstable_by_key(|(used, _)| *used);
        let doomed: Vec<ArtworkKey> = ages.into_iter().take(over).map(|(_, key)| key).collect();
        for key in doomed {
            let Some(resident) = cx.default_global::<Self>().forget(&key) else {
                continue;
            };
            // The atlas texture goes first; it is the only one that needs the window.
            if let Some(rendered) = resident.image.clone().get_render_image(window, cx) {
                cx.drop_image(rendered, Some(window));
            }
            resident.image.remove_asset(cx);
            cx.remove_asset::<ArtworkBytes>(&key);
            profile::artwork::released();
        }
    }
}

fn image_format(content_type: &str) -> ImageFormat {
    match content_type {
        "image/png" => ImageFormat::Png,
        "image/webp" => ImageFormat::Webp,
        "image/gif" => ImageFormat::Gif,
        "image/bmp" => ImageFormat::Bmp,
        "image/svg+xml" => ImageFormat::Svg,
        _ => ImageFormat::Jpeg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str, version: u64, hq: bool) -> ArtworkKey {
        ArtworkKey::new(
            &ArtworkRef::new(ArtworkTarget::Album(id.into()), version),
            hq,
        )
    }

    fn image(byte: u8) -> Arc<Image> {
        Arc::new(Image::from_bytes(ImageFormat::Png, vec![byte]))
    }

    #[test]
    fn a_new_version_can_keep_the_previous_cover_while_loading() {
        let old = key("album", 1, false);
        let image = image(1);
        let mut residents = Residents::default();
        residents.record(&old, &image, Instant::now());
        let (previous, fallback) = residents.previous(&key("album", 2, false)).unwrap();
        assert_eq!(previous, old);
        assert!(Arc::ptr_eq(&fallback, &image));
        assert!(residents.previous(&old).is_none());
        assert!(
            residents
                .previous(&key("another-album", 2, false))
                .is_none()
        );
        assert!(residents.previous(&key("album", 2, true)).is_none());
    }

    #[test]
    fn replacement_uses_the_most_recently_displayed_cover() {
        let mut residents = Residents::default();
        for version in [1, 3, 2] {
            residents.record(
                &key("album", version, false),
                &image(version as u8),
                Instant::now(),
            );
        }
        assert_eq!(
            residents.previous(&key("album", 4, false)).unwrap().0,
            key("album", 2, false),
            "the last one drawn, not the highest version"
        );
    }

    #[test]
    fn a_released_cover_is_no_stand_in() {
        let mut residents = Residents::default();
        let old = key("album", 1, false);
        residents.record(&old, &image(1), Instant::now());
        assert!(residents.forget(&old).is_some());
        assert!(residents.previous(&key("album", 2, false)).is_none());
    }

    #[test]
    fn a_cover_drawn_for_the_first_time_arrives_then() {
        let mut residents = Residents::default();
        let cover = key("album", 1, false);
        assert_eq!(residents.arrival_of(&cover), Arrival::Waiting);
        let now = Instant::now();
        assert!(residents.record(&cover, &image(1), now));
        assert_eq!(residents.arrival_of(&cover), Arrival::Since(now));
        assert!(
            !residents.record(&cover, &image(1), now),
            "drawn again is no arrival"
        );
        assert_eq!(residents.arrival_of(&cover), Arrival::Since(now));
    }

    #[test]
    fn a_new_version_takes_over_without_fading() {
        let mut residents = Residents::default();
        residents.record(&key("album", 1, false), &image(1), Instant::now());
        let newer = key("album", 2, false);
        assert_eq!(
            residents.arrival_of(&newer),
            Arrival::Settled,
            "the old one stands in"
        );
        residents.record(&newer, &image(2), Instant::now());
        assert_eq!(residents.arrival_of(&newer), Arrival::Settled);
    }

    #[test]
    fn old_arrivals_are_forgotten_and_settle() {
        let mut residents = Residents::default();
        let start = Instant::now();
        let first = key("first", 1, false);
        residents.record(&first, &image(1), start);
        residents.record(&key("second", 1, false), &image(2), start + ARRIVAL_KEPT);
        assert_eq!(residents.arrival_of(&first), Arrival::Settled);
    }

    #[test]
    fn a_failed_or_released_cover_has_nothing_to_fade() {
        let mut residents = Residents::default();
        let broken = key("broken", 1, false);
        residents.failed.insert(broken.clone());
        assert_eq!(residents.arrival_of(&broken), Arrival::Settled);
        let gone = key("gone", 1, false);
        residents.record(&gone, &image(1), Instant::now());
        residents.forget(&gone);
        assert_eq!(residents.arrival_of(&gone), Arrival::Waiting);
    }
}
