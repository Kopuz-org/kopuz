//! What a failed discover read says: the daemon's own sentence, or the one case worth wording ourselves.

use api::{ApiError, ErrorCode};
use components::format::{t, t_with};
use gpui_kit::SharedString;

/// What a catalog failure says; a source nobody signed into is the one case worth wording ourselves.
pub fn catalog_failure(error: &ApiError) -> SharedString {
    if error.code == ErrorCode::SourceAuthExpired {
        return t("source_anon_discover");
    }
    t_with(
        "discover_failed",
        &[("error", store::human(error).to_string().into())],
    )
}

/// What a library read's failure says: the daemon's own sentence, not a Rust enum.
pub fn read_failure(error: &ApiError) -> SharedString {
    t_with(
        "discover_read_failed",
        &[("error", store::human(error).to_string().into())],
    )
}
