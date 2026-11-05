//! Sorting a track table by the heading pressed: the artist and genre pages share it.

use api::TrackInfo;
use config::SortDirection;

use crate::album_collection::compare_text;

/// A sortable column of a track table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Title,
    Artist,
    Album,
    Duration,
}

impl Column {
    const ALL: [Self; 4] = [Self::Title, Self::Artist, Self::Album, Self::Duration];

    /// The table column it sorts, by the id the shared header names it with.
    pub fn id(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Duration => "duration",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|column| column.id() == id)
    }
}

/// A press on a heading sorts by it ascending, then descending, then back to the order the daemon gave.
pub fn next_sort(
    current: Option<(Column, SortDirection)>,
    pressed: Column,
) -> Option<(Column, SortDirection)> {
    match current {
        Some((column, SortDirection::Asc)) if column == pressed => {
            Some((pressed, SortDirection::Desc))
        }
        Some((column, SortDirection::Desc)) if column == pressed => None,
        _ => Some((pressed, SortDirection::Asc)),
    }
}

pub fn sort_tracks(tracks: &mut [TrackInfo], column: Column, direction: SortDirection) {
    tracks.sort_by(|left, right| {
        let ordering = match column {
            Column::Title => compare_text(&left.title, &right.title),
            Column::Artist => compare_text(&left.artist, &right.artist),
            Column::Album => compare_text(&left.album, &right.album),
            Column::Duration => left.duration_ms.cmp(&right.duration_ms),
        };
        let ordering = match direction {
            SortDirection::Asc => ordering,
            SortDirection::Desc => ordering.reverse(),
        };
        // An album keeps its own running order inside the sort.
        match column {
            Column::Album => ordering
                .then_with(|| {
                    left.disc_number
                        .unwrap_or(0)
                        .cmp(&right.disc_number.unwrap_or(0))
                })
                .then_with(|| {
                    left.track_number
                        .unwrap_or(0)
                        .cmp(&right.track_number.unwrap_or(0))
                })
                .then_with(|| compare_text(&left.title, &right.title)),
            _ => ordering,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, album: &str, disc: u32, number: u32) -> TrackInfo {
        TrackInfo {
            title: title.to_string(),
            album: album.to_string(),
            disc_number: Some(disc),
            track_number: Some(number),
            ..TrackInfo::default()
        }
    }

    fn titles(tracks: &[TrackInfo]) -> Vec<&str> {
        tracks.iter().map(|track| track.title.as_str()).collect()
    }

    #[test]
    fn a_heading_sorts_up_then_down_then_lets_go() {
        let up = next_sort(None, Column::Title);
        assert_eq!(up, Some((Column::Title, SortDirection::Asc)));
        let down = next_sort(up, Column::Title);
        assert_eq!(down, Some((Column::Title, SortDirection::Desc)));
        assert_eq!(next_sort(down, Column::Title), None);
        assert_eq!(
            next_sort(down, Column::Artist),
            Some((Column::Artist, SortDirection::Asc))
        );
    }

    #[test]
    fn an_album_sort_keeps_each_album_in_running_order() {
        let mut tracks = [
            track("b2", "B", 1, 2),
            track("a2", "A", 2, 1),
            track("a1", "A", 1, 5),
            track("b1", "B", 1, 1),
        ];
        sort_tracks(&mut tracks, Column::Album, SortDirection::Asc);
        assert_eq!(titles(&tracks), ["a1", "a2", "b1", "b2"]);
        sort_tracks(&mut tracks, Column::Album, SortDirection::Desc);
        assert_eq!(titles(&tracks), ["b1", "b2", "a1", "a2"]);
    }

    #[test]
    fn column_ids_round_trip() {
        for column in Column::ALL {
            assert_eq!(Column::parse(column.id()), Some(column));
        }
        assert_eq!(Column::parse("plays"), None);
    }
}
