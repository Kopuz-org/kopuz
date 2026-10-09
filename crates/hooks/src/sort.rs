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

#[cfg(test)]
mod tests {
    use super::*;

    fn album(id: &str, title: &str, artist: &str, year: u16, genre: &str) -> AlbumInfo {
        AlbumInfo {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            genre: genre.to_string(),
            year,
            ..AlbumInfo::default()
        }
    }

    fn crit(field: AlbumSortField, dir: SortDirection) -> SortCriterion<AlbumSortField> {
        SortCriterion::new(field, dir)
    }

    #[test]
    fn sorts_by_title_case_insensitive_ascending() {
        let mut albums = vec![
            album("1", "banana", "x", 0, ""),
            album("2", "Apple", "x", 0, ""),
            album("3", "cherry", "x", 0, ""),
        ];
        sort_albums(
            &mut albums,
            &[crit(AlbumSortField::Title, SortDirection::Asc)],
        );
        let titles: Vec<&str> = albums.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["Apple", "banana", "cherry"]);
    }

    #[test]
    fn artist_then_year_breaks_ties() {
        let mut albums = vec![
            album("1", "Later", "same", 2020, ""),
            album("2", "Earlier", "same", 2010, ""),
            album("3", "Other", "aaa", 1999, ""),
        ];
        sort_albums(
            &mut albums,
            &[
                crit(AlbumSortField::Artist, SortDirection::Asc),
                crit(AlbumSortField::Year, SortDirection::Asc),
            ],
        );
        let ids: Vec<&str> = albums.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["3", "2", "1"]);
    }

    #[test]
    fn year_descending() {
        let mut albums = vec![
            album("1", "A", "x", 2000, ""),
            album("2", "B", "x", 2020, ""),
            album("3", "C", "x", 2010, ""),
        ];
        sort_albums(
            &mut albums,
            &[crit(AlbumSortField::Year, SortDirection::Desc)],
        );
        let ids: Vec<&str> = albums.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["2", "3", "1"]);
    }

    #[test]
    fn available_fields_gate_year_and_genre() {
        let bare = vec![album("1", "One", "x", 0, "")];
        let rich = vec![album("2", "Two", "x", 2001, "Rock")];
        assert_eq!(
            available_album_fields(&bare),
            vec![AlbumSortField::Title, AlbumSortField::Artist]
        );
        assert_eq!(
            available_album_fields(&rich),
            vec![
                AlbumSortField::Title,
                AlbumSortField::Artist,
                AlbumSortField::Year,
                AlbumSortField::Genre,
            ]
        );
    }

    #[test]
    fn empty_criteria_leaves_order_unchanged() {
        let mut albums = vec![album("2", "C", "x", 0, ""), album("1", "A", "x", 0, "")];
        sort_albums(&mut albums, &[]);
        let ids: Vec<&str> = albums.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["2", "1"]);
    }
}
