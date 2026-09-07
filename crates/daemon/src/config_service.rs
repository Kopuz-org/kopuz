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

use api::{ApiError, ConfigView};
use tokio::sync::RwLock;

/// Never serialized to a client and never patchable: credentials move through
/// the dedicated provisioning endpoints, and `offline_tracks` is
/// machine-local path state that reaches clients as per-track flags instead.
pub struct ConfigService {
    db: db::Db,
    settings_path: PathBuf,
    current: RwLock<config::AppConfig>,
}

impl ConfigService {
    pub fn new(db: db::Db, settings_path: PathBuf, current: config::AppConfig) -> Self {
        Self {
            db,
            settings_path,
            current: RwLock::new(current),
        }
    }

    fn locked_keys(&self) -> Vec<String> {
        config::store::FileLayers::read(&self.settings_path)
            .locked_keys
            .into_iter()
            .collect()
    }

    /// Persist one offline-track registration without rewriting the whole
    /// config, then update the in-memory snapshot used by daemon services.
    pub async fn set_offline_track(
        &self,
        item_id: &str,
        path: Option<String>,
    ) -> Result<config::AppConfig, ApiError> {
        let mut current = self.current.write().await;
        self.db
            .set_offline_track(item_id, path.as_deref())
            .await
            .map_err(|error| {
                ApiError::internal(format!("offline track registration failed: {error}"))
            })?;
        match path {
            Some(path) => {
                current.offline_tracks.insert(item_id.to_string(), path);
            }
            None => {
                current.offline_tracks.remove(item_id);
            }
        }
        Ok(current.clone())
    }

    pub async fn snapshot(&self) -> config::AppConfig {
        self.current.read().await.clone()
    }

    pub async fn view(&self) -> Result<ConfigView, ApiError> {
        Ok(ConfigView {
            config: stripped(&self.current.read().await.clone()),
            locked_keys: self.locked_keys(),
        })
    }

    /// Replace the settings surface. The incoming config is whole, so the
    /// keys the daemon never sends come back as defaults -- those are
    /// restored from what is held rather than trusted from the caller, and
    /// a locked key is refused only when the value actually differs, so a
    /// read-modify-write that leaves it alone still succeeds.
    ///
    /// Returns the new view plus the changed top-level keys, which the
    /// caller forwards to the session for `config.changed` and any live
    /// audio-setting updates.
    pub async fn set(
        &self,
        incoming: config::AppConfig,
    ) -> Result<(ConfigView, config::AppConfig, Vec<String>), ApiError> {
        let mut current = self.current.write().await;
        let updated = with_preserved_secrets(incoming, &current);

        let changed = changed_keys(&current, &updated)?;
        if changed.is_empty() {
            let view = ConfigView {
                config: stripped(&current),
                locked_keys: self.locked_keys(),
            };
            return Ok((view, current.clone(), Vec::new()));
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

        self.db
            .save_config(&updated)
            .await
            .map_err(|error| ApiError::internal(format!("config save failed: {error}")))?;
        *current = updated.clone();
        drop(current);

        let view = self.view().await?;
        Ok((view, updated, changed))
    }
}

/// The keys the daemon owns: credentials, and path state that only means
/// something on this machine. They are absent from the wire, so a caller
/// cannot set them and does not have to know them to write anything else.
fn with_preserved_secrets(
    mut incoming: config::AppConfig,
    current: &config::AppConfig,
) -> config::AppConfig {
    incoming.server = current.server.clone();
    incoming.servers = current.servers.clone();
    incoming.musicbrainz_token = current.musicbrainz_token.clone();
    incoming.lastfm_api_key = current.lastfm_api_key.clone();
    incoming.lastfm_api_secret = current.lastfm_api_secret.clone();
    incoming.lastfm_session_key = current.lastfm_session_key.clone();
    incoming.librefm_api_key = current.librefm_api_key.clone();
    incoming.librefm_api_secret = current.librefm_api_secret.clone();
    incoming.librefm_session_key = current.librefm_session_key.clone();
    incoming.offline_tracks = current.offline_tracks.clone();
    incoming
}

/// The same fields, blanked for a caller. Not a security boundary on a
/// socket only this user can open -- it keeps credentials out of a surface
/// that is written back wholesale, so a frontend cannot round-trip a stale
/// copy over them.
fn stripped(config: &config::AppConfig) -> config::AppConfig {
    with_preserved_secrets(config.clone(), &config::AppConfig::default())
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
        next.volume = 0.25;
        next.crossfade_seconds = 4;
        let (view, updated, changed) = service.set(next).await.expect("set");
        assert_eq!(view.config.volume, 0.25);
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
}
