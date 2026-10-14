//! ConfigService: authority over the running `Settings`, loaded from and saved to the settings files; it keeps daemon-owned and hjem-locked keys, applies RFC 7396 patches, and saves each change at once rather than debounced.

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

/// The running settings and the revision of their last saved change.
struct Held {
    settings: config::Settings,
    revision: u64,
}

/// The settings every layer at `settings_path` adds up to.
pub fn load_settings(settings_path: &std::path::Path) -> Result<config::Settings, ApiError> {
    let layers = config::store::FileLayers::read(settings_path);
    layers
        .settings()
        .map_err(|error| ApiError::internal(format!("settings load failed: {error}")))
}

impl ConfigService {
    pub fn new(db: db::Db, settings_path: PathBuf, current: config::Settings) -> Self {
        // Seeded from the clock so a restarted daemon never hands out a revision a client already passed.
        let revision = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or_default();
        Self {
            db,
            settings_path,
            current: RwLock::new(Held {
                settings: current,
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
        session.set_settings(held.settings.clone(), changed);
    }

    #[tracing::instrument(name = "config.save", skip_all)]
    async fn save(&self, settings: &config::Settings) -> Result<(), ApiError> {
        let failed = |error: String| ApiError::internal(format!("config save failed: {error}"));
        self.db
            .ensure_app_state()
            .await
            .map_err(|error| failed(error.to_string()))?;
        config::store::save_settings(&self.settings_path, settings)
            .map_err(|error| failed(format!("{}: {error}", self.settings_path.display())))?;
        Ok(())
    }

    fn locked_keys(&self) -> Vec<String> {
        config::store::FileLayers::read(&self.settings_path)
            .locked_keys
            .into_iter()
            .collect()
    }

    /// Refuse a write that a managed settings file has pinned, naming the keys the caller is about to change.
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

    /// Change settings from inside the daemon, daemon-owned fields included, persist them, pass them to the session under `keys`, and hand back the result.
    pub async fn mutate_state(
        &self,
        keys: &[&str],
        mutate: impl FnOnce(&mut config::Settings) + Send,
    ) -> Result<config::Settings, ApiError> {
        let mut held = self.current.write().await;
        let mut next = held.settings.clone();
        mutate(&mut next);
        self.save(&next).await?;
        held.settings = next;
        self.publish(&mut held, keys.iter().map(|key| key.to_string()).collect());
        Ok(held.settings.clone())
    }

    pub async fn snapshot(&self) -> config::Settings {
        self.current.read().await.settings.clone()
    }

    pub async fn view(&self) -> Result<ConfigView, ApiError> {
        let held = self.current.read().await;
        Ok(self.view_of(&held))
    }

    fn view_of(&self, held: &Held) -> ConfigView {
        ConfigView {
            settings: stripped(&held.settings),
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
        incoming: config::Settings,
    ) -> Result<(ConfigView, config::Settings, Vec<String>), ApiError> {
        let mut held = self.current.write().await;
        let updated = with_daemon_owned_fields(incoming, &held.settings);

        let changed = changed_keys(&held.settings, &updated)?;
        if changed.is_empty() {
            return Ok((self.view_of(&held), held.settings.clone(), Vec::new()));
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
        held.settings = updated.clone();
        self.publish(&mut held, changed.clone());
        Ok((self.view_of(&held), updated, changed))
    }
}

/// Keep the settings the daemon publishes as field lists of its own, so a stale whole-settings write cannot roll them back.
fn with_daemon_owned_fields(
    mut incoming: config::Settings,
    current: &config::Settings,
) -> config::Settings {
    incoming.library.auto_fetch_covers = current.library.auto_fetch_covers;
    incoming.library.cover_fetch_strategy = current.library.cover_fetch_strategy;
    incoming.library.local_sources = current.library.local_sources.clone();
    incoming.spotify = current.spotify.clone();
    incoming.integrations.discord_presence = current.integrations.discord_presence;
    incoming.integrations.discord_presence_source = current.integrations.discord_presence_source;
    incoming.downloads = current.downloads.clone();
    incoming
}

/// The daemon-owned fields left at their defaults for a caller, except the ones it reads truthfully though it cannot write them.
fn stripped(settings: &config::Settings) -> config::Settings {
    let mut view = with_daemon_owned_fields(settings.clone(), &config::Settings::default());
    view.library.auto_fetch_covers = settings.library.auto_fetch_covers;
    view.library.cover_fetch_strategy = settings.library.cover_fetch_strategy;
    view.library.local_sources = settings.library.local_sources.clone();
    view
}

/// Which top-level keys differ, compared through serialization so a newly added field never needs a hand-written comparison.
fn changed_keys(
    current: &config::Settings,
    updated: &config::Settings,
) -> Result<Vec<String>, ApiError> {
    let to_map = |settings: &config::Settings| -> Result<serde_json::Map<_, _>, ApiError> {
        match serde_json::to_value(settings) {
            Ok(serde_json::Value::Object(map)) => Ok(map),
            Ok(_) => Err(ApiError::internal("settings are not a JSON object")),
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
        let mut current = config::Settings::default();
        current
            .library
            .local_sources
            .push(config::SavedLocalSource {
                id: "local:a".into(),
                name: "a".into(),
                directories: vec!["/music".into()],
            });
        let updated = with_daemon_owned_fields(config::Settings::default(), &current);
        assert_eq!(updated.library.local_sources, current.library.local_sources);
        let view = stripped(&current);
        assert_eq!(view.library.local_sources, current.library.local_sources);
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
            config::Settings::default(),
        );

        let view = service.view().await.expect("view");
        let rendered = serde_json::to_string(&view.settings).expect("json");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(view.locked_keys.is_empty());

        let mut next = view.settings.clone();
        next.background.cover_art_darkening = 32;
        next.playback.crossfade_seconds = 4;
        let (view, updated, changed) = service.set(next).await.expect("set");
        assert_eq!(view.settings.background.cover_art_darkening, 32);
        assert_eq!(updated.playback.crossfade_seconds, 4);
        assert_eq!(changed.len(), 2, "only the two edited keys are reported");

        let reloaded = load_settings(&dir.path().join("settings.toml")).expect("reload");
        assert_eq!(reloaded.playback.crossfade_seconds, 4);
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
            config::Settings::default(),
        );

        let view = service.view().await.expect("view");
        let (_, _, changed) = service.set(view.settings).await.expect("idempotent set");
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

        let service = ConfigService::new(database, settings, config::Settings::default());
        let view = service.view().await.expect("view");
        assert!(view.locked_keys.contains(&"theme".to_string()));
        let mut changed_locked = view.settings.clone();
        changed_locked.ui.theme = "other".to_string();
        let err = service
            .set(changed_locked)
            .await
            .expect_err("changing a locked key is refused");
        assert_eq!(err.code, api::ErrorCode::InvalidInput);

        // Leaving it alone is fine, so a read-modify-write of any other key
        // still works while a managed layer pins this one.
        let mut other = view.settings.clone();
        other.playback.crossfade_seconds = 6;
        let (_, updated, changed) = service.set(other).await.expect("untouched locked key");
        assert_eq!(updated.playback.crossfade_seconds, 6);
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
            config::Settings::default(),
        );
        database
            .set_playback_state(0.2, true)
            .await
            .expect("playback state");
        let mut snapshot = service.view().await.expect("view").settings;
        snapshot.ui.theme = "nord".to_string();
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
        let mut seeded = config::Settings::default();
        seeded.library.auto_fetch_covers = true;
        let service =
            ConfigService::new(database.clone(), dir.path().join("settings.toml"), seeded);
        let mut snapshot = service.view().await.expect("view").settings;
        assert!(
            snapshot.library.auto_fetch_covers,
            "the view reports the live value"
        );

        service
            .mutate_state(&["auto_fetch_covers", "cover_fetch_strategy"], |settings| {
                settings.library.auto_fetch_covers = false;
                settings.library.cover_fetch_strategy = config::FetchStrategy::LastFmOnly;
            })
            .await
            .expect("cover settings");

        snapshot.ui.theme = "nord".to_string();
        let (view, updated, changed) = service.set(snapshot).await.expect("set");

        assert!(
            !view.settings.library.auto_fetch_covers,
            "automatic covers stay off"
        );
        assert_eq!(
            updated.library.cover_fetch_strategy,
            config::FetchStrategy::LastFmOnly
        );
        assert_eq!(changed, vec!["theme".to_string()]);
    }
}
