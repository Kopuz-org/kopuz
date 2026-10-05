//! Which orderings a list offers. The ordering itself is the daemon's: a page
//! hands it the person's criteria in an `AlbumQuery` or `ArtistQuery`.

use api::AlbumInfo;
use config::AlbumSortField;

/// The fields worth offering: one nothing carries a value for would sort the
/// list into the order it was already in.
pub fn available_album_fields(albums: &[AlbumInfo]) -> Vec<AlbumSortField> {
    let mut fields = vec![AlbumSortField::Title, AlbumSortField::Artist];
    if albums.iter().any(|album| album.year > 0) {
        fields.push(AlbumSortField::Year);
    }
    if albums.iter().any(|album| !album.genre.trim().is_empty()) {
        fields.push(AlbumSortField::Genre);
    }
    fields
}
