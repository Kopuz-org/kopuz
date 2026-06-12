use config::{AppConfig, SortCriterion, TrackSortField};
use dioxus::prelude::*;
use reader::Library;
use reader::models::Track;
use reader::sort;
use std::collections::HashMap;

pub struct LibraryItems {
    pub all_tracks: Memo<Vec<Track>>,
    pub album_covers: Memo<HashMap<String, Option<utils::CoverUrl>>>,
    pub artist_count: Memo<usize>,
    /// Multi-priority sort for the Tracks tab. Seeded from config; the view
    /// mirrors changes back into config for persistence.
    pub track_sort: Signal<Vec<SortCriterion<TrackSortField>>>,
}

pub fn use_library_items(library: Signal<Library>) -> LibraryItems {
    let config = use_context::<Signal<AppConfig>>();

    let initial_sort = config.read().track_sort.clone();
    let track_sort = use_signal(move || initial_sort);

    let artist_count = use_memo(move || {
        let lib = library.read();
        let mut artists = std::collections::HashSet::new();
        for album in &lib.albums {
            artists.insert(&album.artist);
        }
        for track in &lib.tracks {
            artists.insert(&track.artist);
        }
        artists.len()
    });

    let album_covers = use_memo(move || {
        let lib = library.read();

        lib.albums
            .iter()
            .map(|a| {
                (
                    a.id.clone(),
                    a.cover_path
                        .as_ref()
                        .and_then(|p| utils::format_artwork_url(Some(p))),
                )
            })
            .collect::<HashMap<String, Option<utils::CoverUrl>>>()
    });

    let all_tracks = use_memo(move || {
        let lib = library.read();
        let conf = config.read();
        let criteria = track_sort.read();

        let mut tracks: Vec<Track> = lib.tracks.to_vec();
        let album_years = sort::album_year_map(&lib.albums);
        let ctx = sort::TrackSortContext {
            listen_counts: Some(&conf.listen_counts),
            album_years: Some(&album_years),
        };
        sort::sort_tracks(&mut tracks, &criteria, ctx);

        tracks
    });

    LibraryItems {
        all_tracks,
        album_covers,
        artist_count,
        track_sort,
    }
}
