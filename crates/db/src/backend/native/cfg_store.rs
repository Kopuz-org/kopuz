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

use config::{AppConfig, Browser, MusicServer, MusicService, SavedServer};
use sqlx::SqlitePool;

use crate::DbError;

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
    let rows = sqlx::query!(
        "SELECT id, name, url, service, access_token, user_id, yt_browser, yt_anonymous \
         FROM servers"
    )
    .fetch_all(pool)
    .await?;

    cfg.servers = rows
        .iter()
        .map(|r| SavedServer {
            id: r.id.clone(),
            name: r.name.clone(),
            url: r.url.clone(),
            service: parse_service(&r.service),
            yt_browser: parse_browser(r.yt_browser.as_deref()),
            yt_anonymous: r.yt_anonymous != 0,
        })
        .collect();

    cfg.server = cfg.active_server_id.as_deref().and_then(|active| {
        rows.iter().find(|r| r.id == active).map(|r| MusicServer {
            name: r.name.clone(),
            url: r.url.clone(),
            service: parse_service(&r.service),
            access_token: r.access_token.clone(),
            user_id: r.user_id.clone(),
            id: Some(r.id.clone()),
            yt_browser: parse_browser(r.yt_browser.as_deref()),
            yt_anonymous: r.yt_anonymous != 0,
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

pub async fn save_config(pool: &SqlitePool, cfg: &AppConfig) -> Result<(), DbError> {
    let now = now_secs();
    let mut tx = pool.begin().await?;

    // Sync the saved-servers list (non-cred fields only — never clobber a stored
    // token from the in-memory cache, which doesn't carry other servers' creds).
    for s in &cfg.servers {
        let service = service_str(s.service);
        let browser = s.yt_browser.map(browser_str);
        let anon = s.yt_anonymous as i64;
        sqlx::query!(
            "INSERT INTO servers (id, name, url, service, yt_browser, yt_anonymous, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(id) DO UPDATE SET name=?2, url=?3, service=?4, yt_browser=?5, \
               yt_anonymous=?6, updated_at=?7",
            s.id,
            s.name,
            s.url,
            service,
            browser,
            anon,
            now
        )
        .execute(&mut *tx)
        .await?;
    }

    // Upsert the active server WITH its creds, and remember its id for the blob.
    let mut active_id: Option<String> = cfg.active_server_id.clone();
    if let Some(srv) = &cfg.server {
        let id = srv
            .id
            .clone()
            .or_else(|| cfg.active_server_id.clone())
            .unwrap_or_else(|| format!("legacy-{}", service_str(srv.service)));
        let service = service_str(srv.service);
        let browser = srv.yt_browser.map(browser_str);
        let anon = srv.yt_anonymous as i64;
        let auth = if srv.access_token.is_some() || srv.yt_anonymous {
            "active"
        } else {
            "unauthenticated"
        };
        sqlx::query!(
            "INSERT INTO servers \
               (id, name, url, service, access_token, user_id, yt_browser, yt_anonymous, auth_state, cred_updated_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
             ON CONFLICT(id) DO UPDATE SET name=?2, url=?3, service=?4, access_token=?5, \
               user_id=?6, yt_browser=?7, yt_anonymous=?8, auth_state=?9, cred_updated_at=?10, updated_at=?10",
            id,
            srv.name,
            srv.url,
            service,
            srv.access_token,
            srv.user_id,
            browser,
            anon,
            auth,
            now
        )
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

    // Sync play counts (counts only grow; no deletes).
    for (key, count) in &cfg.listen_counts {
        let c = *count as i64;
        sqlx::query!(
            "INSERT INTO listen_counts (track_key, count) VALUES (?1, ?2) \
             ON CONFLICT(track_key) DO UPDATE SET count = ?2",
            key,
            c
        )
        .execute(&mut *tx)
        .await?;
    }

    // Store the blob, stripped of creds/servers/counts, stamped with the active id.
    let mut blob = serde_json::to_value(cfg)?;
    if let Some(obj) = blob.as_object_mut() {
        obj.remove("server");
        obj.remove("servers");
        obj.remove("listen_counts");
        obj.insert("active_server_id".into(), serde_json::json!(active_id));
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
    let row = sqlx::query!(
        "SELECT id, name, url, service, access_token, user_id, yt_browser, yt_anonymous \
         FROM servers WHERE id = ?1",
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| MusicServer {
        name: r.name,
        url: r.url,
        service: parse_service(&r.service),
        access_token: r.access_token,
        user_id: r.user_id,
        id: Some(r.id),
        yt_browser: parse_browser(r.yt_browser.as_deref()),
        yt_anonymous: r.yt_anonymous != 0,
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

fn parse_service(s: &str) -> MusicService {
    match s {
        "Subsonic" => MusicService::Subsonic,
        "Custom" => MusicService::Custom,
        "YtMusic" => MusicService::YtMusic,
        _ => MusicService::Jellyfin,
    }
}

fn service_str(s: MusicService) -> &'static str {
    match s {
        MusicService::Jellyfin => "Jellyfin",
        MusicService::Subsonic => "Subsonic",
        MusicService::Custom => "Custom",
        MusicService::YtMusic => "YtMusic",
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
