use config::{AppConfig, SortOrder};
use dioxus::prelude::*;
use reader::Library;
use reader::models::Track;

pub struct LibraryItems {
    pub all_tracks: Memo<Vec<(Track, Option<String>)>>,
    pub artist_count: Memo<usize>,
    pub sort_order: Signal<SortOrder>,
}

pub fn use_library_items(library: Signal<Library>) -> LibraryItems {
    let config = use_context::<Signal<AppConfig>>();

    let initial_sort_order = config.read().sort_order.clone();
    let sort_order = use_signal(move || initial_sort_order);

    use_effect({
        let mut config = config.clone();
        let sort_order = sort_order.clone();
        move || {
            let curr = sort_order.read().clone();
            if config.read().sort_order != curr {
                config.write().sort_order = curr;
            }
        }
    });

    let artist_count = use_memo(move || {
        let lib = library.read();
        let mut artists = std::collections::HashSet::new();
        for album in &lib.albums {
            artists.insert(album.artist.clone());
        }
        for track in &lib.tracks {
            artists.insert(track.artist.clone());
        }
        artists.len()
    });

    let all_tracks = use_memo(move || {
        let lib = library.read();

        let mut tracks: Vec<(Track, Option<String>)> = lib
            .tracks
            .iter()
            .map(|track| {
                let album = lib.albums.iter().find(|a| a.id == track.album_id);
                let cover_url = album
                    .and_then(|a| a.cover_path.as_ref())
                    .and_then(|p| utils::format_artwork_url(Some(p)));
                (track.clone(), cover_url)
            })
            .collect();

        match *sort_order.read() {
            SortOrder::Title => {
                tracks.sort_by(|(a, _), (b, _)| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            }
            SortOrder::Artist => tracks
                .sort_by(|(a, _), (b, _)| a.artist.to_lowercase().cmp(&b.artist.to_lowercase())),
            SortOrder::Album => {
                tracks.sort_by(|(a, _), (b, _)| a.album.to_lowercase().cmp(&b.album.to_lowercase()))
            }
        }

        tracks
    });

    LibraryItems {
        all_tracks,
        artist_count,
        sort_order,
    }
}
