//! ConfigService: authority over the running `AppConfig`.
//!
//! Reads arrive layered from the database and settings files, writes go back through `db::save_config`; this adds daemon-owned keys, hjem-locked keys, RFC 7396 patches and pushing changes into the session.
//!
//! Patches persist immediately rather than debounced: API clients act on
//! explicit user intent (a settings form submit), not per-keystroke signal
//! churn, and an immediate write keeps the daemon free of idle timers.

use std::path::PathBuf;
use std::sync::OnceLock;

use api::{ApiError, ConfigView};
use tokio::sync::{RwLock, RwLockWriteGuard};

use crate::session::SessionHandle;

pub struct ConfigService {
    db: db::Db,
    settings_path: PathBuf,
    current: RwLock<Held>,
    session: OnceLock<SessionHandle>,
}

/// The running config and the revision of its last saved change.
struct Held {
    config: config::AppConfig,
    revision: u64,
}

impl ConfigService {
    pub fn new(db: db::Db, settings_path: PathBuf, current: config::AppConfig) -> Self {
        // Seeded from the clock so a restarted daemon never hands out a revision a client already passed.
        let revision = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or_default();
        Self {
            db,
            settings_path,
            current: RwLock::new(Held {
                config: current,
                revision,
            }),
            session: OnceLock::new(),
        }
    }

    /// Hand every later change to `session`, which then always holds what was saved last.
    pub fn attach_session(&self, session: SessionHandle) {
        let _ = self.session.set(session);
    }

    /// Mark a saved change and pass it on while the write guard is still held, so the session sees changes in the order they were saved.
    fn publish(&self, held: &mut RwLockWriteGuard<'_, Held>, changed: Vec<String>) {
        held.revision += 1;
        let Some(session) = self.session.get() else {
            return;
        };
        session.set_config(held.config.clone(), changed);
    }

    async fn save(&self, config: &config::AppConfig) -> Result<(), ApiError> {
        self.db
            .save_config(config)
            .await
            .map_err(|error| ApiError::internal(format!("config save failed: {error}")))
    }

    fn locked_keys(&self) -> Vec<String> {
        config::store::FileLayers::read(&self.settings_path)
            .locked_keys
            .into_iter()
            .collect()
    }

    /// Refuse a write that a managed settings file has pinned.
    ///
    /// A caller checks the keys it is about to change, so the refusal names
    /// them rather than failing later with a diff nobody asked about.
    pub fn ensure_unlocked(&self, keys: &[&str]) -> Result<(), ApiError> {
        let locked = self.locked_keys();
        let refused: Vec<&str> = keys
            .iter()
            .copied()
            .filter(|key| locked.iter().any(|locked| locked == key))
            .collect();
        if refused.is_empty() {
            Ok(())
        } else {
            Err(ApiError::invalid_input(format!(
                "these settings are pinned by a managed file: {}",
                refused.join(", ")
            )))
        }
    }

    /// Change config from inside the daemon, daemon-owned fields included, persist it, pass it to the session under `keys`, and hand back the result.
    pub async fn mutate_state(
        &self,
        keys: &[&str],
        mutate: impl FnOnce(&mut config::AppConfig) + Send,
    ) -> Result<config::AppConfig, ApiError> {
        let mut held = self.current.write().await;
        let mut next = held.config.clone();
        mutate(&mut next);
        self.save(&next).await?;
        held.config = next;
        self.publish(&mut held, keys.iter().map(|key| key.to_string()).collect());
        Ok(held.config.clone())
    }

    pub async fn snapshot(&self) -> config::AppConfig {
        self.current.read().await.config.clone()
    }

    pub async fn view(&self) -> Result<ConfigView, ApiError> {
        let held = self.current.read().await;
        Ok(self.view_of(&held))
    }

    fn view_of(&self, held: &Held) -> ConfigView {
        ConfigView {
            config: stripped(&held.config),
            locked_keys: self.locked_keys(),
            revision: held.revision,
        }
    }

    /// Replace the settings surface. The incoming config is whole, so the
    /// keys the daemon never sends come back as defaults -- those are
    /// restored from what is held rather than trusted from the caller, and
    /// a locked key is refused only when the value actually differs, so a
    /// read-modify-write that leaves it alone still succeeds.
    ///
    /// Returns the new view plus the changed top-level keys, which have
    /// already reached the session for `config.changed` and any live
    /// audio-setting updates.
    pub async fn set(
        &self,
        incoming: config::AppConfig,
    ) -> Result<(ConfigView, config::AppConfig, Vec<String>), ApiError> {
        let mut held = self.current.write().await;
        let updated = with_daemon_owned_fields(incoming, &held.config);

        let changed = changed_keys(&held.config, &updated)?;
        if changed.is_empty() {
            return Ok((self.view_of(&held), held.config.clone(), Vec::new()));
        }
        let locked = self.locked_keys();
        let refused: Vec<&str> = changed
            .iter()
            .filter(|key| locked.contains(*key))
            .map(String::as_str)
            .collect();
        if !refused.is_empty() {
            return Err(ApiError::invalid_input(format!(
                "keys locked by a managed settings file: {}",
                refused.join(", ")
            )));
        }

        self.save(&updated).await?;
        held.config = updated.clone();
        self.publish(&mut held, changed.clone());
        Ok((self.view_of(&held), updated, changed))
    }
}

/// Keep the settings the daemon publishes as field lists of its own, so a stale whole-config write cannot roll them back.
fn with_daemon_owned_fields(
    mut incoming: config::AppConfig,
    current: &config::AppConfig,
) -> config::AppConfig {
    incoming.auto_fetch_covers = current.auto_fetch_covers;
    incoming.cover_fetch_strategy = current.cover_fetch_strategy;
    incoming.local_sources = current.local_sources.clone();
    incoming.spotify_browser = current.spotify_browser.clone();
    incoming.spotify_prefer_active_device = current.spotify_prefer_active_device;
    incoming.discord_presence = current.discord_presence;
    incoming.discord_presence_source = current.discord_presence_source;
    incoming.downloader_output_dir = current.downloader_output_dir.clone();
    incoming.downloader_options = current.downloader_options.clone();
    incoming
}

/// The daemon-owned fields left at their defaults for a caller, except the ones it reads truthfully though it cannot write them.
fn stripped(config: &config::AppConfig) -> config::AppConfig {
    let mut view = with_daemon_owned_fields(config.clone(), &config::AppConfig::default());
    view.auto_fetch_covers = config.auto_fetch_covers;
    view.cover_fetch_strategy = config.cover_fetch_strategy;
    view.local_sources = config.local_sources.clone();
    view
}

/// Which top-level keys differ. Serialization is an implementation detail
/// here -- it never reaches the wire -- and it keeps this from being 78
/// hand-written comparisons that drift the moment a field is added.
fn changed_keys(
    current: &config::AppConfig,
    updated: &config::AppConfig,
) -> Result<Vec<String>, ApiError> {
    let to_map = |config: &config::AppConfig| -> Result<serde_json::Map<_, _>, ApiError> {
        match serde_json::to_value(config) {
            Ok(serde_json::Value::Object(map)) => Ok(map),
            Ok(_) => Err(ApiError::internal("config is not a JSON object")),
            Err(error) => Err(ApiError::internal(error.to_string())),
        }
    };
    let (before, after) = (to_map(current)?, to_map(updated)?);
    Ok(after
        .into_iter()
        .filter(|(key, value)| before.get(key) != Some(value))
        .map(|(key, _)| key)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_whole_config_write_cannot_revert_folder_sources() {
        let mut current = config::AppConfig::default();
        current.local_sources.push(config::SavedLocalSource {
            id: "local:a".into(),
            name: "a".into(),
            directories: vec!["/music".into()],
        });
        let updated = with_daemon_owned_fields(config::AppConfig::default(), &current);
        assert_eq!(updated.local_sources, current.local_sources);
        let view = stripped(&current);
        assert_eq!(view.local_sources, current.local_sources);
    }

    #[tokio::test]
    async fn set_round_trips_and_keeps_credentials_the_caller_never_saw() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        database
            .set_integration_credential("lastfm_session_key", "secret")
            .await
            .expect("seed credential");
        let service = ConfigService::new(
            database.clone(),
            dir.path().join("settings.toml"),
            config::AppConfig::default(),
        );

        let view = service.view().await.expect("view");
        let rendered = serde_json::to_string(&view.config).expect("json");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(view.locked_keys.is_empty());

        let mut next = view.config.clone();
        next.cover_art_darkening = 32;
        next.crossfade_seconds = 4;
        let (view, updated, changed) = service.set(next).await.expect("set");
        assert_eq!(view.config.cover_art_darkening, 32);
        assert_eq!(updated.crossfade_seconds, 4);
        assert_eq!(changed.len(), 2, "only the two edited keys are reported");

        let reloaded = database.load_config().await.expect("reload").expect("some");
        assert_eq!(reloaded.crossfade_seconds, 4);
        let credentials = database
            .integration_credentials()
            .await
            .expect("credentials");
        assert_eq!(
            credentials.get("lastfm_session_key").map(String::as_str),
            Some("secret"),
            "writing back the view must not erase the credential"
        );
    }

    /// A caller that sends back exactly what it read changes nothing, so it
    /// must not be refused even when a managed layer pins a key.
    #[tokio::test]
    async fn an_unchanged_write_reports_no_changed_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let service = ConfigService::new(
            database,
            dir.path().join("settings.toml"),
            config::AppConfig::default(),
        );

        let view = service.view().await.expect("view");
        let (_, _, changed) = service.set(view.config).await.expect("idempotent set");
        assert!(changed.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn locked_keys_are_reported_and_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let settings = dir.path().join("settings.toml");
        std::fs::write(&settings, "theme = \"pinned\"\n").expect("write settings");
        std::fs::set_permissions(&settings, {
            use std::os::unix::fs::PermissionsExt;
            std::fs::Permissions::from_mode(0o444)
        })
        .expect("chmod");

        let service = ConfigService::new(database, settings, config::AppConfig::default());
        let view = service.view().await.expect("view");
        assert!(view.locked_keys.contains(&"theme".to_string()));
        let mut changed_locked = view.config.clone();
        changed_locked.theme = "other".to_string();
        let err = service
            .set(changed_locked)
            .await
            .expect_err("changing a locked key is refused");
        assert_eq!(err.code, api::ErrorCode::InvalidInput);

        // Leaving it alone is fine, so a read-modify-write of any other key
        // still works while a managed layer pins this one.
        let mut other = view.config.clone();
        other.crossfade_seconds = 6;
        let (_, updated, changed) = service.set(other).await.expect("untouched locked key");
        assert_eq!(updated.crossfade_seconds, 6);
        assert_eq!(changed, vec!["crossfade_seconds".to_string()]);
    }

    /// A frontend's whole-config write saves the state row too, and must not reset the playback state the session persists beside it.
    #[tokio::test]
    async fn a_whole_config_write_leaves_the_playback_state_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("vol-stale.db"))
            .await
            .expect("db");
        let service = ConfigService::new(
            database.clone(),
            dir.path().join("settings.toml"),
            config::AppConfig::default(),
        );
        database
            .set_playback_state(0.2, true)
            .await
            .expect("playback state");
        let mut snapshot = service.view().await.expect("view").config;
        snapshot.theme = "nord".to_string();
        service.set(snapshot).await.expect("set");

        assert_eq!(database.playback_state().await.expect("load"), (0.2, true));
    }

    /// `ArtworkApi` publishes the cover-lookup keys as a field list of its own
    /// and writes them through `mutate_state`, exactly as this test does, so a
    /// frontend holding the settings surface never sees them move. Saving that
    /// surface for an unrelated reason must not undo them.
    #[tokio::test]
    async fn a_stale_settings_surface_cannot_undo_the_cover_settings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("covers.db")).await.expect("db");
        let seeded = config::AppConfig {
            auto_fetch_covers: true,
            ..Default::default()
        };
        let service =
            ConfigService::new(database.clone(), dir.path().join("settings.toml"), seeded);
        let mut snapshot = service.view().await.expect("view").config;
        assert!(
            snapshot.auto_fetch_covers,
            "the view reports the live value"
        );

        service
            .mutate_state(&["auto_fetch_covers", "cover_fetch_strategy"], |config| {
                config.auto_fetch_covers = false;
                config.cover_fetch_strategy = config::FetchStrategy::LastFmOnly;
            })
            .await
            .expect("cover settings");

        snapshot.theme = "nord".to_string();
        let (view, updated, changed) = service.set(snapshot).await.expect("set");

        assert!(!view.config.auto_fetch_covers, "automatic covers stay off");
        assert_eq!(
            updated.cover_fetch_strategy,
            config::FetchStrategy::LastFmOnly
        );
        assert_eq!(changed, vec!["theme".to_string()]);
    }
}
