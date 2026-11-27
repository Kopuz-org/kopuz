//! The scrobbling and metadata accounts' credentials, which only the daemon ever holds.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, PoisonError, RwLock};

const MUSICBRAINZ_TOKEN: &str = "musicbrainz_token";
const LIBREFM_SESSION_KEY: &str = "librefm_session_key";
/// An integration plugin's answers are stored as `plugin/<id>/<field>`.
const PLUGIN_PREFIX: &str = "plugin/";

/// One integration plugin's answers, by field key.
pub type PluginAnswers = BTreeMap<String, String>;

/// Each value is empty while it is unset.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    /// The ListenBrainz user token.
    pub musicbrainz_token: String,
    pub librefm_session_key: String,
    /// Every answer an integration plugin was given, secret or not, by plugin id.
    /// None of it belongs in a settings file, since a secret may sit beside it.
    pub plugins: BTreeMap<String, PluginAnswers>,
}

impl Credentials {
    fn from_rows(mut rows: HashMap<String, String>) -> Self {
        let mut take = |key: &str| rows.remove(key).unwrap_or_default();
        let musicbrainz_token = take(MUSICBRAINZ_TOKEN);
        let librefm_session_key = take(LIBREFM_SESSION_KEY);
        let mut plugins: BTreeMap<String, PluginAnswers> = BTreeMap::new();
        for (key, value) in rows {
            let Some((id, field)) = key
                .strip_prefix(PLUGIN_PREFIX)
                .and_then(|rest| rest.split_once('/'))
            else {
                continue;
            };
            plugins
                .entry(id.to_string())
                .or_default()
                .insert(field.to_string(), value);
        }
        Self {
            musicbrainz_token,
            librefm_session_key,
            plugins,
        }
    }

    /// Each value under the `integration_credentials` key it is stored by.
    fn rows(&self) -> HashMap<String, &str> {
        let mut rows = HashMap::from([
            (
                MUSICBRAINZ_TOKEN.to_string(),
                self.musicbrainz_token.as_str(),
            ),
            (
                LIBREFM_SESSION_KEY.to_string(),
                self.librefm_session_key.as_str(),
            ),
        ]);
        for (id, answers) in &self.plugins {
            for (field, value) in answers {
                rows.insert(format!("{PLUGIN_PREFIX}{id}/{field}"), value.as_str());
            }
        }
        rows
    }

    /// What `id` was answered; empty when it never was.
    pub fn plugin(&self, id: &str) -> PluginAnswers {
        self.plugins.get(id).cloned().unwrap_or_default()
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut rows: Vec<_> = self.rows().into_iter().collect();
        rows.sort();
        let mut debug = f.debug_struct("Credentials");
        for (key, value) in &rows {
            debug.field(key, &if value.is_empty() { "unset" } else { "set" });
        }
        debug.finish()
    }
}

/// A cheap handle on the held credentials; every change is stored before it is seen.
#[derive(Clone)]
pub struct CredentialStore {
    db: db::Db,
    held: Arc<RwLock<Credentials>>,
    writer: Arc<tokio::sync::Mutex<()>>,
}

impl CredentialStore {
    pub async fn load(db: &db::Db) -> Result<Self, db::DbError> {
        let held = Credentials::from_rows(db.integration_credentials().await?);
        Ok(Self {
            db: db.clone(),
            held: Arc::new(RwLock::new(held)),
            writer: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub fn snapshot(&self) -> Credentials {
        self.held
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Change the credentials, store every value that moved, and hand back the result.
    pub async fn update(
        &self,
        change: impl FnOnce(&mut Credentials),
    ) -> Result<Credentials, db::DbError> {
        let _writing = self.writer.lock().await;
        let before = self.snapshot();
        let mut after = before.clone();
        change(&mut after);
        let (old, new) = (before.rows(), after.rows());
        // A row the change took away is stored as empty, which drops it.
        // Each key once: one in both maps would otherwise be written twice.
        let keys: std::collections::BTreeSet<&str> = old
            .keys()
            .chain(new.keys())
            .map(|key| key.as_ref())
            .collect();
        for key in keys {
            let value = new.get(key).copied().unwrap_or_default();
            if old.get(key).copied().unwrap_or_default() != value {
                self.db.set_integration_credential(key, value).await?;
            }
        }
        *self.held.write().unwrap_or_else(PoisonError::into_inner) = after.clone();
        Ok(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_change_is_stored_and_a_cleared_value_is_dropped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("creds.db")).await.expect("db");
        database
            .set_integration_credential("librefm_api_key", "dead")
            .await
            .expect("seed");
        database
            .set_integration_credential("lastfm_session_key", "retired")
            .await
            .expect("seed retired credential");
        let store = CredentialStore::load(&database).await.expect("load");
        assert_eq!(store.snapshot(), Credentials::default());

        store
            .update(|held| {
                held.musicbrainz_token = "key".into();
                held.librefm_session_key = "session".into();
            })
            .await
            .expect("set");
        store
            .update(|held| held.librefm_session_key.clear())
            .await
            .expect("clear");

        let reloaded = CredentialStore::load(&database).await.expect("reload");
        assert_eq!(reloaded.snapshot(), store.snapshot());
        assert_eq!(reloaded.snapshot().musicbrainz_token, "key");
        assert!(reloaded.snapshot().librefm_session_key.is_empty());
        let rows = database.integration_credentials().await.expect("rows");
        assert!(!rows.contains_key("librefm_session_key"));
        assert_eq!(
            rows.get("librefm_api_key").map(String::as_str),
            Some("dead"),
            "a row nothing reads is left alone"
        );
    }

    #[tokio::test]
    async fn a_plugins_answers_are_kept_apart_and_dropped_with_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("creds.db")).await.expect("db");
        let store = CredentialStore::load(&database).await.expect("load");
        store
            .update(|held| {
                held.musicbrainz_token = "key".into();
                let lights = held.plugins.entry("lights".into()).or_default();
                lights.insert("url".into(), "ws://127.0.0.1:12345".into());
                lights.insert("token".into(), "hunter2".into());
                held.plugins
                    .entry("webhook".into())
                    .or_default()
                    .insert("url".into(), "http://hook.test".into());
            })
            .await
            .expect("set");

        let reloaded = CredentialStore::load(&database).await.expect("reload");
        assert_eq!(reloaded.snapshot(), store.snapshot());
        assert_eq!(
            reloaded
                .snapshot()
                .plugin("lights")
                .get("token")
                .map(String::as_str),
            Some("hunter2")
        );

        store
            .update(|held| {
                held.plugins.remove("lights");
            })
            .await
            .expect("clear");
        let rows = database.integration_credentials().await.expect("rows");
        assert!(
            !rows.keys().any(|key| key.starts_with("plugin/lights/")),
            "{rows:?}"
        );
        assert!(rows.contains_key("plugin/webhook/url"));
        assert_eq!(
            rows.get("musicbrainz_token").map(String::as_str),
            Some("key")
        );
    }

    #[test]
    fn debug_never_prints_a_value() {
        let credentials = Credentials {
            musicbrainz_token: "hunter2".into(),
            ..Default::default()
        };
        let rendered = format!("{credentials:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(
            rendered.contains("musicbrainz_token: \"set\""),
            "{rendered}"
        );
    }
}
