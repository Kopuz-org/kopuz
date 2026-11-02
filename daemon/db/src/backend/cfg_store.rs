//! App state persistence: the state row and its tables; settings live in the settings file (`config::store`), and credentials and library data have tables of their own.

use std::collections::HashMap;
use std::path::Path;

use config::{Browser, MusicServer, MusicService, SavedServer, Settings, Source};
use sqlx::SqlitePool;

use crate::DbError;

/// How many recent entries to keep per source.
const RECENT_LIMIT: i64 = 50;

pub async fn has_app_state(pool: &SqlitePool) -> Result<bool, DbError> {
    let state: Option<i64> = sqlx::query_scalar("SELECT id FROM app_state WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    Ok(state.is_some())
}

pub async fn load_sources(pool: &SqlitePool) -> Result<crate::StoredSources, DbError> {
    let state = sqlx::query!("SELECT device_id, active_source FROM app_state WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    let active = state
        .as_ref()
        .map(|state| Source::from_column(&state.active_source))
        .unwrap_or_default();
    let mut folders: HashMap<String, Vec<String>> = HashMap::new();
    let rows =
        sqlx::query!("SELECT server_id, path FROM server_folders ORDER BY server_id, position")
            .fetch_all(pool)
            .await?;
    for row in rows {
        folders.entry(row.server_id).or_default().push(row.path);
    }
    let servers = stored_servers(pool, None).await?;
    let server = active.server_id().and_then(|active| {
        servers
            .iter()
            .find(|server| server.id == *active)
            .map(StoredServer::music_server)
    });
    Ok(crate::StoredSources {
        active,
        server,
        servers: servers.iter().map(StoredServer::saved).collect(),
        folders,
        device_id: state.map(|state| state.device_id),
    })
}

pub async fn play_history(
    pool: &SqlitePool,
    source: &Source,
) -> Result<Vec<crate::PlayRecord>, DbError> {
    let rows: Vec<(String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT track_key, count, last_played_at FROM listen_counts \
          WHERE source = ?1 AND count > 0 ORDER BY count DESC, track_key",
    )
    .bind(source.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(track_key, count, last_played_at)| crate::PlayRecord {
            track_key,
            count: count.max(0) as u64,
            last_played_at,
        })
        .collect())
}

/// Every play count, under the uid keys every reader of the map looks them up by.
pub async fn listen_counts(pool: &SqlitePool) -> Result<HashMap<String, u64>, DbError> {
    let counts = sqlx::query!(
        "SELECT lc.source, lc.track_key, lc.count, s.service \
           FROM listen_counts lc LEFT JOIN servers s ON s.id = lc.source"
    )
    .fetch_all(pool)
    .await?;
    Ok(counts
        .into_iter()
        .map(|r| {
            let uid = match r.service {
                Some(service) => format!("{}:{}", service.to_lowercase(), r.track_key),
                None => r.track_key,
            };
            let key = Source::from_column(&r.source).listen_count_key(&uid);
            (key, r.count.max(0) as u64)
        })
        .collect())
}

pub async fn offline_tracks(pool: &SqlitePool) -> Result<HashMap<String, String>, DbError> {
    let offline = sqlx::query!("SELECT item_id, path FROM offline_tracks")
        .fetch_all(pool)
        .await?;
    Ok(offline.into_iter().map(|r| (r.item_id, r.path)).collect())
}

pub async fn pinned_stations(pool: &SqlitePool) -> Result<Vec<String>, DbError> {
    sqlx::query_scalar!("SELECT manifest FROM pinned_stations ORDER BY position")
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn history_rows(
    conn: &mut sqlx::SqliteConnection,
) -> Result<Vec<config::DownloaderHistoryEntry>, DbError> {
    Ok(sqlx::query!(
        "SELECT url, title, format, status, error FROM ytdlp_history ORDER BY position"
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| config::DownloaderHistoryEntry {
        url: row.url,
        title: row.title,
        format: row.format,
        status: row.status,
        error: row.error,
    })
    .collect())
}

pub async fn downloader_history(
    pool: &SqlitePool,
) -> Result<Vec<config::DownloaderHistoryEntry>, DbError> {
    let mut conn = pool.acquire().await?;
    history_rows(&mut conn).await
}

/// Put `entry` at the head of the downloader history, keeping at most `limit` rows.
pub async fn push_downloader_history(
    pool: &SqlitePool,
    entry: &config::DownloaderHistoryEntry,
    limit: usize,
) -> Result<(), DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    let mut entries = history_rows(&mut tx).await?;
    entries.insert(0, entry.clone());
    entries.truncate(limit);
    write_history(&mut tx, &entries).await?;
    tx.commit().await
}

pub async fn clear_downloader_history(pool: &SqlitePool) -> Result<(), DbError> {
    let mut conn = pool.acquire().await?;
    write_history(&mut conn, &[]).await
}

pub(crate) async fn write_history(
    conn: &mut sqlx::SqliteConnection,
    entries: &[config::DownloaderHistoryEntry],
) -> Result<(), DbError> {
    sqlx::query!("DELETE FROM ytdlp_history")
        .execute(&mut *conn)
        .await?;
    for (position, entry) in entries.iter().enumerate() {
        let position = position as i64;
        sqlx::query!(
            "INSERT INTO ytdlp_history (position, url, title, format, status, error) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            position,
            entry.url,
            entry.title,
            entry.format,
            entry.status,
            entry.error
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub async fn integration_credentials(
    pool: &SqlitePool,
) -> Result<HashMap<String, String>, DbError> {
    let rows = sqlx::query!("SELECT key, value FROM integration_credentials")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|row| (row.key, row.value)).collect())
}

/// Store one integration credential, or drop it when `value` is empty.
pub(crate) async fn write_integration_credential(
    conn: &mut sqlx::SqliteConnection,
    key: &str,
    value: &str,
) -> Result<(), DbError> {
    if value.is_empty() {
        sqlx::query("DELETE FROM integration_credentials WHERE key = ?1")
            .bind(key)
            .execute(&mut *conn)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO integration_credentials (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub async fn ensure_app_state(pool: &SqlitePool) -> Result<(), DbError> {
    // It can read before it writes, and a deferred BEGIN then fails at once against a write that landed meanwhile.
    let mut tx = super::begin_immediate(pool).await?;
    ensure_state_row(&mut tx).await?;
    tx.commit().await
}

/// The volume a fresh state row starts at, before the engine saved one.
const DEFAULT_VOLUME: f32 = 1.0;

/// The volume and mute stored for the engine.
pub async fn playback_state(pool: &SqlitePool) -> Result<(f32, bool), DbError> {
    let row: Option<(f64, bool)> =
        sqlx::query_as("SELECT volume, muted FROM app_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    Ok(row.map_or((DEFAULT_VOLUME, false), |(volume, muted)| {
        (volume as f32, muted)
    }))
}

pub async fn set_playback_state(
    pool: &SqlitePool,
    volume: f32,
    muted: bool,
) -> Result<(), DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    write_playback_state(&mut tx, volume, muted).await?;
    tx.commit().await
}

/// Set the stored volume and mute, creating the state row if it is missing.
pub(crate) async fn write_playback_state(
    conn: &mut sqlx::SqliteConnection,
    volume: f32,
    muted: bool,
) -> Result<(), DbError> {
    ensure_state_row(conn).await?;
    sqlx::query("UPDATE app_state SET volume = ?1, muted = ?2 WHERE id = 1")
        .bind(f64::from(volume.clamp(0.0, 1.0)))
        .bind(muted)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn discord_presence_paused(pool: &SqlitePool) -> Result<Option<bool>, DbError> {
    let paused: Option<Option<bool>> =
        sqlx::query_scalar("SELECT discord_presence_paused FROM app_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    Ok(paused.flatten())
}

pub async fn set_discord_presence_paused(pool: &SqlitePool, paused: bool) -> Result<(), DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    write_discord_presence_paused(&mut tx, Some(paused)).await?;
    tx.commit().await
}

pub(crate) async fn write_discord_presence_paused(
    conn: &mut sqlx::SqliteConnection,
    paused: Option<bool>,
) -> Result<(), DbError> {
    ensure_state_row(conn).await?;
    sqlx::query("UPDATE app_state SET discord_presence_paused = ?1 WHERE id = 1")
        .bind(paused)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Create the app_state row if nothing has written it yet, with a fresh device id, the default source and the default playback state, which have writers of their own.
pub(crate) async fn ensure_state_row(conn: &mut sqlx::SqliteConnection) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO app_state (id, device_id, active_source, volume) VALUES (1, ?1, ?2, ?3) \
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(Source::default().as_str())
    .bind(f64::from(DEFAULT_VOLUME))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Replace one server's library roots.
pub(crate) async fn write_server_folders(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    folders: &[String],
) -> Result<(), DbError> {
    sqlx::query("DELETE FROM server_folders WHERE server_id = ?1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    for (position, path) in folders.iter().enumerate() {
        // A folder list for a server the app no longer has would name nothing.
        sqlx::query(
            "INSERT INTO server_folders (server_id, position, path) \
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM servers WHERE id = ?1)",
        )
        .bind(id)
        .bind(position as i64)
        .bind(path)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub async fn set_server_folders(
    pool: &SqlitePool,
    id: &str,
    folders: &[String],
) -> Result<(), DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    write_server_folders(&mut tx, id, folders).await?;
    tx.commit().await
}

/// Set the stored source and device id, creating the state row if it is missing.
pub(crate) async fn write_source_state(
    conn: &mut sqlx::SqliteConnection,
    active: Option<&Source>,
    device_id: Option<&str>,
) -> Result<(), DbError> {
    ensure_state_row(conn).await?;
    if let Some(active) = active {
        sqlx::query("UPDATE app_state SET active_source = ?1 WHERE id = 1")
            .bind(active.as_str())
            .execute(&mut *conn)
            .await?;
    }
    if let Some(device_id) = device_id {
        sqlx::query("UPDATE app_state SET device_id = ?1 WHERE id = 1")
            .bind(device_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

pub async fn set_active_source(pool: &SqlitePool, source: &Source) -> Result<(), DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    write_source_state(&mut tx, Some(source), None).await?;
    tx.commit().await
}

pub async fn set_device_id(pool: &SqlitePool, device_id: &str) -> Result<(), DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    write_source_state(&mut tx, None, Some(device_id)).await?;
    tx.commit().await
}

pub async fn save_server(pool: &SqlitePool, server: &SavedServer) -> Result<(), DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    upsert_server_row(
        &mut tx,
        &server.id,
        &server.name,
        &server.url,
        service_str(server.service),
        now_secs(),
    )
    .await?;
    let browser = server.yt_browser.map(browser_str);
    let options = server_options(
        browser.as_deref(),
        server.yt_anonymous,
        server.yt_profile.as_deref(),
        server.account.as_deref(),
        &server.apple_music_storefront,
        &server.apple_music_language,
    );
    write_server_options(&mut tx, &server.id, &options).await?;
    tx.commit().await
}

pub async fn save_active_server(pool: &SqlitePool, server: &MusicServer) -> Result<(), DbError> {
    let now = now_secs();
    let id = server
        .id
        .clone()
        .unwrap_or_else(|| format!("legacy-{}", service_str(server.service)));
    let mut tx = super::begin_immediate(pool).await?;
    upsert_server_row(
        &mut tx,
        &id,
        &server.name,
        &server.url,
        service_str(server.service),
        now,
    )
    .await?;
    let browser = server.yt_browser.map(browser_str);
    let options = server_options(
        browser.as_deref(),
        server.yt_anonymous,
        server.yt_profile.as_deref(),
        server.account.as_deref(),
        &server.apple_music_storefront,
        &server.apple_music_language,
    );
    write_server_options(&mut tx, &id, &options).await?;
    write_server_credentials(
        &mut tx,
        &id,
        server.access_token.as_deref(),
        server.user_id.as_deref(),
        now,
    )
    .await?;
    tx.commit().await
}

pub async fn delete_server(pool: &SqlitePool, id: &str) -> Result<(), DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    purge_source(&mut tx, id).await?;
    sqlx::query("DELETE FROM servers WHERE id = ?1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// The legacy importers' settings go to the settings file, every key of them, which is only rewritten when one of them changed.
pub(crate) fn write_settings(settings: &Settings, settings_path: &Path) -> Result<(), DbError> {
    let every_key: Vec<config::SettingKey> = config::SettingKey::iter().collect();
    config::store::save_settings(settings_path, settings, &every_key)
        .map_err(|error| DbError::Io(format!("{}: {error}", settings_path.display())))?;
    Ok(())
}

/// Hydrate one server row (creds included) — the server-switch path, so stored
/// creds are reused instead of re-prompting sign-in.
pub async fn load_server(pool: &SqlitePool, id: &str) -> Result<Option<MusicServer>, DbError> {
    Ok(stored_servers(pool, Some(id))
        .await?
        .first()
        .map(StoredServer::music_server))
}

/// A server's identity row with its credentials and the options it set.
struct StoredServer {
    id: String,
    name: String,
    url: String,
    service: MusicService,
    access_token: Option<String>,
    user_id: Option<String>,
    options: std::collections::HashMap<String, String>,
}

impl StoredServer {
    fn option_or(&self, key: &str, default: String) -> String {
        self.options.get(key).cloned().unwrap_or(default)
    }

    fn browser(&self) -> Option<Browser> {
        parse_browser(self.options.get(OPT_BROWSER).map(String::as_str))
    }

    fn profile(&self) -> Option<String> {
        self.options.get(OPT_PROFILE).cloned()
    }

    fn account(&self) -> Option<String> {
        self.options.get(OPT_ACCOUNT).cloned()
    }

    fn anonymous(&self) -> bool {
        self.options
            .get(OPT_ANONYMOUS)
            .is_some_and(|value| value == "1")
    }

    fn saved(&self) -> SavedServer {
        let defaults = MusicServer::default();
        SavedServer {
            id: self.id.clone(),
            name: self.name.clone(),
            url: self.url.clone(),
            service: self.service,
            yt_browser: self.browser(),
            yt_anonymous: self.anonymous(),
            yt_profile: self.profile(),
            account: self.account(),
            apple_music_storefront: self.option_or(OPT_STOREFRONT, defaults.apple_music_storefront),
            apple_music_language: self.option_or(OPT_LANGUAGE, defaults.apple_music_language),
        }
    }

    fn music_server(&self) -> MusicServer {
        let defaults = MusicServer::default();
        MusicServer {
            name: self.name.clone(),
            url: self.url.clone(),
            service: self.service,
            access_token: self.access_token.clone(),
            user_id: self.user_id.clone(),
            id: Some(self.id.clone()),
            yt_browser: self.browser(),
            yt_anonymous: self.anonymous(),
            yt_profile: self.profile(),
            account: self.account(),
            apple_music_storefront: self.option_or(OPT_STOREFRONT, defaults.apple_music_storefront),
            apple_music_language: self.option_or(OPT_LANGUAGE, defaults.apple_music_language),
        }
    }
}

const OPT_BROWSER: &str = "yt_browser";
const OPT_ANONYMOUS: &str = "yt_anonymous";
const OPT_PROFILE: &str = "yt_profile";
const OPT_ACCOUNT: &str = "account";
const OPT_STOREFRONT: &str = "apple_music_storefront";
const OPT_LANGUAGE: &str = "apple_music_language";

/// Every server, or the one `id` names, with its credentials and options joined on.
async fn stored_servers(pool: &SqlitePool, id: Option<&str>) -> Result<Vec<StoredServer>, DbError> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.url, s.service, c.access_token, c.user_id \
           FROM servers s LEFT JOIN server_credentials c ON c.server_id = s.id \
          WHERE ?1 IS NULL OR s.id = ?1",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let options: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT server_id, key, value FROM server_settings WHERE ?1 IS NULL OR server_id = ?1",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let mut by_server: std::collections::HashMap<
        String,
        std::collections::HashMap<String, String>,
    > = std::collections::HashMap::new();
    for (server, key, value) in options {
        by_server.entry(server).or_default().insert(key, value);
    }
    Ok(rows
        .iter()
        .map(|row| {
            let id: String = row.get("id");
            StoredServer {
                options: by_server.remove(&id).unwrap_or_default(),
                name: row.get("name"),
                url: row.get("url"),
                service: parse_service(row.get::<String, _>("service").as_str()),
                access_token: row.get("access_token"),
                user_id: row.get("user_id"),
                id,
            }
        })
        .collect())
}

/// The options a server stores: only what differs from the defaults, so most servers store none.
pub(crate) fn server_options(
    browser: Option<&str>,
    anonymous: bool,
    profile: Option<&str>,
    account: Option<&str>,
    storefront: &str,
    language: &str,
) -> Vec<(&'static str, String)> {
    let defaults = MusicServer::default();
    let mut rows = Vec::new();
    if let Some(browser) = browser {
        rows.push((OPT_BROWSER, browser.to_string()));
    }
    if anonymous {
        rows.push((OPT_ANONYMOUS, "1".to_string()));
    }
    if let Some(profile) = profile.filter(|profile| !profile.is_empty()) {
        rows.push((OPT_PROFILE, profile.to_string()));
    }
    if let Some(account) = account.filter(|account| !account.is_empty()) {
        rows.push((OPT_ACCOUNT, account.to_string()));
    }
    if storefront != defaults.apple_music_storefront {
        rows.push((OPT_STOREFRONT, storefront.to_string()));
    }
    if language != defaults.apple_music_language {
        rows.push((OPT_LANGUAGE, language.to_string()));
    }
    rows
}

pub(crate) async fn write_server_options(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    options: &[(&str, String)],
) -> Result<(), DbError> {
    sqlx::query("DELETE FROM server_settings WHERE server_id = ?1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    for (key, value) in options {
        sqlx::query("INSERT INTO server_settings (server_id, key, value) VALUES (?1, ?2, ?3)")
            .bind(id)
            .bind(key)
            .bind(value)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Store a server's credentials, or drop them when there is no token to keep.
pub(crate) async fn write_server_credentials(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    access_token: Option<&str>,
    user_id: Option<&str>,
    now: i64,
) -> Result<(), DbError> {
    match access_token {
        Some(token) => {
            sqlx::query(
                "INSERT INTO server_credentials (server_id, access_token, user_id, updated_at) \
                 SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM servers WHERE id = ?1) \
                 ON CONFLICT(server_id) DO UPDATE SET access_token = ?2, user_id = ?3, updated_at = ?4",
            )
            .bind(id)
            .bind(token)
            .bind(user_id)
            .bind(now)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM server_credentials WHERE server_id = ?1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// Increment one track's play count and stamp when it was played (1-row upsert — no whole-blob rewrite).
pub async fn bump_listen_count(
    pool: &SqlitePool,
    source: &Source,
    track_key: &str,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO listen_counts (source, track_key, count, last_played_at) \
         VALUES (?1, ?2, 1, unixepoch()) \
         ON CONFLICT(source, track_key) DO UPDATE \
         SET count = count + 1, last_played_at = excluded.last_played_at",
    )
    .bind(source.as_str())
    .bind(track_key)
    .execute(pool)
    .await?;
    Ok(())
}

/// One source's recently-played track keys, newest first.
pub async fn recently_played(
    pool: &SqlitePool,
    source: &Source,
    limit: u32,
) -> Result<Vec<String>, DbError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT track_key FROM recently_played WHERE source = ?1 \
         ORDER BY played_at DESC LIMIT ?2",
    )
    .bind(source.as_str())
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Record a play for this source: move the key to the front (a monotonic
/// per-source rank — `MAX+1`, so rapid plays can't tie like a ms timestamp
/// would), then trim the source's history to [`RECENT_LIMIT`]. A per-play
/// handful of statements — no whole-blob rewrite.
pub async fn push_recent(pool: &SqlitePool, source: &Source, key: &str) -> Result<(), DbError> {
    let src = source.as_str();
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO recently_played (source, track_key, played_at) \
         VALUES (?1, ?2, (SELECT COALESCE(MAX(played_at), 0) + 1 \
                          FROM recently_played WHERE source = ?1)) \
         ON CONFLICT(source, track_key) DO UPDATE SET played_at = excluded.played_at",
    )
    .bind(src)
    .bind(key)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM recently_played WHERE source = ?1 AND track_key NOT IN \
         (SELECT track_key FROM recently_played WHERE source = ?1 \
          ORDER BY played_at DESC LIMIT ?2)",
    )
    .bind(src)
    .bind(RECENT_LIMIT)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

fn parse_service(s: &str) -> MusicService {
    match s {
        "Subsonic" => MusicService::Subsonic,
        "Custom" => MusicService::Custom,
        "YtMusic" => MusicService::YtMusic,
        "SoundCloud" => MusicService::SoundCloud,
        "AppleMusic" => MusicService::AppleMusic,
        "Spotify" => MusicService::Spotify,
        "Nextcloud" => MusicService::Nextcloud,
        "Clippsly" => MusicService::Clippsly,
        "Smb" => MusicService::Smb,
        "AudioCd" => MusicService::AudioCd,
        _ => MusicService::Jellyfin,
    }
}

fn service_str(s: MusicService) -> &'static str {
    match s {
        MusicService::Jellyfin => "Jellyfin",
        MusicService::Subsonic => "Subsonic",
        MusicService::Custom => "Custom",
        MusicService::YtMusic => "YtMusic",
        MusicService::SoundCloud => "SoundCloud",
        MusicService::AppleMusic => "AppleMusic",
        MusicService::Spotify => "Spotify",
        MusicService::Nextcloud => "Nextcloud",
        MusicService::Clippsly => "Clippsly",
        MusicService::Smb => "Smb",
        MusicService::AudioCd => "AudioCd",
    }
}

/// An unknown id (an older row, or one written by a newer build) reads back as
/// `None`, which the sign-in resolves to the system default browser.
fn parse_browser(s: Option<&str>) -> Option<Browser> {
    s.and_then(Browser::from_id)
}

fn browser_str(b: Browser) -> String {
    b.id().to_string()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Store one server's credentials without touching the rest of its row.
pub async fn set_server_credentials(
    pool: &SqlitePool,
    id: &str,
    access_token: Option<&str>,
    user_id: Option<&str>,
) -> Result<(), DbError> {
    let mut conn = pool.acquire().await?;
    write_server_credentials(&mut conn, id, access_token, user_id, now_secs()).await
}

/// Removable media are session-only; recover their cached rows after a crash too.
pub(super) async fn purge_removable(pool: &SqlitePool) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    let sources = sqlx::query_scalar::<_, String>(
        "SELECT source FROM tracks WHERE source GLOB 'removable:*' \
         UNION SELECT source FROM albums WHERE source GLOB 'removable:*' \
         UNION SELECT source FROM queue_state WHERE source GLOB 'removable:*' \
         UNION SELECT source FROM recently_played WHERE source GLOB 'removable:*' \
         UNION SELECT source FROM listen_counts WHERE source GLOB 'removable:*' \
         UNION SELECT server_id FROM favorites WHERE server_id GLOB 'removable:*'",
    )
    .fetch_all(&mut *tx)
    .await?;
    for source in sources {
        purge_source(&mut tx, &source).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Everything a source left behind: its rows name it by text, so nothing cascades from `servers`.
pub(super) async fn purge_source(
    conn: &mut sqlx::SqliteConnection,
    source: &str,
) -> Result<(), DbError> {
    for sql in [
        "DELETE FROM tracks WHERE source = ?1",
        "DELETE FROM albums WHERE source = ?1",
        "DELETE FROM artists WHERE source = ?1",
        "DELETE FROM playlists WHERE source = ?1",
        "DELETE FROM favorites WHERE server_id = ?1",
        "DELETE FROM recently_played WHERE source = ?1",
        "DELETE FROM listen_counts WHERE source = ?1",
        "DELETE FROM kv WHERE kind = ?1",
        "DELETE FROM artist_images WHERE source = ?1",
        "DELETE FROM queue_tracks WHERE source = ?1",
        "DELETE FROM queue_state WHERE source = ?1",
    ] {
        sqlx::query(sql).bind(source).execute(&mut *conn).await?;
    }
    let miss_prefix = format!("{source}\u{1f}");
    sqlx::query("DELETE FROM kv WHERE kind = ?1 AND substr(name, 1, length(?2)) = ?2")
        .bind(crate::ARTIST_PHOTO_MISS_KIND)
        .bind(&miss_prefix)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Insert or rename one server's identity row.
pub(crate) async fn upsert_server_row(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    name: &str,
    url: &str,
    service: &str,
    now: i64,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO servers (id, name, url, service, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(id) DO UPDATE SET name = ?2, url = ?3, service = ?4, updated_at = ?5",
    )
    .bind(id)
    .bind(name)
    .bind(url)
    .bind(service)
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QueueSnapshot;

    /// A real file with the production pool settings: WAL, five connections,
    /// a 5 s busy timeout. An in-memory pool gives every connection its own
    /// database, which cannot contend with itself.
    async fn file_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = crate::backend::open_pool(&dir.path().join("t.db"))
            .await
            .expect("pool");
        crate::backend::migrations::run_migrations(&pool, None)
            .await
            .expect("migrate");
        (dir, pool)
    }

    /// The SQLite rule the fix rests on, made executable: a deferred
    /// transaction that reads first cannot upgrade to a write once another
    /// connection has committed, and the busy handler is not consulted.
    #[tokio::test]
    async fn a_deferred_read_then_write_is_refused_after_a_concurrent_commit() {
        let (_dir, pool) = file_pool().await;
        let source = Source::default();

        let mut reader = pool.begin().await.expect("begin");
        let _: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(played_at), 0) FROM recently_played")
            .fetch_one(&mut *reader)
            .await
            .expect("read under a shared lock");

        push_recent(&pool, &source, "/other.flac")
            .await
            .expect("another connection commits meanwhile");

        let started = std::time::Instant::now();
        let refused = sqlx::query(
            "INSERT INTO recently_played (source, track_key, played_at) VALUES ('local', '/a', 1)",
        )
        .execute(&mut *reader)
        .await;

        assert!(refused.is_err(), "the upgrade must be refused: {refused:?}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "refused at once, not after the 5 s busy timeout: {:?}",
            started.elapsed()
        );
    }

    /// Eight recents landing while the queue is being saved underneath them.
    /// Every writer is write-first now, so they queue on the busy timeout and
    /// all of them succeed.
    #[tokio::test]
    async fn concurrent_recents_and_queue_saves_all_land() {
        let (_dir, pool) = file_pool().await;
        let source = Source::default();
        let snapshot = QueueSnapshot::default();

        let recent = |key: &'static str| {
            let pool = pool.clone();
            let source = source.clone();
            async move { push_recent(&pool, &source, key).await }
        };
        let saver = {
            let pool = pool.clone();
            let source = source.clone();
            async move {
                for _ in 0..20 {
                    crate::backend::writes::save_queue(&pool, &source, &snapshot).await?;
                }
                Ok::<(), DbError>(())
            }
        };

        let (a, b, c, d, e, f, g, h, saved) = tokio::join!(
            recent("/1"),
            recent("/2"),
            recent("/3"),
            recent("/4"),
            recent("/5"),
            recent("/6"),
            recent("/7"),
            recent("/8"),
            saver,
        );
        for (name, result) in [
            ("1", a),
            ("2", b),
            ("3", c),
            ("4", d),
            ("5", e),
            ("6", f),
            ("7", g),
            ("8", h),
        ] {
            assert!(result.is_ok(), "recent {name} failed: {result:?}");
        }
        assert!(saved.is_ok(), "queue saves failed: {saved:?}");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recently_played")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 8);
    }
}
