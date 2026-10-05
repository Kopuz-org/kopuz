//! Per-frontend preferences: UI state a frontend keeps in the daemon instead of its own files.
//!
//! The daemon stores each value as the opaque string the frontend gave it and
//! never parses it; what a key means, and what format its value has, is the
//! frontend's own business.

/// Longest `frontend` name, in bytes.
pub const MAX_PREF_FRONTEND_BYTES: usize = 64;
/// Longest key, in bytes.
pub const MAX_PREF_KEY_BYTES: usize = 128;
/// Longest value, in bytes.
pub const MAX_PREF_VALUE_BYTES: usize = 64 * 1024;
/// Most entries one `set_frontend_prefs` call may carry.
pub const MAX_PREF_ENTRIES_PER_CALL: usize = 256;

/// One stored preference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrontendPref {
    pub key: String,
    pub value: String,
}

/// One change to a frontend's preferences: `Some` stores the value, `None` deletes the key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefEntry {
    pub key: String,
    pub value: Option<String>,
}
