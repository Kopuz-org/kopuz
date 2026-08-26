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
use tokio::sync::{RwLock, watch};

/// Never serialized to a client and never patchable: credentials move through
/// the dedicated provisioning endpoints, and `offline_tracks` is
/// machine-local path state that reaches clients as per-track flags instead.
const SENSITIVE_KEYS: &[&str] = &[
    "server",
    "servers",
    "musicbrainz_token",
    "lastfm_api_key",
    "lastfm_api_secret",
    "lastfm_session_key",
    "librefm_api_key",
    "librefm_api_secret",
    "librefm_session_key",
    "offline_tracks",
];

pub struct ConfigService {
    db: db::Db,
    settings_path: PathBuf,
    current: RwLock<config::AppConfig>,
    updates: watch::Sender<config::AppConfig>,
}

/// RFC 7396 merge patch: objects merge recursively, `null` removes, anything
/// else replaces.
fn merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    let serde_json::Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = serde_json::Value::Object(serde_json::Map::new());
    }
    let Some(object) = target.as_object_mut() else {
        return;
    };
    for (key, value) in patch {
        if value.is_null() {
            object.remove(key);
        } else {
            merge_patch(
                object.entry(key.clone()).or_insert(serde_json::Value::Null),
                value,
            );
        }
    }
}

fn strip_sensitive(value: &mut serde_json::Value) {
    if let Some(object) = value.as_object_mut() {
        for key in SENSITIVE_KEYS {
            object.remove(*key);
        }
    }
}

impl ConfigService {
    pub fn new(db: db::Db, settings_path: PathBuf, current: config::AppConfig) -> Self {
        let (updates, _) = watch::channel(current.clone());
        Self {
            db,
            settings_path,
            current: RwLock::new(current),
            updates,
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<config::AppConfig> {
        self.updates.subscribe()
    }

    /// Adopt state persisted by the embedded GUI's existing save path.
    pub async fn adopt(&self, config: config::AppConfig) {
        let mut current = self.current.write().await;
        let unchanged = serde_json::to_value(&*current).ok() == serde_json::to_value(&config).ok();
        if unchanged {
            return;
        }
        *current = config.clone();
        drop(current);
        let _ = self.updates.send(config);
    }

    fn locked_keys(&self) -> Vec<String> {
        config::store::FileLayers::read(&self.settings_path)
            .locked_keys
            .into_iter()
            .collect()
    }

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
                "keys locked by a managed settings file: {}",
                refused.join(", ")
            )))
        }
    }

    /// Internal state mutation: bypasses
    /// the wire-facing sensitive/locked checks, persists immediately, and
    /// returns the updated config for the caller to push into the session.
    pub async fn mutate_state(
        &self,
        mutate: impl FnOnce(&mut config::AppConfig),
    ) -> Result<config::AppConfig, ApiError> {
        let mut current = self.current.write().await;
        mutate(&mut current);
        self.db
            .save_config(&current)
            .await
            .map_err(|error| ApiError::internal(format!("config save failed: {error}")))?;
        let updated = current.clone();
        drop(current);
        let _ = self.updates.send(updated.clone());
        Ok(updated)
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
        let updated = current.clone();
        drop(current);
        let _ = self.updates.send(updated.clone());
        Ok(updated)
    }

    pub async fn snapshot(&self) -> config::AppConfig {
        self.current.read().await.clone()
    }

    pub async fn rotate_active_server_credential(
        &self,
        server_id: &str,
        expected: &str,
        rotated: String,
    ) -> Result<Option<config::AppConfig>, ApiError> {
        let mut current = self.current.write().await;
        let Some(server) = current.server.as_mut() else {
            return Ok(None);
        };
        if server.id.as_deref() != Some(server_id)
            || server.access_token.as_deref() != Some(expected)
        {
            return Ok(None);
        }
        self.db
            .set_server_credentials(server_id, Some(&rotated), server.user_id.as_deref())
            .await
            .map_err(|error| {
                ApiError::internal(format!("server credential save failed: {error}"))
            })?;
        server.access_token = Some(rotated);
        let updated = current.clone();
        drop(current);
        let _ = self.updates.send(updated.clone());
        Ok(Some(updated))
    }

    pub fn playback_source(&self, config: &config::AppConfig) -> server::source::ActiveSource {
        server::source::ActiveSource::from(server::source::active(self.db.clone(), config))
    }

    async fn normalize_active_source(
        &self,
        config: &mut config::AppConfig,
    ) -> Result<(), ApiError> {
        match config.active_source.clone() {
            source @ (config::Source::Local | config::Source::LocalLibrary(_)) => {
                if let config::Source::LocalLibrary(id) = &source
                    && !config.local_sources.iter().any(|saved| saved.id == *id)
                {
                    return Err(ApiError::not_found(format!(
                        "local library source not found: {id}"
                    )));
                }
                config.set_active_local_source(source);
            }
            config::Source::Server(id) => {
                let server = self
                    .db
                    .load_server(&id)
                    .await
                    .map_err(|error| ApiError::internal(format!("server lookup failed: {error}")))?
                    .ok_or_else(|| ApiError::not_found(format!("server not found: {id}")))?;
                config.set_active_server_snapshot(server);
            }
        }
        Ok(())
    }

    pub async fn view(&self) -> Result<ConfigView, ApiError> {
        let mut value = serde_json::to_value(&*self.current.read().await)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        strip_sensitive(&mut value);
        Ok(ConfigView {
            config: value,
            locked_keys: self.locked_keys(),
        })
    }

    /// Apply a merge patch. Returns the new view plus the accepted top-level
    /// keys, which the caller forwards to the session for the
    /// `config.changed` event and any live audio-setting updates.
    pub async fn patch(
        &self,
        patch: serde_json::Value,
    ) -> Result<(ConfigView, config::AppConfig, Vec<String>), ApiError> {
        let Some(patch_object) = patch.as_object() else {
            return Err(ApiError::invalid_input(
                "config patch must be a JSON object",
            ));
        };
        let sensitive: Vec<&str> = patch_object
            .keys()
            .filter(|key| SENSITIVE_KEYS.contains(&key.as_str()))
            .map(String::as_str)
            .collect();
        if !sensitive.is_empty() {
            return Err(ApiError::invalid_input(format!(
                "keys managed outside the config surface: {}",
                sensitive.join(", ")
            )));
        }
        let mut patch_keys: Vec<&str> = patch_object.keys().map(String::as_str).collect();
        if patch_object.contains_key("active_source") {
            patch_keys.push("server");
        }
        self.ensure_unlocked(&patch_keys)?;

        let mut current = self.current.write().await;
        let mut value = serde_json::to_value(&*current)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        merge_patch(&mut value, &patch);
        let mut updated: config::AppConfig = serde_json::from_value(value)
            .map_err(|error| ApiError::invalid_input(format!("invalid config value: {error}")))?;
        if patch_object.contains_key("active_source") {
            self.normalize_active_source(&mut updated).await?;
        }
        self.db
            .save_config(&updated)
            .await
            .map_err(|error| ApiError::internal(format!("config save failed: {error}")))?;
        *current = updated.clone();
        drop(current);
        let _ = self.updates.send(updated.clone());

        let changed: Vec<String> = patch_object.keys().cloned().collect();
        let view = self.view().await?;
        Ok((view, updated, changed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_patch_merges_removes_and_replaces() {
        let mut target = serde_json::json!({
            "volume": 0.5,
            "nested": {"a": 1, "b": 2},
            "gone": true,
        });
        merge_patch(
            &mut target,
            &serde_json::json!({
                "volume": 0.9,
                "nested": {"b": null, "c": 3},
                "gone": null,
            }),
        );
        assert_eq!(
            target,
            serde_json::json!({
                "volume": 0.9,
                "nested": {"a": 1, "c": 3},
            })
        );
    }

    #[tokio::test]
    async fn patch_round_trips_and_strips_credentials() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let seeded = config::AppConfig {
            lastfm_session_key: "secret".into(),
            ..Default::default()
        };
        let service =
            ConfigService::new(database.clone(), dir.path().join("settings.toml"), seeded);

        let view = service.view().await.expect("view");
        assert!(view.config.get("lastfm_session_key").is_none());
        assert!(view.config.get("server").is_none());
        assert!(view.locked_keys.is_empty());

        let (view, updated, changed) = service
            .patch(serde_json::json!({"volume": 0.25, "crossfade_seconds": 4}))
            .await
            .expect("patch");
        assert_eq!(view.config["volume"], 0.25);
        assert_eq!(updated.crossfade_seconds, 4);
        assert_eq!(changed.len(), 2);
        assert_eq!(updated.lastfm_session_key, "secret");

        let reloaded = database.load_config().await.expect("reload").expect("some");
        assert_eq!(reloaded.crossfade_seconds, 4);

        let err = service
            .patch(serde_json::json!({"servers": []}))
            .await
            .expect_err("credential keys refused");
        assert_eq!(err.code, api::ErrorCode::InvalidInput);

        let err = service
            .patch(serde_json::json!("not an object"))
            .await
            .expect_err("non-object refused");
        assert_eq!(err.code, api::ErrorCode::InvalidInput);
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
        let err = service
            .patch(serde_json::json!({"theme": "other"}))
            .await
            .expect_err("locked key refused");
        assert_eq!(err.code, api::ErrorCode::InvalidInput);
    }

    #[tokio::test]
    async fn active_source_patch_hydrates_and_persists_the_selected_server() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let mut server = config::MusicServer::new("Server".into(), "https://media.test".into());
        server.id = Some("server-b".into());
        server.access_token = Some("secret".into());
        let saved = config::SavedServer::from_music_server(&server);
        let mut persisted = config::AppConfig::default();
        persisted.servers.push(saved.clone());
        persisted.set_active_server_snapshot(server.clone());
        database.save_config(&persisted).await.expect("seed server");

        let current = config::AppConfig {
            servers: vec![saved],
            ..Default::default()
        };
        let service =
            ConfigService::new(database.clone(), dir.path().join("settings.toml"), current);
        let (_, updated, _) = service
            .patch(serde_json::json!({
                "active_source": {"Server": "server-b"}
            }))
            .await
            .expect("switch accepted");
        assert_eq!(updated.active_source.server_id(), Some("server-b"));
        assert_eq!(
            updated
                .server
                .as_ref()
                .and_then(|server| server.access_token.as_deref()),
            Some("secret")
        );

        let reloaded = database
            .load_config()
            .await
            .expect("reload")
            .expect("config");
        assert_eq!(reloaded.active_source.server_id(), Some("server-b"));
        assert_eq!(
            reloaded
                .server
                .as_ref()
                .and_then(|server| server.access_token.as_deref()),
            Some("secret")
        );
    }

    #[tokio::test]
    async fn credential_rotation_updates_memory_and_the_server_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("cfg.db")).await.expect("db");
        let mut server = config::MusicServer::new("Spotify".into(), "client-id".into());
        server.id = Some("spotify".into());
        server.service = config::MusicService::Spotify;
        server.access_token = Some("old".into());
        let saved = config::SavedServer::from_music_server(&server);
        let mut current = config::AppConfig {
            servers: vec![saved],
            ..Default::default()
        };
        current.set_active_server_snapshot(server);
        database.save_config(&current).await.expect("seed server");
        let service =
            ConfigService::new(database.clone(), dir.path().join("settings.toml"), current);

        let updated = service
            .rotate_active_server_credential("spotify", "old", "new".into())
            .await
            .expect("rotate")
            .expect("matching active server");
        assert_eq!(
            updated
                .server
                .as_ref()
                .and_then(|server| server.access_token.as_deref()),
            Some("new")
        );
        assert_eq!(
            database
                .load_server("spotify")
                .await
                .expect("load server")
                .and_then(|server| server.access_token),
            Some("new".into())
        );
        assert!(
            service
                .rotate_active_server_credential("spotify", "old", "stale".into())
                .await
                .expect("stale rotation")
                .is_none()
        );
    }
}
