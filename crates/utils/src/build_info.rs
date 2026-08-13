//! Compile-time provenance shared by diagnostics and the settings UI.

/// Semantic version of the Kopuz workspace used for this build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Git revision embedded by `build.rs` or explicitly supplied by the packager.
pub const COMMIT: &str = env!("KOPUZ_GIT_COMMIT");

/// Text copied from the settings UI and included in diagnostics.
pub fn summary() -> String {
    format!("Kopuz {VERSION}\nCommit: {COMMIT}")
}
