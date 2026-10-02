//! Config persistence: settings in the settings file (`config::store`), and state, credentials and library data in tables, hydrated into one `AppConfig`.

use std::collections::HashSet;
use std::path::Path;

use config::{AppConfig, Browser, MusicServer, MusicService, SavedServer, Source};
use sqlx::SqlitePool;

use crate::DbError;

/// How many recent entries to keep per source.
const RECENT_LIMIT: i64 = 50;

pub async fn load_config(
    pool: &SqlitePool,
    settings_path: &Path,
) -> Result<Option<AppConfig>, DbError> {
    let layers = config::store::FileLayers::read(settings_path);
    let state = sqlx::query!(
        "SELECT device_id, active_source, source_explicitly_set, volume, discord_presence_paused, \
                fullscreen_tabs_collapsed, sort_order, album_view_mode, artist_album_view_mode, \
                artists_view_mode, artist_view_order, listen_now_style, hero_height \
           FROM app_state WHERE id = 1"
    )
    .fetch_optional(pool)
    .await?;
    // Nothing saved and no settings file either: never configured.
    if state.is_none() && !layers.has_overrides() {
        return Ok(None);
    }
    let mut cfg: AppConfig = layers.merge_and_parse(serde_json::json!({}))?;

    if let Some(state) = state {
        cfg.device_id = state.device_id;
        cfg.active_source = Source::from_column(&state.active_source);
        cfg.source_explicitly_set = state.source_explicitly_set != 0;
        cfg.volume = state.volume as f32;
        cfg.discord_presence_paused = state.discord_presence_paused.map(|paused| paused != 0);
        cfg.fullscreen_tabs_collapsed = state.fullscreen_tabs_collapsed != 0;
        read_variant(&state.sort_order, &mut cfg.sort_order);
        read_variant(&state.album_view_mode, &mut cfg.album_view_mode);
        read_variant(
            &state.artist_album_view_mode,
            &mut cfg.artist_album_view_mode,
        );
        read_variant(&state.artists_view_mode, &mut cfg.artists_view_mode);
        read_variant(&state.artist_view_order, &mut cfg.artist_view_order);
        read_variant(&state.listen_now_style, &mut cfg.listen_now_style);
        cfg.hero_height = state.hero_height.clamp(0, u32::MAX as i64) as u32;
        cfg.album_sort = view_sort(pool, VIEW_ALBUMS).await?;
        cfg.library_sort = view_sort(pool, VIEW_LIBRARY).await?;
        cfg.artist_album_sort = view_sort(pool, VIEW_ARTIST_ALBUMS).await?;
        cfg.artist_sort = view_sort(pool, VIEW_ARTISTS).await?;
        cfg.sidebar_order = sqlx::query_scalar!("SELECT item FROM sidebar_items ORDER BY position")
            .fetch_all(pool)
            .await?;
        cfg.home_sections =
            sqlx::query!("SELECT key, enabled FROM home_sections ORDER BY position")
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(|row| config::HomeSection {
                    key: row.key,
                    enabled: row.enabled != 0,
                })
                .collect();
    }
    let credentials = sqlx::query!("SELECT key, value FROM integration_credentials")
        .fetch_all(pool)
        .await?;
    for row in credentials {
        if let Some(slot) = credential_mut(&mut cfg, &row.key) {
            *slot = row.value;
        }
    }
    cfg.ytdlp_history = sqlx::query!(
        "SELECT url, title, format, status, error FROM ytdlp_history ORDER BY position"
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| config::YtdlpHistoryEntry {
        url: row.url,
        title: row.title,
        format: row.format,
        status: row.status,
        error: row.error,
    })
    .collect();
    let folders =
        sqlx::query!("SELECT server_id, path FROM server_folders ORDER BY server_id, position")
            .fetch_all(pool)
            .await?;
    cfg.server_folders = std::collections::HashMap::new();
    for row in folders {
        cfg.server_folders
            .entry(row.server_id)
            .or_default()
            .push(row.path);
    }
    // The in-memory shape migrations the legacy file load used to run.
    cfg.migrate_home_sections();
    cfg.migrate_sidebar_order();
    cfg.migrate_registry_paths();

    // Hydrate servers from their tables (creds included for the active one).
    let servers = stored_servers(pool, None).await?;
    cfg.servers = servers.iter().map(StoredServer::saved).collect();
    cfg.server = cfg.active_source.server_id().and_then(|active| {
        servers
            .iter()
            .find(|server| server.id == *active)
            .map(StoredServer::music_server)
    });

    // Hydrate play counts under the uid keys every reader of the map looks them up by.
    let counts = sqlx::query!(
        "SELECT lc.source, lc.track_key, lc.count, s.service \
           FROM listen_counts lc LEFT JOIN servers s ON s.id = lc.source"
    )
    .fetch_all(pool)
    .await?;
    cfg.listen_counts = counts
        .into_iter()
        .map(|r| {
            let uid = match r.service {
                Some(service) => format!("{}:{}", service.to_lowercase(), r.track_key),
                None => r.track_key,
            };
            let key = Source::from_column(&r.source).listen_count_key(&uid);
            (key, r.count.max(0) as u64)
        })
        .collect();

    let offline = sqlx::query!("SELECT item_id, path FROM offline_tracks")
        .fetch_all(pool)
        .await?;
    cfg.offline_tracks = offline.into_iter().map(|r| (r.item_id, r.path)).collect();
    cfg.pinned_stations =
        sqlx::query_scalar!("SELECT manifest FROM pinned_stations ORDER BY position")
            .fetch_all(pool)
            .await?;

    Ok(Some(cfg))
}

const VIEW_ALBUMS: &str = "albums";
const VIEW_LIBRARY: &str = "library";
const VIEW_ARTIST_ALBUMS: &str = "artist_albums";
const VIEW_ARTISTS: &str = "artists";

/// A unit variant's serde name, which is what a text column stores.
fn variant<T: serde::Serialize>(value: &T) -> Result<String, DbError> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(name) => Ok(name),
        other => Err(DbError::Serde(format!("{other} is not a unit variant"))),
    }
}

fn parse_variant<T: serde::de::DeserializeOwned>(name: String) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(name)).ok()
}

/// A variant no build knows any more leaves the default in place rather than failing the load.
fn read_variant<T: serde::de::DeserializeOwned>(name: &str, slot: &mut T) {
    if let Some(value) = parse_variant(name.to_owned()) {
        *slot = value;
    }
}

async fn view_sort<F: serde::de::DeserializeOwned>(
    pool: &SqlitePool,
    view: &str,
) -> Result<Vec<config::SortCriterion<F>>, DbError> {
    let rows = sqlx::query!(
        "SELECT field, direction FROM view_sorts WHERE view = ?1 ORDER BY position",
        view
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(config::SortCriterion::new(
                parse_variant(row.field)?,
                parse_variant(row.direction)?,
            ))
        })
        .collect())
}

/// The config field a stored credential fills.
fn credential_mut<'a>(cfg: &'a mut AppConfig, key: &str) -> Option<&'a mut String> {
    Some(match key {
        "musicbrainz_token" => &mut cfg.musicbrainz_token,
        "lastfm_api_key" => &mut cfg.lastfm_api_key,
        "lastfm_api_secret" => &mut cfg.lastfm_api_secret,
        "lastfm_session_key" => &mut cfg.lastfm_session_key,
        "librefm_api_key" => &mut cfg.librefm_api_key,
        "librefm_api_secret" => &mut cfg.librefm_api_secret,
        "librefm_session_key" => &mut cfg.librefm_session_key,
        _ => return None,
    })
}

fn credentials(cfg: &AppConfig) -> [(&'static str, &str); 7] {
    [
        ("musicbrainz_token", &cfg.musicbrainz_token),
        ("lastfm_api_key", &cfg.lastfm_api_key),
        ("lastfm_api_secret", &cfg.lastfm_api_secret),
        ("lastfm_session_key", &cfg.lastfm_session_key),
        ("librefm_api_key", &cfg.librefm_api_key),
        ("librefm_api_secret", &cfg.librefm_api_secret),
        ("librefm_session_key", &cfg.librefm_session_key),
    ]
}

#[tracing::instrument(name = "config.save", skip_all)]
pub async fn save_config(
    pool: &SqlitePool,
    cfg: &AppConfig,
    settings_path: &Path,
) -> Result<(), DbError> {
    // It can read before it writes, and a deferred BEGIN then fails at once against a write that landed meanwhile.
    let mut tx = super::begin_immediate(pool).await?;
    let active = sync_servers(&mut tx, cfg).await?;
    write_state(&mut tx, cfg, &active).await?;
    tx.commit().await?;
    write_settings(cfg, settings_path)
}

/// Sync the saved servers and the active one's creds; answers the active source with any resolved server id stamped in.
async fn sync_servers(tx: &mut sqlx::SqliteConnection, cfg: &AppConfig) -> Result<Source, DbError> {
    let now = now_secs();
    // Non-cred fields only: the in-memory config carries no other server's creds to write.
    for s in &cfg.servers {
        upsert_server_row(tx, &s.id, &s.name, &s.url, service_str(s.service), now).await?;
        let browser = s.yt_browser.map(browser_str);
        let options = server_options(
            browser.as_deref(),
            s.yt_anonymous,
            &s.apple_music_storefront,
            &s.apple_music_language,
        );
        write_server_options(tx, &s.id, &options).await?;
    }

    let mut active_id: Option<String> = cfg.active_source.server_id().map(String::from);
    if let Some(srv) = &cfg.server {
        let id = srv
            .id
            .clone()
            .or_else(|| cfg.active_source.server_id().map(String::from))
            .unwrap_or_else(|| format!("legacy-{}", service_str(srv.service)));
        upsert_server_row(tx, &id, &srv.name, &srv.url, service_str(srv.service), now).await?;
        let browser = srv.yt_browser.map(browser_str);
        let options = server_options(
            browser.as_deref(),
            srv.yt_anonymous,
            &srv.apple_music_storefront,
            &srv.apple_music_language,
        );
        write_server_options(tx, &id, &options).await?;
        write_server_credentials(
            tx,
            &id,
            srv.access_token.as_deref(),
            srv.user_id.as_deref(),
            now,
        )
        .await?;
        active_id = Some(id);
    }

    // Drop server rows the user removed (keep the active one regardless).
    let keep: HashSet<&str> = cfg
        .servers
        .iter()
        .map(|s| s.id.as_str())
        .chain(active_id.as_deref())
        .collect();
    let existing: Vec<String> = sqlx::query_scalar!("SELECT id FROM servers")
        .fetch_all(&mut *tx)
        .await?;
    for id in existing {
        if !keep.contains(id.as_str()) {
            purge_source(tx, &id).await?;
            sqlx::query!("DELETE FROM servers WHERE id = ?1", id)
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(match active_id {
        Some(id) => Source::Server(id),
        None => cfg.active_source.clone(),
    })
}

/// State, credentials and the lists the app keeps; play counts, offline copies and pins each have their own writer.
pub(crate) async fn write_state(
    conn: &mut sqlx::SqliteConnection,
    cfg: &AppConfig,
    active: &Source,
) -> Result<(), DbError> {
    let active = active.as_str();
    let explicit = cfg.source_explicitly_set as i64;
    let volume = f64::from(cfg.volume);
    let paused = cfg.discord_presence_paused.map(i64::from);
    let collapsed = cfg.fullscreen_tabs_collapsed as i64;
    let sort_order = variant(&cfg.sort_order)?;
    let album_view_mode = variant(&cfg.album_view_mode)?;
    let artist_album_view_mode = variant(&cfg.artist_album_view_mode)?;
    let artists_view_mode = variant(&cfg.artists_view_mode)?;
    let artist_view_order = variant(&cfg.artist_view_order)?;
    let listen_now_style = variant(&cfg.listen_now_style)?;
    let hero_height = i64::from(cfg.hero_height);
    sqlx::query!(
        "INSERT INTO app_state (id, device_id, active_source, source_explicitly_set, volume, \
           discord_presence_paused, fullscreen_tabs_collapsed, sort_order, album_view_mode, \
           artist_album_view_mode, artists_view_mode, artist_view_order, listen_now_style, hero_height) \
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
         ON CONFLICT(id) DO UPDATE SET device_id = ?1, active_source = ?2, source_explicitly_set = ?3, \
           volume = ?4, discord_presence_paused = ?5, fullscreen_tabs_collapsed = ?6, sort_order = ?7, \
           album_view_mode = ?8, artist_album_view_mode = ?9, artists_view_mode = ?10, \
           artist_view_order = ?11, listen_now_style = ?12, hero_height = ?13",
        cfg.device_id,
        active,
        explicit,
        volume,
        paused,
        collapsed,
        sort_order,
        album_view_mode,
        artist_album_view_mode,
        artists_view_mode,
        artist_view_order,
        listen_now_style,
        hero_height
    )
    .execute(&mut *conn)
    .await?;

    sqlx::query!("DELETE FROM view_sorts")
        .execute(&mut *conn)
        .await?;
    write_view_sort(conn, VIEW_ALBUMS, &cfg.album_sort).await?;
    write_view_sort(conn, VIEW_LIBRARY, &cfg.library_sort).await?;
    write_view_sort(conn, VIEW_ARTIST_ALBUMS, &cfg.artist_album_sort).await?;
    write_view_sort(conn, VIEW_ARTISTS, &cfg.artist_sort).await?;

    sqlx::query!("DELETE FROM sidebar_items")
        .execute(&mut *conn)
        .await?;
    for (position, item) in cfg.sidebar_order.iter().enumerate() {
        let position = position as i64;
        sqlx::query!(
            "INSERT INTO sidebar_items (position, item) VALUES (?1, ?2)",
            position,
            item
        )
        .execute(&mut *conn)
        .await?;
    }

    sqlx::query!("DELETE FROM home_sections")
        .execute(&mut *conn)
        .await?;
    for (position, section) in cfg.home_sections.iter().enumerate() {
        let position = position as i64;
        let enabled = section.enabled as i64;
        sqlx::query!(
            "INSERT INTO home_sections (position, key, enabled) VALUES (?1, ?2, ?3)",
            position,
            section.key,
            enabled
        )
        .execute(&mut *conn)
        .await?;
    }

    sqlx::query!("DELETE FROM integration_credentials")
        .execute(&mut *conn)
        .await?;
    for (key, value) in credentials(cfg) {
        if value.is_empty() {
            continue;
        }
        sqlx::query!(
            "INSERT INTO integration_credentials (key, value) VALUES (?1, ?2)",
            key,
            value
        )
        .execute(&mut *conn)
        .await?;
    }

    sqlx::query!("DELETE FROM ytdlp_history")
        .execute(&mut *conn)
        .await?;
    for (position, entry) in cfg.ytdlp_history.iter().enumerate() {
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

    sqlx::query!("DELETE FROM server_folders")
        .execute(&mut *conn)
        .await?;
    for (server, paths) in &cfg.server_folders {
        for (position, path) in paths.iter().enumerate() {
            let position = position as i64;
            // A folder list for a server the app no longer has would name nothing.
            sqlx::query!(
                "INSERT INTO server_folders (server_id, position, path) \
                 SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM servers WHERE id = ?1)",
                server,
                position,
                path
            )
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

async fn write_view_sort<F: serde::Serialize>(
    conn: &mut sqlx::SqliteConnection,
    view: &str,
    criteria: &[config::SortCriterion<F>],
) -> Result<(), DbError> {
    for (position, criterion) in criteria.iter().enumerate() {
        let position = position as i64;
        let field = variant(&criterion.field)?;
        let direction = variant(&criterion.direction)?;
        sqlx::query!(
            "INSERT INTO view_sorts (view, position, field, direction) VALUES (?1, ?2, ?3, ?4)",
            view,
            position,
            field,
            direction
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The settings go to the settings file, which is only rewritten when one of them changed.
pub(crate) fn write_settings(cfg: &AppConfig, settings_path: &Path) -> Result<(), DbError> {
    let layers = config::store::FileLayers::read(settings_path);
    let settings = serde_json::to_value(cfg)?;
    config::store::save_settings_file(settings_path, &settings, &layers.locked_keys)
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
            apple_music_storefront: self.option_or(OPT_STOREFRONT, defaults.apple_music_storefront),
            apple_music_language: self.option_or(OPT_LANGUAGE, defaults.apple_music_language),
        }
    }
}

const OPT_BROWSER: &str = "yt_browser";
const OPT_ANONYMOUS: &str = "yt_anonymous";
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

/// Increment one track's play count (1-row upsert — no whole-blob rewrite).
pub async fn bump_listen_count(
    pool: &SqlitePool,
    source: &Source,
    track_key: &str,
) -> Result<(), DbError> {
    let src = source.as_str();
    sqlx::query!(
        "INSERT INTO listen_counts (source, track_key, count) VALUES (?1, ?2, 1) \
         ON CONFLICT(source, track_key) DO UPDATE SET count = count + 1",
        src,
        track_key
    )
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
    // The next position is computed inside the INSERT rather than SELECTed
    // first: a deferred transaction that reads before it writes holds a shared
    // lock it then cannot upgrade if anyone commits in between -- SQLite
    // answers SQLITE_BUSY at once, busy_timeout notwithstanding.
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
///
/// `save_config` only persists credentials for the *active* server, because
/// the in-memory config holds no others. Signing into a server before
/// switching to it needs this narrower write.
pub async fn set_server_credentials(
    pool: &SqlitePool,
    id: &str,
    access_token: Option<&str>,
    user_id: Option<&str>,
) -> Result<(), DbError> {
    let mut conn = pool.acquire().await?;
    write_server_credentials(&mut conn, id, access_token, user_id, now_secs()).await
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
