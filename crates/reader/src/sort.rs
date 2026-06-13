//! Multi-criteria sorting for the library (tracks) and album views.
//!
//! A sort is a list of [`SortCriterion`]s applied in priority order: the
//! first criterion that distinguishes two items decides their order, and the
//! rest break ties (e.g. Artist, then Year). Backs issues #265 (album sort
//! options) and #351 (multiple sort priorities per tab).
//!
//! All comparisons are case-insensitive for text. Absent values sort low
//! (a track with no `date_added` is treated as the oldest), so a descending
//! "recently added" sort naturally pushes undated items — e.g. YT Music —
//! to the bottom.

use crate::models::{Album, Track};
use config::{AlbumSortField, SortCriterion, SortDirection, TrackSortField};
use std::cmp::Ordering;
use std::collections::HashMap;

/// Data the track sort needs beyond the tracks themselves.
#[derive(Default, Clone, Copy)]
pub struct TrackSortContext<'a> {
    /// Play counts keyed by track id (the track's path as a string).
    pub listen_counts: Option<&'a HashMap<String, u64>>,
    /// Album release years keyed by `album_id`, used for the Year field
    /// (tracks do not carry a year of their own).
    pub album_years: Option<&'a HashMap<String, u16>>,
}

/// Data the album sort needs beyond the albums themselves.
#[derive(Default, Clone, Copy)]
pub struct AlbumSortContext<'a> {
    /// Aggregate play counts keyed by `album_id` (sum of member tracks).
    pub play_counts: Option<&'a HashMap<String, u64>>,
}

/// Sort `tracks` in place by `criteria` (priority order). A stable sort, so
/// items equal under all criteria keep their previous relative order.
pub fn sort_tracks(
    tracks: &mut [Track],
    criteria: &[SortCriterion<TrackSortField>],
    ctx: TrackSortContext<'_>,
) {
    tracks.sort_by(|a, b| compare_by(criteria, |c| compare_track(a, b, c, ctx)));
}

/// Sort `albums` in place by `criteria` (priority order).
pub fn sort_albums(
    albums: &mut [Album],
    criteria: &[SortCriterion<AlbumSortField>],
    ctx: AlbumSortContext<'_>,
) {
    albums.sort_by(|a, b| compare_by(criteria, |c| compare_album(a, b, c, ctx)));
}

/// Fold a list of criteria into a single ordering: return the first
/// non-equal comparison; equal under all criteria yields `Equal`.
fn compare_by<F, C>(criteria: &[SortCriterion<F>], mut cmp: C) -> Ordering
where
    C: FnMut(&SortCriterion<F>) -> Ordering,
{
    for criterion in criteria {
        let ord = cmp(criterion);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn compare_track(
    a: &Track,
    b: &Track,
    criterion: &SortCriterion<TrackSortField>,
    ctx: TrackSortContext<'_>,
) -> Ordering {
    let ord = match criterion.field {
        TrackSortField::Title => compare_text(&a.title, &b.title),
        TrackSortField::Artist => compare_text(&a.artist, &b.artist),
        TrackSortField::Album => compare_text(&a.album, &b.album),
        TrackSortField::Year => track_year(a, ctx).cmp(&track_year(b, ctx)),
        TrackSortField::PlayCount => track_plays(a, ctx).cmp(&track_plays(b, ctx)),
        TrackSortField::DateAdded => date_key(a.date_added).cmp(&date_key(b.date_added)),
    };
    direct(ord, criterion.direction)
}

fn compare_album(
    a: &Album,
    b: &Album,
    criterion: &SortCriterion<AlbumSortField>,
    ctx: AlbumSortContext<'_>,
) -> Ordering {
    let ord = match criterion.field {
        AlbumSortField::Title => compare_text(&a.title, &b.title),
        AlbumSortField::Artist => compare_text(&a.artist, &b.artist),
        AlbumSortField::Year => a.year.cmp(&b.year),
        AlbumSortField::PlayCount => album_plays(a, ctx).cmp(&album_plays(b, ctx)),
        AlbumSortField::DateAdded => date_key(a.date_added).cmp(&date_key(b.date_added)),
    };
    direct(ord, criterion.direction)
}

fn direct(ord: Ordering, direction: SortDirection) -> Ordering {
    match direction {
        SortDirection::Asc => ord,
        SortDirection::Desc => ord.reverse(),
    }
}

fn compare_text(left: &str, right: &str) -> Ordering {
    left.trim().to_lowercase().cmp(&right.trim().to_lowercase())
}

/// Map an optional date to a sortable key where absent dates rank lowest, so
/// they appear last under a descending ("most recent first") sort.
fn date_key(date: Option<i64>) -> i64 {
    date.unwrap_or(i64::MIN)
}

fn track_plays(track: &Track, ctx: TrackSortContext<'_>) -> u64 {
    ctx.listen_counts
        .and_then(|m| m.get(track.path.to_string_lossy().as_ref()).copied())
        .unwrap_or(0)
}

fn track_year(track: &Track, ctx: TrackSortContext<'_>) -> u16 {
    ctx.album_years
        .and_then(|m| m.get(&track.album_id).copied())
        .unwrap_or(0)
}

fn album_plays(album: &Album, ctx: AlbumSortContext<'_>) -> u64 {
    ctx.play_counts
        .and_then(|m| m.get(&album.id).copied())
        .unwrap_or(0)
}

/// Which track sort fields to offer for the current dataset, in display
/// order. Title, Artist, Album and Play Count are always present — play
/// counts are tracked for every source and song (starting at zero), so the
/// data is always "there". Year and Date Added only appear when at least one
/// track actually carries that value, so e.g. YT Music — which exposes no
/// release year or added-date — won't offer those as dead sort options.
pub fn available_track_fields(
    tracks: &[Track],
    album_years: &HashMap<String, u16>,
) -> Vec<TrackSortField> {
    let mut fields = vec![
        TrackSortField::Title,
        TrackSortField::Artist,
        TrackSortField::Album,
        TrackSortField::PlayCount,
    ];
    if tracks
        .iter()
        .any(|t| album_years.get(&t.album_id).is_some_and(|y| *y > 0))
    {
        fields.insert(3, TrackSortField::Year);
    }
    if tracks.iter().any(|t| t.date_added.is_some()) {
        fields.push(TrackSortField::DateAdded);
    }
    fields
}

/// Which album sort fields to offer for the current dataset, in display
/// order. See [`available_track_fields`]: Title, Artist and Play Count are
/// always present; Year and Date Added are gated on real data.
pub fn available_album_fields(albums: &[Album]) -> Vec<AlbumSortField> {
    let mut fields = vec![AlbumSortField::Title, AlbumSortField::Artist];
    if albums.iter().any(|a| a.year > 0) {
        fields.push(AlbumSortField::Year);
    }
    fields.push(AlbumSortField::PlayCount);
    if albums.iter().any(|a| a.date_added.is_some()) {
        fields.push(AlbumSortField::DateAdded);
    }
    fields
}

/// Build an `album_id -> year` map from a set of albums, for track-level
/// Year sorting.
pub fn album_year_map(albums: &[Album]) -> HashMap<String, u16> {
    albums.iter().map(|a| (a.id.clone(), a.year)).collect()
}

/// Build an `album_id -> total play count` map by summing the listen counts
/// of each track (keyed by its path string) into its album.
pub fn album_play_count_map(
    tracks: &[Track],
    listen_counts: &HashMap<String, u64>,
) -> HashMap<String, u64> {
    let mut out: HashMap<String, u64> = HashMap::new();
    for track in tracks {
        let id = track.path.to_string_lossy();
        if let Some(count) = listen_counts.get(id.as_ref()) {
            *out.entry(track.album_id.clone()).or_insert(0) += *count;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::SortCriterion;
    use std::path::PathBuf;

    fn track(title: &str, artist: &str, album: &str, date: Option<i64>) -> Track {
        Track {
            path: PathBuf::from(format!("/{title}-{artist}")),
            album_id: format!("{album}|{artist}"),
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
            date_added: date,
        }
    }

    fn album(id: &str, title: &str, artist: &str, year: u16, date: Option<i64>) -> Album {
        Album {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            genre: String::new(),
            year,
            cover_path: None,
            manual_cover: false,
            date_added: date,
        }
    }

    fn crit<F>(field: F, dir: SortDirection) -> SortCriterion<F> {
        SortCriterion::new(field, dir)
    }

    #[test]
    fn sorts_tracks_by_title_case_insensitive_ascending() {
        // Arrange
        let mut tracks = vec![
            track("banana", "x", "a", None),
            track("Apple", "x", "a", None),
            track("cherry", "x", "a", None),
        ];

        // Act
        sort_tracks(
            &mut tracks,
            &[crit(TrackSortField::Title, SortDirection::Asc)],
            TrackSortContext::default(),
        );

        // Assert
        let titles: Vec<&str> = tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Apple", "banana", "cherry"]);
    }

    #[test]
    fn multi_criteria_artist_then_year_breaks_ties() {
        // Arrange — same artist, different album years.
        let years: HashMap<String, u16> = HashMap::from([
            ("a2020|same".to_string(), 2020),
            ("a2010|same".to_string(), 2010),
        ]);
        let mut tracks = vec![
            {
                let mut t = track("t1", "same", "a2020", None);
                t.album_id = "a2020|same".to_string();
                t
            },
            {
                let mut t = track("t2", "same", "a2010", None);
                t.album_id = "a2010|same".to_string();
                t
            },
        ];
        let ctx = TrackSortContext {
            listen_counts: None,
            album_years: Some(&years),
        };

        // Act — primary Artist (tie), secondary Year ascending.
        sort_tracks(
            &mut tracks,
            &[
                crit(TrackSortField::Artist, SortDirection::Asc),
                crit(TrackSortField::Year, SortDirection::Asc),
            ],
            ctx,
        );

        // Assert — 2010 album before 2020 album.
        assert_eq!(tracks[0].album, "a2010");
        assert_eq!(tracks[1].album, "a2020");
    }

    #[test]
    fn descending_date_added_pushes_undated_last() {
        // Arrange — one undated track (e.g. YT Music) among dated ones.
        let mut tracks = vec![
            track("old", "x", "a", Some(1_000)),
            track("undated", "x", "a", None),
            track("new", "x", "a", Some(2_000)),
        ];

        // Act — most-recent first.
        sort_tracks(
            &mut tracks,
            &[crit(TrackSortField::DateAdded, SortDirection::Desc)],
            TrackSortContext::default(),
        );

        // Assert — newest, then older, then undated last.
        let titles: Vec<&str> = tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["new", "old", "undated"]);
    }

    #[test]
    fn sorts_albums_by_play_count_descending() {
        // Arrange
        let mut albums = vec![
            album("a1", "One", "x", 2000, None),
            album("a2", "Two", "x", 2001, None),
            album("a3", "Three", "x", 2002, None),
        ];
        let plays: HashMap<String, u64> =
            HashMap::from([("a1".into(), 5), ("a2".into(), 50), ("a3".into(), 1)]);
        let ctx = AlbumSortContext {
            play_counts: Some(&plays),
        };

        // Act
        sort_albums(
            &mut albums,
            &[crit(AlbumSortField::PlayCount, SortDirection::Desc)],
            ctx,
        );

        // Assert — most-played first.
        let ids: Vec<&str> = albums.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["a2", "a1", "a3"]);
    }

    #[test]
    fn album_play_count_map_sums_member_tracks() {
        // Arrange — two tracks in album "rock|x", one in "jazz|y".
        let t1 = track("a", "x", "rock", None);
        let t2 = track("b", "x", "rock", None);
        let t3 = track("c", "y", "jazz", None);
        let listen: HashMap<String, u64> = HashMap::from([
            (t1.path.to_string_lossy().to_string(), 3),
            (t2.path.to_string_lossy().to_string(), 4),
            (t3.path.to_string_lossy().to_string(), 9),
        ]);

        // Act
        let map = album_play_count_map(&[t1.clone(), t2, t3.clone()], &listen);

        // Assert
        assert_eq!(map.get(&t1.album_id).copied(), Some(7));
        assert_eq!(map.get(&t3.album_id).copied(), Some(9));
    }

    #[test]
    fn available_track_fields_hides_year_and_date_without_data() {
        // Arrange — YT-style tracks: no album year, no date_added.
        let tracks = vec![track("t1", "x", "a", None), track("t2", "y", "b", None)];
        let years = HashMap::new();

        // Act
        let fields = available_track_fields(&tracks, &years);

        // Assert — always-on fields only; no Year, no DateAdded.
        assert_eq!(
            fields,
            vec![
                TrackSortField::Title,
                TrackSortField::Artist,
                TrackSortField::Album,
                TrackSortField::PlayCount,
            ]
        );
    }

    #[test]
    fn available_track_fields_shows_year_and_date_when_present() {
        // Arrange — one dated track whose album has a known year.
        let mut t = track("t1", "x", "a", Some(123));
        t.album_id = "a|x".into();
        let tracks = vec![t];
        let years = HashMap::from([("a|x".to_string(), 1999u16)]);

        // Act
        let fields = available_track_fields(&tracks, &years);

        // Assert — full set, Year placed before PlayCount.
        assert_eq!(
            fields,
            vec![
                TrackSortField::Title,
                TrackSortField::Artist,
                TrackSortField::Album,
                TrackSortField::Year,
                TrackSortField::PlayCount,
                TrackSortField::DateAdded,
            ]
        );
    }

    #[test]
    fn available_album_fields_gates_year_and_date() {
        // Arrange — YT-style albums (year 0, no date) vs. a rich album.
        let bare = vec![album("a1", "One", "x", 0, None)];
        let rich = vec![album("a2", "Two", "x", 2001, Some(42))];

        // Act
        let bare_fields = available_album_fields(&bare);
        let rich_fields = available_album_fields(&rich);

        // Assert
        assert_eq!(
            bare_fields,
            vec![
                AlbumSortField::Title,
                AlbumSortField::Artist,
                AlbumSortField::PlayCount
            ]
        );
        assert_eq!(
            rich_fields,
            vec![
                AlbumSortField::Title,
                AlbumSortField::Artist,
                AlbumSortField::Year,
                AlbumSortField::PlayCount,
                AlbumSortField::DateAdded,
            ]
        );
    }

    #[test]
    fn empty_criteria_leaves_order_unchanged() {
        // Arrange
        let mut tracks = vec![track("c", "x", "a", None), track("a", "x", "a", None)];

        // Act
        sort_tracks(&mut tracks, &[], TrackSortContext::default());

        // Assert — stable: original order preserved.
        let titles: Vec<&str> = tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["c", "a"]);
    }
}
