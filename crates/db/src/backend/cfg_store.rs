//! Config persistence as a DB-backed cache of the in-memory `AppConfig` (#347,
//! step 4), layered with the standalone settings file (#530).
//!
//! The single-row `app_config` blob holds everything EXCEPT creds and play
//! counts: `server`/`servers` live in the `servers` table (creds with the
//! server), `listen_counts` in its own table. [`load_config`] hydrates those
//! back onto the `AppConfig` the UI reads; [`save_config`] strips them out of
//! the blob and syncs the tables. Net effect: same `AppConfig` shape in memory,
//! creds never in the blob.
//!
//! On top of the blob sits the settings file (`settings.toml` + drop-ins +
//! env, see `config::store`): its keys override the blob on load, and every
//! save mirrors the settings back into it — unless it is Nix-managed
//! (immutable), in which case the blob alone keeps persisting runtime state
//! and the file's keys simply keep winning. Keys a layer the app cannot write
//! pins (managed file, drop-in, env) are excluded from both persisted forms:
//! their value belongs to the layer, so saving it as base config would keep it
//! applying once the layer is removed.

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
    let json: Option<String> = sqlx::query_scalar!("SELECT json FROM app_config WHERE id = 1")
        .fetch_optional(pool)
        .await?;

    let layers = config::store::FileLayers::read(settings_path);
    // First launch with no settings file either: report "never configured".
    if json.is_none() && !layers.has_overrides() {
        return Ok(None);
    }

    let value: serde_json::Value = match &json {
        Some(json) => serde_json::from_str(json)?,
        None => serde_json::json!({}),
    };
    let mut cfg: AppConfig = layers.merge_and_parse(value)?;
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

    // Hydrate play counts.
    let counts = sqlx::query!("SELECT track_key, count FROM listen_counts")
        .fetch_all(pool)
        .await?;
    cfg.listen_counts = counts
        .into_iter()
        .map(|r| (r.track_key, r.count.max(0) as u64))
        .collect();

    Ok(Some(cfg))
}

#[tracing::instrument(name = "config.save", skip_all)]
pub async fn save_config(
    pool: &SqlitePool,
    cfg: &AppConfig,
    settings_path: &Path,
) -> Result<(), DbError> {
    let now = now_secs();
    let mut tx = pool.begin().await?;

    // Sync the saved-servers list (non-cred fields only — never clobber a stored
    // token from the in-memory cache, which doesn't carry other servers' creds).
    for s in &cfg.servers {
        upsert_server_row(&mut tx, &s.id, &s.name, &s.url, service_str(s.service), now).await?;
        let browser = s.yt_browser.map(browser_str);
        let options = server_options(
            browser.as_deref(),
            s.yt_anonymous,
            &s.apple_music_storefront,
            &s.apple_music_language,
        );
        write_server_options(&mut tx, &s.id, &options).await?;
    }

    // Upsert the active server WITH its creds, and remember its id for the blob.
    let mut active_id: Option<String> = cfg.active_source.server_id().map(String::from);
    if let Some(srv) = &cfg.server {
        let id = srv
            .id
            .clone()
            .or_else(|| cfg.active_source.server_id().map(String::from))
            .unwrap_or_else(|| format!("legacy-{}", service_str(srv.service)));
        upsert_server_row(
            &mut tx,
            &id,
            &srv.name,
            &srv.url,
            service_str(srv.service),
            now,
        )
        .await?;
        let browser = srv.yt_browser.map(browser_str);
        let options = server_options(
            browser.as_deref(),
            srv.yt_anonymous,
            &srv.apple_music_storefront,
            &srv.apple_music_language,
        );
        write_server_options(&mut tx, &id, &options).await?;
        write_server_credentials(
            &mut tx,
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
            sqlx::query!("DELETE FROM servers WHERE id = ?1", id)
                .execute(&mut *tx)
                .await?;
        }
    }

    // Play counts are NOT synced here: `bump_listen_count` is their sole writer
    // (a per-play 1-row upsert). Looping the whole map made every config save
    // cost hundreds of statements — the downloads-stutter bug.

    // Store the blob, stripped of creds/servers/counts, stamped with the active id.
    let layers = config::store::FileLayers::read(settings_path);
    let mut blob = serde_json::to_value(cfg)?;
    if let Some(obj) = blob.as_object_mut() {
        obj.remove("server");
        obj.remove("servers");
        obj.remove("listen_counts");
        // Preserve local-library selections; only a server snapshot may need
        // its generated/resolved id stamped into the typed source.
        obj.insert(
            "active_source".into(),
            match &active_id {
                Some(id) => serde_json::json!({ "Server": id }),
                None => serde_json::to_value(&cfg.active_source)?,
            },
        );
    }
    // A key pinned by an unwritable layer holds that layer's value, merged in
    // by `load_config` — persisting it would make the override the base config
    // and keep it applying after the layer is gone. Keep what the blob had.
    if !layers.locked_keys.is_empty() {
        let prior: Option<String> = sqlx::query_scalar!("SELECT json FROM app_config WHERE id = 1")
            .fetch_optional(&mut *tx)
            .await?;
        let prior: serde_json::Value = prior
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(obj) = blob.as_object_mut() {
            for key in &layers.locked_keys {
                match prior.get(key.as_str()) {
                    Some(value) => obj.insert(key.clone(), value.clone()),
                    None => obj.remove(key.as_str()),
                };
            }
        }
    }

    let blob_str = serde_json::to_string(&blob)?;
    sqlx::query!(
        "INSERT INTO app_config (id, json) VALUES (1, ?1) \
         ON CONFLICT(id) DO UPDATE SET json = ?1",
        blob_str
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // Mirror the settings into the standalone file too (skipped when it is
    // Nix-managed). Best-effort: the DB save above already succeeded, and a
    // missing file write only means the blob's values apply on next load.
    if let Err(e) = config::store::save_settings_file(settings_path, &blob, &layers.locked_keys) {
        tracing::warn!(path = %settings_path.display(), "failed to write settings file: {e}");
    }

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
    track_uid: &str,
) -> Result<(), DbError> {
    let key = source.listen_count_key(track_uid);
    sqlx::query!(
        "INSERT INTO listen_counts (track_key, count) VALUES (?1, 1) \
         ON CONFLICT(track_key) DO UPDATE SET count = count + 1",
        key
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
        crate::backend::migrations::run_migrations(&pool)
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
        let source = Source::Local;

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
        let source = Source::Local;
        let snapshot = QueueSnapshot::default();

        let recent = |key: &'static str| {
            let pool = pool.clone();
            let source = source.clone();
            async move { push_recent(&pool, &source, key).await }
        };
        let saver = {
            let pool = pool.clone();
            async move {
                for _ in 0..20 {
                    crate::backend::writes::save_queue(&pool, &snapshot).await?;
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
