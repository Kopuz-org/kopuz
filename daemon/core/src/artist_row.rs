//! The one place an artist key becomes a library row or a not-found.

use api::ApiError;

/// The row `key` names in `source`, or `None` when the library files no such artist.
pub(crate) async fn find(
    db: &db::Db,
    source: &config::Source,
    key: &str,
) -> Result<Option<db::ArtistRow>, ApiError> {
    db.artist(source, key)
        .await
        .map_err(|error| ApiError::internal(format!("database error: {error}")))
}

/// The row `key` names in `source`, or not-found.
pub(crate) async fn require(
    db: &db::Db,
    source: &config::Source,
    key: &str,
) -> Result<db::ArtistRow, ApiError> {
    find(db, source, key)
        .await?
        .ok_or_else(|| ApiError::not_found("the library files no such artist"))
}
