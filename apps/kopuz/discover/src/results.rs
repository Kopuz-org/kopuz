//! What a query turned up: the strips of albums and artists over the list of matching tracks.

use std::rc::Rc;

use api::{AlbumInfo, TrackInfo};
use components::{Carousel, Heading, Tile, icons};
use gpui_kit::{AnyElement, IntoElement, Pixels, SharedString, px};
use store::{Pictured, Playing, Route, Target};

const ALBUM_TILE: Pixels = px(128.);
// An artist strip has no artwork to show off, so its tiles only have to be legible.
const ARTIST_TILE: Pixels = px(88.);
/// How many of each a strip is worth showing; the rest are one more word in the query away.
const STRIP: usize = 16;

/// Search answers with tracks and albums; the artists behind them are worth a row of their own.
pub fn artists_of(tracks: &[TrackInfo], albums: &[AlbumInfo]) -> Vec<(String, SharedString)> {
    let mut artists: Vec<(String, SharedString)> = Vec::new();
    let credits = tracks
        .iter()
        .filter_map(|track| {
            let credit = track.primary_credit()?;
            Some((credit.key.clone()?, credit.name.as_str()))
        })
        .chain(
            albums
                .iter()
                .filter_map(|album| Some((album.artist_key.clone()?, album.artist.as_str()))),
        );
    for (key, name) in credits {
        let name = name.trim();
        if name.is_empty() || artists.len() >= STRIP {
            continue;
        }
        if artists.iter().any(|(held, _)| held == &key) {
            continue;
        }
        artists.push((key, name.to_string().into()));
    }
    artists
}

/// One album a query turned up: opens the album, right-clicks to the album's own menu.
fn album_card(album: &AlbumInfo, playing: &Playing) -> AnyElement {
    Tile::new(
        SharedString::from(format!("result-album-{}", album.id)),
        album.title.clone(),
    )
    .subtitle(album.artist.clone())
    .artwork(album.artwork.as_ref())
    .pictured(Pictured::Album {
        artist: album.artist.clone().into(),
        title: album.title.clone().into(),
    })
    .placeholder(icons::DISC)
    .size(ALBUM_TILE)
    .playing(playing.is_album(&album.id))
    .route(Route::Album(album.id.clone().into()))
    .menu(Some(Target::Album {
        id: album.id.clone().into(),
        title: album.title.clone().into(),
    }))
    .into_any_element()
}

/// One artist credited on what a query turned up.
fn artist_card(key: &String, name: &SharedString, playing: &Playing) -> AnyElement {
    Tile::new(
        SharedString::from(format!("result-artist-{key}")),
        name.clone(),
    )
    .placeholder(icons::MIC)
    .size(ARTIST_TILE)
    .round(true)
    .playing(playing.is_artist(key))
    .route(Route::Artist(key.clone()))
    .menu(Some(Target::Artist {
        key: key.clone(),
        name: name.clone(),
    }))
    .into_any_element()
}

/// A band of cards that slides sideways, so the track list below keeps the page's one scroller.
fn strip(
    id: &'static str,
    title: SharedString,
    count: usize,
    tile: Pixels,
    build: impl Fn(usize) -> AnyElement + 'static,
) -> AnyElement {
    Carousel::new(id, title, count, tile, move |index, _, _| build(index))
        .gap(px(16.))
        .heading(Heading::Band)
        .into_any_element()
}

/// The albums a query turned up, or nothing at all when it turned up none.
pub fn album_strip(
    albums: &[AlbumInfo],
    title: SharedString,
    playing: &Playing,
) -> Option<AnyElement> {
    if albums.is_empty() {
        return None;
    }
    let albums: Rc<[AlbumInfo]> = albums.iter().take(STRIP).cloned().collect();
    let playing = playing.clone();
    Some(strip(
        "result-albums",
        title,
        albums.len(),
        ALBUM_TILE,
        move |index| album_card(&albums[index], &playing),
    ))
}

/// The artists credited on what a query turned up.
pub fn artist_strip(
    artists: &[(String, SharedString)],
    title: SharedString,
    playing: &Playing,
) -> Option<AnyElement> {
    if artists.is_empty() {
        return None;
    }
    let artists: Rc<[(String, SharedString)]> = artists.into();
    let playing = playing.clone();
    Some(strip(
        "result-artists",
        title,
        artists.len(),
        ARTIST_TILE,
        move |index| {
            let (key, name) = &artists[index];
            artist_card(key, name, &playing)
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(artist: &str, key: &str) -> TrackInfo {
        TrackInfo {
            artist: artist.to_string(),
            credits: vec![api::ArtistCredit {
                name: artist.to_string(),
                key: Some(key.to_string()),
            }],
            ..TrackInfo::default()
        }
    }

    fn names(artists: &[(String, SharedString)]) -> Vec<&str> {
        artists.iter().map(|(_, name)| name.as_ref()).collect()
    }

    #[test]
    fn an_artist_is_listed_once_per_key_whatever_its_credits_say() {
        let tracks = [
            track("Boards of Canada", "boc"),
            track("boards of canada", "boc"),
            track("Boards of Canada", "a-namesake"),
        ];
        assert_eq!(
            names(&artists_of(&tracks, &[])),
            ["Boards of Canada", "Boards of Canada"]
        );
    }

    #[test]
    fn a_track_with_no_credit_contributes_no_artist() {
        let tracks = [track("  ", "blank"), track("Autechre", "autechre")];
        assert_eq!(names(&artists_of(&tracks, &[])), ["Autechre"]);
    }

    #[test]
    fn a_strip_never_grows_past_what_it_can_show() {
        let tracks: Vec<TrackInfo> = (0..40)
            .map(|at| track(&format!("artist {at}"), &format!("key-{at}")))
            .collect();
        assert_eq!(artists_of(&tracks, &[]).len(), STRIP);
    }
}
