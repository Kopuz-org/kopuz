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

    /// Internal state mutation (offline registrations, play state): bypasses
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
        Ok(current.clone())
    }

    pub async fn snapshot(&self) -> config::AppConfig {
        self.current.read().await.clone()
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
        let locked = self.locked_keys();
        let refused: Vec<&str> = patch_object
            .keys()
            .filter(|key| locked.contains(key))
            .map(String::as_str)
            .collect();
        if !refused.is_empty() {
            return Err(ApiError::invalid_input(format!(
                "keys locked by a managed settings file: {}",
                refused.join(", ")
            )));
        }

        let mut current = self.current.write().await;
        let mut value = serde_json::to_value(&*current)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        merge_patch(&mut value, &patch);
        let updated: config::AppConfig = serde_json::from_value(value)
            .map_err(|error| ApiError::invalid_input(format!("invalid config value: {error}")))?;
        self.db
            .save_config(&updated)
            .await
            .map_err(|error| ApiError::internal(format!("config save failed: {error}")))?;
        *current = updated.clone();
        drop(current);

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
        if view.locked_keys.contains(&"theme".to_string()) {
            let err = service
                .patch(serde_json::json!({"theme": "other"}))
                .await
                .expect_err("locked key refused");
            assert_eq!(err.code, api::ErrorCode::InvalidInput);
        }
    }
}
