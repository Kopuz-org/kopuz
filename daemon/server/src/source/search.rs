//! In-memory Unicode-aware search for corpus-backed media sources.

use std::collections::{HashMap, HashSet};

use reader::{Album, Track};

fn fold_fields<'a>(fields: impl IntoIterator<Item = &'a str>) -> String {
    let fields = fields.into_iter();
    let mut folded = String::with_capacity(fields.size_hint().0 * 16);
    for field in fields {
        folded.extend(field.chars().flat_map(char::to_lowercase));
        folded.push('\0');
    }
    folded
}

/// Filter a lowercased query against tracks and albums from one source.
pub(super) fn filter(
    query: &str,
    tracks: Vec<Track>,
    albums: Vec<Album>,
) -> (Vec<Track>, Vec<Album>) {
    let album_genres: HashMap<&str, &str> = albums
        .iter()
        .map(|album| (album.id.as_str(), album.genre.as_str()))
        .collect();
    let matching_tracks = tracks
        .into_iter()
        .filter(|track| {
            let genre = album_genres
                .get(track.album_id.as_str())
                .copied()
                .unwrap_or_default();
            fold_fields([
                track.title.as_str(),
                track.artist.as_str(),
                track.album.as_str(),
                genre,
            ])
            .contains(query)
        })
        .take(100)
        .collect();
    drop(album_genres);

    let mut seen_titles = HashSet::new();
    let matching_albums = albums
        .into_iter()
        .filter(|album| {
            fold_fields([
                album.title.as_str(),
                album.artist.as_str(),
                album.genre.as_str(),
            ])
            .contains(query)
                && seen_titles.insert(fold_fields([album.title.trim()]))
        })
        .take(30)
        .collect();

    (matching_tracks, matching_albums)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_multiple_fields_into_one_search_buffer() {
        let folded = fold_fields(["Björk", "İstanbul", "ROCK"]);
        assert!(folded.contains(&"BJÖRK".to_lowercase()));
        assert!(folded.contains(&"İSTANBUL".to_lowercase()));
        assert!(folded.contains("rock"));
    }
}
