//! The scrobbling and metadata accounts' credentials, which only the daemon ever holds.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

const MUSICBRAINZ_TOKEN: &str = "musicbrainz_token";
const LASTFM_API_KEY: &str = "lastfm_api_key";
const LASTFM_API_SECRET: &str = "lastfm_api_secret";
const LASTFM_SESSION_KEY: &str = "lastfm_session_key";
const LIBREFM_SESSION_KEY: &str = "librefm_session_key";

/// Each value is empty while it is unset.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    /// The ListenBrainz user token.
    pub musicbrainz_token: String,
    pub lastfm_api_key: String,
    pub lastfm_api_secret: String,
    pub lastfm_session_key: String,
    pub librefm_session_key: String,
}

impl Credentials {
    fn from_rows(mut rows: HashMap<String, String>) -> Self {
        let mut take = |key: &str| rows.remove(key).unwrap_or_default();
        Self {
            musicbrainz_token: take(MUSICBRAINZ_TOKEN),
            lastfm_api_key: take(LASTFM_API_KEY),
            lastfm_api_secret: take(LASTFM_API_SECRET),
            lastfm_session_key: take(LASTFM_SESSION_KEY),
            librefm_session_key: take(LIBREFM_SESSION_KEY),
        }
    }

    /// Each value under the `integration_credentials` key it is stored by.
    fn rows(&self) -> [(&'static str, &str); 5] {
        [
            (MUSICBRAINZ_TOKEN, &self.musicbrainz_token),
            (LASTFM_API_KEY, &self.lastfm_api_key),
            (LASTFM_API_SECRET, &self.lastfm_api_secret),
            (LASTFM_SESSION_KEY, &self.lastfm_session_key),
            (LIBREFM_SESSION_KEY, &self.librefm_session_key),
        ]
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("Credentials");
        for (key, value) in self.rows() {
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
        for ((key, old), (_, new)) in before.rows().into_iter().zip(after.rows()) {
            if old != new {
                self.db.set_integration_credential(key, new).await?;
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
        let store = CredentialStore::load(&database).await.expect("load");
        assert_eq!(store.snapshot(), Credentials::default());

        store
            .update(|held| {
                held.lastfm_api_key = "key".into();
                held.lastfm_session_key = "session".into();
            })
            .await
            .expect("set");
        store
            .update(|held| held.lastfm_session_key.clear())
            .await
            .expect("clear");

        let reloaded = CredentialStore::load(&database).await.expect("reload");
        assert_eq!(reloaded.snapshot(), store.snapshot());
        assert_eq!(reloaded.snapshot().lastfm_api_key, "key");
        assert!(reloaded.snapshot().lastfm_session_key.is_empty());
        let rows = database.integration_credentials().await.expect("rows");
        assert!(!rows.contains_key("lastfm_session_key"));
        assert_eq!(
            rows.get("librefm_api_key").map(String::as_str),
            Some("dead"),
            "a row nothing reads is left alone"
        );
    }

    #[test]
    fn debug_never_prints_a_value() {
        let credentials = Credentials {
            lastfm_api_secret: "hunter2".into(),
            ..Default::default()
        };
        let rendered = format!("{credentials:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(
            rendered.contains("lastfm_api_secret: \"set\""),
            "{rendered}"
        );
    }
}
