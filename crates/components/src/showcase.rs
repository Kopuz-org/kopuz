use config::AppConfig;
use dioxus::prelude::*;
use reader::{Library, Track};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortField {
    Title,
    Artist,
    Album,
    Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

pub type SortState = Option<(SortField, SortDirection)>;

pub fn next_sort_state(current: SortState, field: SortField) -> SortState {
    match current {
        Some((current_field, SortDirection::Asc)) if current_field == field => {
            Some((field, SortDirection::Desc))
        }
        Some((current_field, SortDirection::Desc)) if current_field == field => None,
        _ => Some((field, SortDirection::Asc)),
    }
}

pub fn toggle_sort_state(mut sort_state: Signal<SortState>, field: SortField) {
    let next = next_sort_state(*sort_state.peek(), field);
    sort_state.set(next);
}

pub fn sort_icon(sort_state: SortState, field: SortField) -> &'static str {
    match sort_state {
        Some((current_field, SortDirection::Asc)) if current_field == field => {
            "fa-solid fa-sort-up"
        }
        Some((current_field, SortDirection::Desc)) if current_field == field => {
            "fa-solid fa-sort-down"
        }
        _ => "fa-solid fa-sort",
    }
}

pub fn sorted_track_pairs<T: Clone>(
    tracks: &[(Track, T)],
    sort_state: SortState,
) -> Vec<(Track, T)> {
    let tracks_for_sorting: Vec<Track> = tracks.iter().map(|(track, _)| track.clone()).collect();
    sorted_track_indices(&tracks_for_sorting, sort_state)
        .into_iter()
        .map(|idx| tracks[idx].clone())
        .collect()
}

pub fn sorted_track_indices(tracks: &[Track], sort_state: SortState) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..tracks.len()).collect();

    if let Some((field, direction)) = sort_state {
        indices.sort_by(|&left_idx, &right_idx| {
            let left = &tracks[left_idx];
            let right = &tracks[right_idx];

            let primary = match field {
                SortField::Title => compare_text(&left.title, &right.title),
                SortField::Artist => compare_text(&left.artist, &right.artist),
                SortField::Album => compare_text(&left.album, &right.album),
                SortField::Duration => left.duration.cmp(&right.duration),
            };
            let directional = match direction {
                SortDirection::Asc => primary,
                SortDirection::Desc => primary.reverse(),
            };
            directional.then_with(|| left_idx.cmp(&right_idx))
        });
    }

    indices
}

fn compare_text(left: &str, right: &str) -> Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

#[derive(Props, Clone, PartialEq)]
pub struct ShowcaseProps {
    pub name: String,
    pub description: String,
    pub cover_url: Option<utils::CoverUrl>,
    pub tracks: Vec<Track>,
    pub library: Signal<Library>,
    pub on_play_all: EventHandler<()>,
    pub on_play: EventHandler<usize>,
    pub on_queue: Option<EventHandler<usize>>,
    pub on_add_to_playlist: Option<EventHandler<usize>>,
    pub on_delete_track: Option<EventHandler<usize>>,
    pub on_remove_from_playlist: Option<EventHandler<usize>>,
    pub on_download_track: Option<EventHandler<usize>>,
    pub active_track: Option<std::path::PathBuf>,
    pub on_click_menu: Option<EventHandler<usize>>,
    pub on_close_menu: Option<EventHandler<()>>,
    pub actions: Option<Element>,
    pub on_download_all: Option<EventHandler<()>>,
    pub on_delete_all: Option<EventHandler<()>>,
    #[props(default = false)]
    pub is_album: bool,
    #[props(default = false)]
    pub is_downloading_all: bool,
    #[props(default = false)]
    pub is_selection_mode: bool,
    #[props(default = HashSet::new())]
    pub selected_tracks: HashSet<PathBuf>,
    pub on_select: Option<EventHandler<(usize, bool)>>,
    pub on_select_all: Option<EventHandler<bool>>,
    #[props(default = false)]
    pub all_selected: bool,
    pub on_long_press: Option<EventHandler<usize>>,
    pub on_cover_click: Option<EventHandler<()>>,
    #[props(default = false)]
    pub is_reorderable: bool,
    #[props(default)]
    pub on_move_up: EventHandler<usize>,
    #[props(default)]
    pub on_move_down: EventHandler<usize>,
}

#[component]
pub fn Showcase(props: ShowcaseProps) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    match config.read().ui_style {
        config::UiStyle::Modern => rsx! {
            crate::modern::showcase::ShowcaseModern { ..props }
        },
        config::UiStyle::Normal => rsx! {
            crate::normal::showcase::ShowcaseNormal { ..props }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SortDirection, SortField, SortState, next_sort_state, sort_icon, sorted_track_indices,
    };
    use reader::Track;
    use std::path::PathBuf;

    fn track(title: &str, artist: &str, album: &str, duration: u64) -> Track {
        Track {
            path: PathBuf::from(format!("/music/{title}.flac")),
            album_id: format!("album-{album}"),
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            duration,
            khz: 44_100,
            bitrate: 320,
            track_number: Some(1),
            disc_number: Some(1),
            musicbrainz_release_id: None,
            playlist_item_id: None,
            artists: vec![artist.to_string()],
        }
    }

    #[test]
    fn sort_state_cycles_asc_desc_none() {
        let state0: SortState = None;
        let state1 = next_sort_state(state0, SortField::Title);
        let state2 = next_sort_state(state1, SortField::Title);
        let state3 = next_sort_state(state2, SortField::Title);

        assert_eq!(state1, Some((SortField::Title, SortDirection::Asc)));
        assert_eq!(state2, Some((SortField::Title, SortDirection::Desc)));
        assert_eq!(state3, None);
    }

    #[test]
    fn sort_state_switches_fields_back_to_ascending() {
        let state = Some((SortField::Title, SortDirection::Desc));
        let next = next_sort_state(state, SortField::Artist);

        assert_eq!(next, Some((SortField::Artist, SortDirection::Asc)));
    }

    #[test]
    fn sort_icon_matches_state() {
        assert_eq!(
            sort_icon(
                Some((SortField::Album, SortDirection::Asc)),
                SortField::Album
            ),
            "fa-solid fa-sort-up"
        );
        assert_eq!(
            sort_icon(
                Some((SortField::Album, SortDirection::Desc)),
                SortField::Album
            ),
            "fa-solid fa-sort-down"
        );
        assert_eq!(sort_icon(None, SortField::Album), "fa-solid fa-sort");
    }

    #[test]
    fn sorted_track_indices_are_case_insensitive_and_stable() {
        let tracks = vec![
            track("beta", "Zulu", "Two", 200),
            track("Alpha", "alpha", "One", 180),
            track("alpha", "Bravo", "Three", 240),
        ];

        let sorted = sorted_track_indices(&tracks, Some((SortField::Title, SortDirection::Asc)));

        assert_eq!(sorted, vec![1, 2, 0]);
    }

    #[test]
    fn sorted_track_indices_support_duration_descending() {
        let tracks = vec![
            track("a", "a", "a", 120),
            track("b", "b", "b", 360),
            track("c", "c", "c", 240),
        ];

        let sorted =
            sorted_track_indices(&tracks, Some((SortField::Duration, SortDirection::Desc)));

        assert_eq!(sorted, vec![1, 2, 0]);
    }

    #[test]
    fn sorted_track_pairs_maintains_pairing() {
        use super::sorted_track_pairs;

        let pairs = vec![
            (track("Z", "a", "a", 120), "z-meta"),
            (track("A", "b", "b", 360), "a-meta"),
            (track("M", "c", "c", 240), "m-meta"),
        ];

        let sorted = sorted_track_pairs(&pairs, Some((SortField::Title, SortDirection::Asc)));

        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].0.title, "A");
        assert_eq!(sorted[0].1, "a-meta");
        assert_eq!(sorted[1].0.title, "M");
        assert_eq!(sorted[1].1, "m-meta");
        assert_eq!(sorted[2].0.title, "Z");
        assert_eq!(sorted[2].1, "z-meta");
    }
}
