use crate::models::Album;
use config::{AlbumSortField, SortCriterion, SortDirection};
use std::cmp::Ordering;

pub fn sort_albums(albums: &mut [Album], criteria: &[SortCriterion<AlbumSortField>]) {
    albums.sort_by(|a, b| {
        for criterion in criteria {
            let ord = compare_album(a, b, criterion);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    });
}

fn compare_album(a: &Album, b: &Album, criterion: &SortCriterion<AlbumSortField>) -> Ordering {
    let ord = match criterion.field {
        AlbumSortField::Title => compare_text(&a.title, &b.title),
        AlbumSortField::Artist => compare_text(&a.artist, &b.artist),
        AlbumSortField::Year => a.year.cmp(&b.year),
        AlbumSortField::Genre => compare_text(&a.genre, &b.genre),
    };
    match criterion.direction {
        SortDirection::Asc => ord,
        SortDirection::Desc => ord.reverse(),
    }
}

fn compare_text(left: &str, right: &str) -> Ordering {
    left.trim().to_lowercase().cmp(&right.trim().to_lowercase())
}

pub fn available_album_fields(albums: &[Album]) -> Vec<AlbumSortField> {
    let mut fields = vec![AlbumSortField::Title, AlbumSortField::Artist];
    if albums.iter().any(|a| a.year > 0) {
        fields.push(AlbumSortField::Year);
    }
    if albums.iter().any(|a| !a.genre.trim().is_empty()) {
        fields.push(AlbumSortField::Genre);
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(id: &str, title: &str, artist: &str, year: u16, genre: &str) -> Album {
        Album {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            genre: genre.to_string(),
            year,
            cover_path: None,
            manual_cover: false,
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
