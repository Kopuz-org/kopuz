//! Config persistence as a DB-backed cache of the in-memory `AppConfig` (#347,
//! step 4).
//!
//! The single-row `app_config` blob holds everything EXCEPT creds and play
//! counts: `server`/`servers` live in the `servers` table (creds with the
//! server), `listen_counts` in its own table. [`load_config`] hydrates those
//! back onto the `AppConfig` the UI reads; [`save_config`] strips them out of
//! the blob and syncs the tables. Net effect: same `AppConfig` shape in memory,
//! creds never in the blob.

use std::collections::HashSet;

use config::{AppConfig, Browser, MusicServer, MusicService, SavedServer, Source};
use sqlx::SqlitePool;

use crate::DbError;

/// How many recent entries to keep per source.
const RECENT_LIMIT: i64 = 50;

pub async fn load_config(pool: &SqlitePool) -> Result<Option<AppConfig>, DbError> {
    let Some(json): Option<String> =
        sqlx::query_scalar!("SELECT json FROM app_config WHERE id = 1")
            .fetch_optional(pool)
            .await?
    else {
        return Ok(None);
    };

    let mut cfg: AppConfig = serde_json::from_str(&json)?;
    // The in-memory shape migrations the legacy file load used to run.
    cfg.migrate_home_sections();
    cfg.migrate_sidebar_order();
    cfg.migrate_registry_paths();

    // Hydrate servers from their table (creds included for the active one).
    use sqlx::Row;
    let rows: Vec<sqlx::sqlite::SqliteRow> = sqlx::query(
        "SELECT id, name, url, service, access_token, user_id, yt_browser, yt_anonymous, \
         apple_music_storefront, apple_music_language \
         FROM servers",
    )
    .fetch_all(pool)
    .await?;

    cfg.servers = rows
        .iter()
        .map(|r| SavedServer {
            id: r.get("id"),
            name: r.get("name"),
            url: r.get("url"),
            service: parse_service(r.get::<String, _>("service").as_str()),
            yt_browser: r
                .get::<Option<String>, _>("yt_browser")
                .as_deref()
                .and_then(|s| parse_browser(Some(s))),
            yt_anonymous: r.get::<i64, _>("yt_anonymous") != 0,
            apple_music_storefront: r.get("apple_music_storefront"),
            apple_music_language: r.get("apple_music_language"),
        })
        .collect();

    cfg.server = cfg.active_source.server_id().and_then(|active| {
        rows.iter()
            .find(|r| r.get::<String, _>("id") == *active)
            .map(|r| MusicServer {
                name: r.get("name"),
                url: r.get("url"),
                service: parse_service(r.get::<String, _>("service").as_str()),
                access_token: r.get("access_token"),
                user_id: r.get("user_id"),
                id: Some(r.get("id")),
                yt_browser: r
                    .get::<Option<String>, _>("yt_browser")
                    .as_deref()
                    .and_then(|s| parse_browser(Some(s))),
                yt_anonymous: r.get::<i64, _>("yt_anonymous") != 0,
                apple_music_storefront: r.get("apple_music_storefront"),
                apple_music_language: r.get("apple_music_language"),
            })
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
pub async fn save_config(pool: &SqlitePool, cfg: &AppConfig) -> Result<(), DbError> {
    let now = now_secs();
    let mut tx = pool.begin().await?;

    // Sync the saved-servers list (non-cred fields only — never clobber a stored
    // token from the in-memory cache, which doesn't carry other servers' creds).
    for s in &cfg.servers {
        let service = service_str(s.service);
        let browser = s.yt_browser.map(browser_str);
        let anon = s.yt_anonymous as i64;
        sqlx::query(
            "INSERT INTO servers (id, name, url, service, yt_browser, yt_anonymous, apple_music_storefront, apple_music_language, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(id) DO UPDATE SET name=?2, url=?3, service=?4, yt_browser=?5, \
               yt_anonymous=?6, apple_music_storefront=?7, apple_music_language=?8, updated_at=?9",
        )
        .bind(&s.id)
        .bind(&s.name)
        .bind(&s.url)
        .bind(service)
        .bind(browser)
        .bind(anon)
        .bind(&s.apple_music_storefront)
        .bind(&s.apple_music_language)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    // Upsert the active server WITH its creds, and remember its id for the blob.
    let mut active_id: Option<String> = cfg.active_source.server_id().map(String::from);
    if let Some(srv) = &cfg.server {
        let id = srv
            .id
            .clone()
            .or_else(|| cfg.active_source.server_id().map(String::from))
            .unwrap_or_else(|| format!("legacy-{}", service_str(srv.service)));
        let service = service_str(srv.service);
        let browser = srv.yt_browser.map(browser_str);
        let anon = srv.yt_anonymous as i64;
        let auth = if srv.access_token.is_some() || srv.yt_anonymous {
            "active"
        } else {
            "unauthenticated"
        };
        sqlx::query(
            "INSERT INTO servers \
               (id, name, url, service, access_token, user_id, yt_browser, yt_anonymous, apple_music_storefront, apple_music_language, auth_state, cred_updated_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12) \
             ON CONFLICT(id) DO UPDATE SET name=?2, url=?3, service=?4, access_token=?5, \
               user_id=?6, yt_browser=?7, yt_anonymous=?8, apple_music_storefront=?9, apple_music_language=?10, auth_state=?11, cred_updated_at=?12, updated_at=?12",
        )
        .bind(&id)
        .bind(&srv.name)
        .bind(&srv.url)
        .bind(service)
        .bind(&srv.access_token)
        .bind(&srv.user_id)
        .bind(browser)
        .bind(anon)
        .bind(&srv.apple_music_storefront)
        .bind(&srv.apple_music_language)
        .bind(auth)
        .bind(now)
        .execute(&mut *tx)
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
    let mut blob = serde_json::to_value(cfg)?;
    if let Some(obj) = blob.as_object_mut() {
        obj.remove("server");
        obj.remove("servers");
        obj.remove("listen_counts");
        // The resolved/generated active id is authoritative — persist it as the
        // typed `active_source` (`{"Server": id}` or `"Local"`).
        obj.insert(
            "active_source".into(),
            match &active_id {
                Some(id) => serde_json::json!({ "Server": id }),
                None => serde_json::json!("Local"),
            },
        );
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
    Ok(())
}

/// Hydrate one server row (creds included) — the server-switch path, so stored
/// creds are reused instead of re-prompting sign-in.
pub async fn load_server(pool: &SqlitePool, id: &str) -> Result<Option<MusicServer>, DbError> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT id, name, url, service, access_token, user_id, yt_browser, yt_anonymous, \
         apple_music_storefront, apple_music_language \
         FROM servers WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| MusicServer {
        name: r.get("name"),
        url: r.get("url"),
        service: parse_service(r.get::<String, _>("service").as_str()),
        access_token: r.get("access_token"),
        user_id: r.get("user_id"),
        id: Some(r.get("id")),
        yt_browser: r
            .get::<Option<String>, _>("yt_browser")
            .as_deref()
            .and_then(|s| parse_browser(Some(s))),
        yt_anonymous: r.get::<i64, _>("yt_anonymous") != 0,
        apple_music_storefront: r.get("apple_music_storefront"),
        apple_music_language: r.get("apple_music_language"),
    }))
}

/// Increment one track's play count (1-row upsert — no whole-blob rewrite).
pub async fn bump_listen_count(pool: &SqlitePool, key: &str) -> Result<(), DbError> {
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
    let next: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(played_at), 0) + 1 FROM recently_played WHERE source = ?1",
    )
    .bind(src)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO recently_played (source, track_key, played_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(source, track_key) DO UPDATE SET played_at = ?3",
    )
    .bind(src)
    .bind(key)
    .bind(next)
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
    }
}

fn parse_browser(s: Option<&str>) -> Option<Browser> {
    match s {
        Some("chrome") => Some(Browser::Chrome),
        Some("chromium") => Some(Browser::Chromium),
        Some("brave") => Some(Browser::Brave),
        Some("edge") => Some(Browser::Edge),
        Some("vivaldi") => Some(Browser::Vivaldi),
        _ => None,
    }
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
