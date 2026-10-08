//! Ordering rows for display.
//!
//! What a list is sorted by is the person's choice, not the library's, so it
//! is applied here rather than asked of the daemon: the rows are already on
//! screen and re-fetching them to reorder would be a round trip for nothing.

use api::AlbumInfo;
use config::{AlbumSortField, SortCriterion, SortDirection};
use std::cmp::Ordering;

pub fn sort_albums(albums: &mut [AlbumInfo], criteria: &[SortCriterion<AlbumSortField>]) {
    albums.sort_by(|left, right| {
        for criterion in criteria {
            let ordering = compare_album(left, right, criterion);
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    });
}

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

fn compare_album(
    left: &AlbumInfo,
    right: &AlbumInfo,
    criterion: &SortCriterion<AlbumSortField>,
) -> Ordering {
    let ordering = match criterion.field {
        AlbumSortField::Title => compare_text(&left.title, &right.title),
        AlbumSortField::Artist => compare_text(&left.artist, &right.artist),
        AlbumSortField::Year => left.year.cmp(&right.year),
        AlbumSortField::Genre => compare_text(&left.genre, &right.genre),
    };
    match criterion.direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    }
}

fn compare_text(left: &str, right: &str) -> Ordering {
    left.trim().to_lowercase().cmp(&right.trim().to_lowercase())
}
