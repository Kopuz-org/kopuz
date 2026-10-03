//! ConfigService: authority over the running `AppConfig`.
//!
//! Reads arrive fully layered from the database (blob, `settings.toml`,
//! `settings.d` drop-ins, env); writes go back through `db::save_config`,
//! which owns the blob/settings-file split. This service adds the wire
//! contract on top: credential stripping, hjem-locked keys, RFC 7396 merge
//! patches, and pushing accepted changes into the player session.
//!
//! Patches persist immediately rather than debounced: API clients act on
//! explicit user intent (a settings form submit), not per-keystroke signal
//! churn, and an immediate write keeps the daemon free of idle timers.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use api::{ApiError, ConfigView};
use tokio::sync::{RwLock, RwLockWriteGuard};

use crate::session::SessionHandle;

/// Keys whose change means the active media source has to be rebuilt.
const SOURCE_KEYS: &[&str] = &[
    "active_source",
    "server",
    "servers",
    "local_sources",
    "server_folders",
];

/// Never serialized to a client and never patchable: credentials move through
/// the dedicated provisioning endpoints, and `offline_tracks` is
/// machine-local path state that reaches clients as per-track flags instead.
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
        if changed
            .iter()
            .any(|key| SOURCE_KEYS.contains(&key.as_str()))
        {
            session.set_active_source(Some(Arc::from(server::source::active(
                self.db.clone(),
                &held.config,
            ))));
        }
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

    /// Change config from inside the daemon, persist it, pass it to the session
    /// under `keys`, and hand back the result.
    ///
    /// Unlike [`Self::set`] this touches credential fields, which is the
    /// point: signing in writes a token no caller ever sent us.
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

    /// Persist one offline-track registration without rewriting the whole
    /// config, then update the in-memory snapshot used by daemon services.
    pub async fn set_offline_track(
        &self,
        item_id: &str,
        path: Option<String>,
    ) -> Result<config::AppConfig, ApiError> {
        let mut held = self.current.write().await;
        self.db
            .set_offline_track(item_id, path.as_deref())
            .await
            .map_err(|error| {
                ApiError::internal(format!("offline track registration failed: {error}"))
            })?;
        match path {
            Some(path) => {
                held.config.offline_tracks.insert(item_id.to_string(), path);
            }
            None => {
                held.config.offline_tracks.remove(item_id);
            }
        }
        self.publish(&mut held, vec!["offline_tracks".to_string()]);
        Ok(held.config.clone())
    }

    /// Pin a station's manifest after the other pins (`Some`), or unpin it (`None`), without a whole-config save.
    pub async fn set_pinned_station(
        &self,
        id: &str,
        manifest: Option<String>,
    ) -> Result<config::AppConfig, ApiError> {
        let mut held = self.current.write().await;
        self.db
            .set_pinned_station(id, manifest.as_deref())
            .await
            .map_err(|error| ApiError::internal(format!("station pin failed: {error}")))?;
        // Mirrors the row write: a re-pin keeps its place, a new pin goes last.
        let pins = &mut held.config.pinned_stations;
        let at = pins
            .iter()
            .position(|pinned| manifest_id(pinned).as_deref() == Some(id));
        match (manifest, at) {
            (Some(manifest), Some(at)) => pins[at] = manifest,
            (Some(manifest), None) => pins.push(manifest),
            (None, Some(at)) => {
                pins.remove(at);
            }
            (None, None) => {}
        }
        self.publish(&mut held, vec!["pinned_stations".to_string()]);
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

    /// Persist the engine's own volume. It is not a caller-set key -- the
    /// session owns it and every frontend just reports what the user did --
    /// so it skips the locked-key and secret machinery of [`Self::set`].
    pub async fn set_volume(&self, volume: f32) -> Result<(), ApiError> {
        let mut held = self.current.write().await;
        let volume = volume.clamp(0.0, 1.0);
        if (held.config.volume - volume).abs() < f32::EPSILON {
            return Ok(());
        }
        let mut next = held.config.clone();
        next.volume = volume;
        // Saved under the guard: a snapshot saved after it drops could undo a server added meanwhile.
        self.save(&next).await?;
        held.config = next;
        held.revision += 1;
        Ok(())
    }
}

/// The keys the daemon owns: credentials, and path state that only means
/// something on this machine. They are absent from the wire, so a caller
/// cannot set them and does not have to know them to write anything else.
/// Keep what the daemon owns: the credentials, and the settings it publishes
/// as field lists of its own. Both are absent from the surface a caller reads,
/// so a caller writing that surface back must not be able to blank them.
///
/// `volume` is here for the same reason even though it is on the wire: the
/// session owns it and [`Self::set_volume`] is how it moves, so a whole-config
/// write carrying a frontend's older copy must not roll the engine back. The
/// two cover-lookup keys follow the same rule: `ArtworkApi` publishes them as
/// a field list and writes them through `mutate_state`, so a frontend that
/// changed one never saw it in the settings surface it holds.
fn with_daemon_owned_fields(
    mut incoming: config::AppConfig,
    current: &config::AppConfig,
) -> config::AppConfig {
    incoming.volume = current.volume;
    incoming.auto_fetch_covers = current.auto_fetch_covers;
    incoming.cover_fetch_strategy = current.cover_fetch_strategy;
    incoming.server = current.server.clone();
    incoming.servers = current.servers.clone();
    incoming.active_source = current.active_source.clone();
    incoming.local_sources = current.local_sources.clone();
    incoming.server_folders = current.server_folders.clone();
    incoming.musicbrainz_token = current.musicbrainz_token.clone();
    incoming.lastfm_api_key = current.lastfm_api_key.clone();
    incoming.lastfm_api_secret = current.lastfm_api_secret.clone();
    incoming.lastfm_session_key = current.lastfm_session_key.clone();
    incoming.librefm_api_key = current.librefm_api_key.clone();
    incoming.librefm_api_secret = current.librefm_api_secret.clone();
    incoming.librefm_session_key = current.librefm_session_key.clone();
    incoming.offline_tracks = current.offline_tracks.clone();
    incoming.pinned_stations = current.pinned_stations.clone();
    incoming.spotify_browser = current.spotify_browser.clone();
    incoming.spotify_prefer_active_device = current.spotify_prefer_active_device;
    incoming.discord_presence = current.discord_presence;
    incoming.discord_presence_paused = current.discord_presence_paused;
    incoming.discord_presence_source = current.discord_presence_source;
    incoming.downloader_output_dir = current.downloader_output_dir.clone();
    incoming.downloader_options = current.downloader_options.clone();
    incoming.downloader_history = current.downloader_history.clone();
    incoming
}

/// The id a pinned manifest names; the registry's document is otherwise opaque here.
fn manifest_id(manifest: &str) -> Option<String> {
    let manifest: serde_json::Value = serde_json::from_str(manifest).ok()?;
    manifest.get("id")?.as_str().map(str::to_string)
}

/// The credentials blanked for a caller. Not a security boundary on a socket
/// only this user can open -- it keeps secrets out of a surface that is
/// written back wholesale, so a frontend cannot round-trip a stale copy over
/// them.
///
/// `offline_tracks` is deliberately not blanked. It is not a secret: it is
/// which tracks have a local copy, which is exactly what a download indicator
/// renders. Blanking it made every one of those read empty.
///
/// `volume` and the cover-lookup keys are restored for the same reason: they
/// are daemon-owned on the way in, so the helper above would hand back the
/// default here. A frontend reads this to place its volume slider at startup,
/// and a read that lied about the others would be a worse surface than one
/// that simply refuses to take them.
fn stripped(config: &config::AppConfig) -> config::AppConfig {
    let mut view = with_daemon_owned_fields(config.clone(), &config::AppConfig::default());
    view.offline_tracks = config.offline_tracks.clone();
    view.volume = config.volume;
    view.auto_fetch_covers = config.auto_fetch_covers;
    view.cover_fetch_strategy = config.cover_fetch_strategy;
    view.active_source = config.active_source.clone();
    view.local_sources = config.local_sources.clone();
    view.server_folders = config.server_folders.clone();
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
        current.active_source = config::Source::LocalLibrary("local:a".into());
        let updated = with_daemon_owned_fields(config::AppConfig::default(), &current);
        assert_eq!(updated.local_sources, current.local_sources);
        assert_eq!(updated.active_source, current.active_source);
        let view = stripped(&current);
        assert_eq!(view.local_sources, current.local_sources);
    }

    #[tokio::test]
    async fn set_round_trips_and_keeps_credentials_the_caller_never_saw() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let seeded = config::AppConfig {
            lastfm_session_key: "secret".into(),
            ..Default::default()
        };
        let service =
            ConfigService::new(database.clone(), dir.path().join("settings.toml"), seeded);

        let view = service.view().await.expect("view");
        assert!(view.config.lastfm_session_key.is_empty());
        assert!(view.config.server.is_none());
        assert!(view.locked_keys.is_empty());

        let mut next = view.config.clone();
        next.hero_height = 320;
        next.crossfade_seconds = 4;
        let (view, updated, changed) = service.set(next).await.expect("set");
        assert_eq!(view.config.hero_height, 320);
        assert_eq!(updated.crossfade_seconds, 4);
        assert_eq!(changed.len(), 2, "only the two edited keys are reported");
        assert_eq!(
            updated.lastfm_session_key, "secret",
            "writing back a blanked view must not erase the credential"
        );

        let reloaded = database.load_config().await.expect("reload").expect("some");
        assert_eq!(reloaded.crossfade_seconds, 4);
        assert_eq!(reloaded.lastfm_session_key, "secret");
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

    #[tokio::test]
    async fn set_volume_persists_without_touching_the_rest_of_the_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("vol.db");
        let database = db::init(&path).await.expect("db");
        let seeded = config::AppConfig {
            lastfm_session_key: "secret".into(),
            crossfade_seconds: 7,
            ..Default::default()
        };
        let service = ConfigService::new(
            database.clone(),
            dir.path().join("settings.toml"),
            seeded.clone(),
        );

        service.set_volume(0.42).await.expect("set volume");

        let stored = database
            .load_config()
            .await
            .expect("load")
            .expect("stored config");
        assert_eq!(stored.volume, 0.42);
        assert_eq!(stored.crossfade_seconds, 7);
        assert_eq!(stored.lastfm_session_key, "secret");
        assert_eq!(service.view().await.expect("view").config.volume, 0.42);
    }

    /// A frontend reads the whole settings surface once and writes it back on
    /// every change, while volume moves separately through `set_volume`. Its
    /// copy is therefore stale by design, and must not roll the engine back.
    #[tokio::test]
    async fn a_whole_config_write_cannot_roll_the_volume_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("vol-stale.db"))
            .await
            .expect("db");
        let seeded = config::AppConfig {
            volume: 0.8,
            ..Default::default()
        };
        let service =
            ConfigService::new(database.clone(), dir.path().join("settings.toml"), seeded);
        let snapshot = service.view().await.expect("view").config;

        service.set_volume(0.2).await.expect("set volume");
        let (view, updated, changed) = service.set(snapshot).await.expect("set");

        assert_eq!(view.config.volume, 0.2, "the view reports the live volume");
        assert_eq!(updated.volume, 0.2);
        assert!(!changed.iter().any(|key| key == "volume"));
        let stored = database
            .load_config()
            .await
            .expect("load")
            .expect("stored config");
        assert_eq!(stored.volume, 0.2);
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
