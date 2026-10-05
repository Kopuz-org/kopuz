//! Per-frontend preferences: bounded, namespaced strings the daemon keeps and never interprets.
//!
//! A write is validated whole before anything is stored, and the change event
//! goes out only after the transaction commits, so a client that hears it
//! always reads the new values.

use std::collections::HashSet;
use std::sync::Arc;

use api::{
    ApiError, ApiEvent, FrontendPref, MAX_PREF_ENTRIES_PER_CALL, MAX_PREF_FRONTEND_BYTES,
    MAX_PREF_KEY_BYTES, MAX_PREF_VALUE_BYTES, PrefEntry,
};

use crate::session::SessionHandle;

pub struct PrefsService {
    db: db::Db,
    session: SessionHandle,
}

fn name(what: &str, text: &str, max: usize) -> Result<(), ApiError> {
    if text.is_empty() {
        return Err(ApiError::invalid_input(format!("{what} must not be empty")));
    }
    if text.len() > max {
        return Err(ApiError::invalid_input(format!(
            "{what} is over {max} bytes"
        )));
    }
    if text.chars().any(char::is_control) {
        return Err(ApiError::invalid_input(format!(
            "{what} must not hold control characters"
        )));
    }
    Ok(())
}

fn frontend_name(frontend: &str) -> Result<(), ApiError> {
    name("frontend", frontend, MAX_PREF_FRONTEND_BYTES)
}

fn validate(frontend: &str, entries: &[PrefEntry]) -> Result<(), ApiError> {
    frontend_name(frontend)?;
    if entries.len() > MAX_PREF_ENTRIES_PER_CALL {
        return Err(ApiError::invalid_input(format!(
            "more than {MAX_PREF_ENTRIES_PER_CALL} entries in one call"
        )));
    }
    let mut seen = HashSet::new();
    for entry in entries {
        name("key", &entry.key, MAX_PREF_KEY_BYTES)?;
        if !seen.insert(entry.key.as_str()) {
            return Err(ApiError::invalid_input(format!(
                "key {:?} appears twice",
                entry.key
            )));
        }
        if entry
            .value
            .as_ref()
            .is_some_and(|value| value.len() > MAX_PREF_VALUE_BYTES)
        {
            return Err(ApiError::invalid_input(format!(
                "value of {:?} is over {MAX_PREF_VALUE_BYTES} bytes",
                entry.key
            )));
        }
    }
    Ok(())
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("prefs database error: {error}"))
}

impl PrefsService {
    pub fn new(db: db::Db, session: SessionHandle) -> Arc<Self> {
        Arc::new(Self { db, session })
    }

    pub async fn get(&self, frontend: &str) -> Result<Vec<FrontendPref>, ApiError> {
        frontend_name(frontend)?;
        let rows = self.db.frontend_prefs(frontend).await.map_err(db_error)?;
        Ok(rows
            .into_iter()
            .map(|(key, value)| FrontendPref { key, value })
            .collect())
    }

    pub async fn set(&self, frontend: &str, entries: Vec<PrefEntry>) -> Result<(), ApiError> {
        validate(frontend, &entries)?;
        let rows: Vec<(String, Option<String>)> = entries
            .into_iter()
            .map(|entry| (entry.key, entry.value))
            .collect();
        let keys = self
            .db
            .set_frontend_prefs(frontend, &rows)
            .await
            .map_err(db_error)?;
        if !keys.is_empty() {
            self.session.emit_event(ApiEvent::FrontendPrefsChanged {
                frontend: frontend.to_string(),
                keys,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(key: &str, value: &str) -> PrefEntry {
        PrefEntry {
            key: key.into(),
            value: Some(value.into()),
        }
    }

    #[test]
    fn a_well_formed_write_passes() {
        let delete = PrefEntry {
            key: "gone".into(),
            value: None,
        };
        assert!(validate("gpui", &[put("skin", "dark"), put("empty", ""), delete]).is_ok());
        assert!(validate("gpui", &[]).is_ok());
    }

    #[test]
    fn names_are_required_and_bounded() {
        let long_frontend = "f".repeat(MAX_PREF_FRONTEND_BYTES + 1);
        let long_key = "k".repeat(MAX_PREF_KEY_BYTES + 1);
        let at_limit = "k".repeat(MAX_PREF_KEY_BYTES);
        assert!(validate("", &[]).is_err());
        assert!(validate(&long_frontend, &[]).is_err());
        assert!(validate("a\nb", &[]).is_err());
        assert!(validate("gpui", &[put("", "x")]).is_err());
        assert!(validate("gpui", &[put(&long_key, "x")]).is_err());
        assert!(validate("gpui", &[put("a\u{0}b", "x")]).is_err());
        assert!(validate("gpui", &[put(&at_limit, "x")]).is_ok());
        assert!(frontend_name("").is_err());
    }

    #[test]
    fn a_multibyte_name_counts_bytes() {
        let wide = "\u{3042}".repeat(MAX_PREF_KEY_BYTES / 3 + 1);
        assert!(wide.chars().count() <= MAX_PREF_KEY_BYTES);
        assert!(validate("gpui", &[put(&wide, "x")]).is_err());
    }

    #[test]
    fn a_value_is_capped_but_a_delete_has_none() {
        let big = "v".repeat(MAX_PREF_VALUE_BYTES + 1);
        let fits = "v".repeat(MAX_PREF_VALUE_BYTES);
        assert!(validate("gpui", &[put("k", &big)]).is_err());
        assert!(validate("gpui", &[put("k", &fits)]).is_ok());
    }

    #[test]
    fn a_call_is_capped_and_cannot_name_a_key_twice() {
        let many: Vec<PrefEntry> = (0..=MAX_PREF_ENTRIES_PER_CALL)
            .map(|n| put(&format!("k{n}"), "v"))
            .collect();
        assert!(validate("gpui", &many).is_err());
        assert!(validate("gpui", &many[..MAX_PREF_ENTRIES_PER_CALL]).is_ok());
        assert!(validate("gpui", &[put("k", "1"), put("k", "2")]).is_err());
    }

    #[test]
    fn every_refusal_is_invalid_input() {
        let error = validate("", &[]).expect_err("refused");
        assert_eq!(error.code, api::ErrorCode::InvalidInput);
    }
}
