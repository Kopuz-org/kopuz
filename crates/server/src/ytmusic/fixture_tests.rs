//! Every parser against a response recorded from music.youtube.com with
//! hl=en, gl=US and no session, in `testdata/`.

use reader::models::Track;
use serde_json::Value;

use super::browse::{self, parse_continuation, parse_page};
use super::discover::{
    self, BrowsePage, DiscoverItem, DiscoverShelf, LinkKind, PageHeader, ShelfLayout,
};
use super::search::synthesize_album_id;

macro_rules! fixture {
    ($name:literal) => {
        serde_json::from_str::<Value>(include_str!(concat!("testdata/", $name, ".json")))
            .expect(concat!($name, " is JSON"))
    };
}

fn page(response: &Value) -> BrowsePage {
    parse_page(response).expect("a browse page")
}

fn titled<'a>(page: &'a BrowsePage, title: &str) -> &'a DiscoverShelf {
    page.shelves
        .iter()
        .find(|shelf| shelf.title == title)
        .unwrap_or_else(|| {
            let titles: Vec<&str> = page.shelves.iter().map(|s| s.title.as_str()).collect();
            panic!("no shelf {title:?} in {titles:?}")
        })
}

fn tracks(shelf: &DiscoverShelf) -> Vec<&Track> {
    shelf.items.iter().filter_map(track_of).collect()
}

fn track_of(item: &DiscoverItem) -> Option<&Track> {
    match item {
        DiscoverItem::Song(track) | DiscoverItem::Video(track) => Some(track),
        DiscoverItem::Episode { track, .. } => Some(track),
        _ => None,
    }
}

fn assert_track(track: &Track) {
    assert_eq!(track.id.key().len(), 11, "video id of {:?}", track.title);
    assert!(!track.title.trim().is_empty());
    assert!(
        track
            .cover
            .as_deref()
            .is_some_and(|c| c.starts_with("https://")),
        "{} has no cover",
        track.title
    );
}

/// Every shelf has something in it, and every playable item is a real track.
fn assert_shelves(page: &BrowsePage) {
    assert!(!page.shelves.is_empty(), "no shelves");
    for shelf in &page.shelves {
        assert!(
            !shelf.items.is_empty() || shelf.continuation.is_some(),
            "empty shelf {:?}",
            shelf.title
        );
        shelf
            .items
            .iter()
            .filter_map(track_of)
            .for_each(assert_track);
    }
}

fn all_tracks(page: &BrowsePage) -> Vec<&Track> {
    page.shelves.iter().flat_map(tracks).collect()
}

#[test]
fn home() {
    let home = page(&fixture!("home"));
    assert!(home.chips.len() >= 5, "chips: {:?}", home.chips);
    assert!(home.chips.iter().all(|chip| {
        chip.page_id.starts_with(&format!("{}?", discover::HOME)) && !chip.title.is_empty()
    }));
    assert!(home.continuation.is_some());
    assert_shelves(&home);
    assert!(
        home.shelves
            .iter()
            .all(|s| !s.title.is_empty() && s.layout == ShelfLayout::Carousel)
    );
}

#[test]
fn home_continuation() {
    let more = parse_continuation(&fixture!("home_continuation"));
    assert_shelves(&more);
    let quick_picks = titled(&more, "Quick picks");
    assert_eq!(quick_picks.layout, ShelfLayout::TrackGrid);
    let picks = tracks(quick_picks);
    assert!(picks.len() >= 8);
    assert!(
        picks
            .iter()
            .all(|t| !t.credits.is_empty() && !t.album.is_empty())
    );
}

/// The hand-written home parser the Discover page reads keeps its carousels.
#[test]
fn legacy_home_and_its_continuation() {
    let home = discover::parse_initial(&fixture!("home"));
    assert!(!home.shelves.is_empty());
    assert!(home.continuation.is_some());
    assert!(
        home.shelves
            .iter()
            .all(|s| s.layout == ShelfLayout::Carousel)
    );

    let more = discover::parse_continuation(&fixture!("home_continuation"));
    assert!(!more.shelves.is_empty());
}

#[test]
fn explore() {
    let explore = page(&fixture!("explore"));
    assert_shelves(&explore);
    let buttons: Vec<&str> = explore.shelves[0]
        .items
        .iter()
        .map(|item| match item {
            DiscoverItem::Page { page_id, .. } => page_id.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    let browse_ids: Vec<&str> = buttons
        .iter()
        .map(|id| browse::split_page_id(id).0)
        .collect();
    assert_eq!(
        browse_ids,
        [browse::NEW_RELEASES, browse::CHARTS, browse::MOODS]
    );
    let moods = titled(&explore, "Moods & genres");
    assert_eq!(moods.layout, ShelfLayout::Grid);
    assert_eq!(
        moods
            .more
            .as_ref()
            .map(|more| (more.kind, more.id.as_str())),
        Some((LinkKind::Page, browse::MOODS))
    );
    assert!(moods.items.iter().all(|item| matches!(
        item,
        DiscoverItem::Mood { accent: Some(_), browse_id, .. }
            if browse_id.starts_with(&format!("{}?", browse::MOOD_CATEGORY))
    )));
    let albums = titled(&explore, "New albums & singles");
    assert!(
        albums
            .items
            .iter()
            .all(|item| matches!(item, DiscoverItem::Album { .. }))
    );
    assert_eq!(titled(&explore, "Trending").layout, ShelfLayout::TrackGrid);
}

#[test]
fn charts() {
    let charts = page(&fixture!("charts"));
    assert_eq!(charts.header, PageHeader::Title);
    assert_eq!(charts.title, "Charts");
    assert_shelves(&charts);
    let artists = titled(&charts, "Top artists");
    assert!(artists.items.len() >= 10);
    assert!(artists.items.iter().all(|item| matches!(
        item,
        DiscoverItem::Artist {
            subtitle: Some(_),
            ..
        }
    )));
}

#[test]
fn moods_and_genres() {
    let moods = page(&fixture!("moods"));
    assert_eq!(moods.shelves.len(), 2);
    assert!(moods.shelves.iter().all(|s| s.layout == ShelfLayout::Grid));
    let tiles: Vec<&DiscoverItem> = moods.shelves.iter().flat_map(|s| &s.items).collect();
    assert!(tiles.len() >= 20);
    assert!(
        tiles
            .iter()
            .all(|item| matches!(item, DiscoverItem::Mood { .. }))
    );
}

#[test]
fn mood_category() {
    let category = page(&fixture!("mood_category"));
    assert_eq!(category.header, PageHeader::Title);
    assert_eq!(category.title, "Blues");
    assert_shelves(&category);
    let songs = titled(&category, "Songs");
    assert_eq!(songs.layout, ShelfLayout::TrackGrid);
    assert!(tracks(songs).iter().all(|t| !t.credits.is_empty()));
}

#[test]
fn new_releases() {
    let releases = page(&fixture!("new_releases"));
    assert_shelves(&releases);
    let albums = titled(&releases, "Albums & singles");
    assert!(albums.items.len() >= 10);
    assert_eq!(
        albums.more.as_ref().map(|more| more.kind),
        Some(LinkKind::Page)
    );
}

#[test]
fn legacy_artist() {
    let artist = discover::parse_artist("UCRr1xG_2WIDs18a6cIiCxeA", &fixture!("artist"));
    assert_eq!(artist.name, "Daft Punk");
    assert!(artist.subscribers.is_some() && artist.banner_thumbnail.is_some());
    assert!(artist.sections.len() >= 6);
    let top = artist
        .sections
        .iter()
        .find(|s| s.title == "Top songs")
        .expect("top songs");
    assert_eq!(top.layout, ShelfLayout::List);
    assert_eq!(
        top.more.as_ref().map(|more| more.kind),
        Some(LinkKind::Playlist)
    );
    // "Show all" on a carousel picks the shelf with params, so it opens as a page.
    let singles = artist
        .sections
        .iter()
        .find(|s| s.title == "Singles & EPs")
        .expect("singles");
    let more = singles.more.as_ref().expect("a show-all link");
    assert_eq!(more.kind, LinkKind::Page);
    assert!(more.id.contains('?'), "{}", more.id);
}

/// An artist's "show all", opened from the link above.
#[test]
fn artist_shelf() {
    let shelf = page(&fixture!("artist_singles"));
    assert_eq!(shelf.shelves.len(), 1);
    assert_eq!(shelf.shelves[0].layout, ShelfLayout::Grid);
    assert!(shelf.shelves[0].items.len() >= 10);
    assert!(
        shelf.shelves[0]
            .items
            .iter()
            .all(|item| matches!(item, DiscoverItem::Album { .. }))
    );
}

#[test]
fn legacy_album_and_single() {
    let album = discover::parse_album("MPREb_K8qWMWVqXGi", &fixture!("album"));
    assert_eq!(album.title, "Random Access Memories");
    assert_eq!(album.artist.as_deref(), Some("Daft Punk"));
    assert!(
        album
            .artist_id
            .as_deref()
            .is_some_and(|id| id.starts_with("UC"))
    );
    assert!(
        album
            .audio_playlist_id
            .as_deref()
            .is_some_and(|id| id.starts_with("OLAK5uy_"))
    );
    assert_eq!(album.tracks.len(), 13);
    album.tracks.iter().for_each(assert_track);

    let single = discover::parse_album("MPREb_X1DQ1j0PPrX", &fixture!("single"));
    assert_eq!(single.tracks.len(), 3);
}

#[test]
fn legacy_playlist_and_its_continuation() {
    // A row YouTube has made unplayable carries no video id and is left out.
    let (first, next) = super::search::walk_playlist_shelf(&fixture!("playlist_large"));
    assert!(first.len() >= 95, "{} rows", first.len());
    assert!(next.is_some());
    let (more, next) =
        super::search::walk_playlist_continuation(&fixture!("playlist_continuation"));
    assert!(more.len() >= 95, "{} rows", more.len());
    assert!(next.is_some());
    first.iter().chain(&more).for_each(assert_track);

    let (chart, _) = super::search::walk_playlist_shelf(&fixture!("playlist"));
    assert_eq!(chart.len(), 100);
}

/// More of one shelf comes back as that shelf's items and next token, not
/// as more of the page.
#[test]
fn a_shelf_continuation_continues_the_shelf() {
    let more = parse_continuation(&fixture!("playlist_continuation"));
    assert!(more.continuation.is_none());
    assert_eq!(more.shelves.len(), 1);
    assert_eq!(tracks(&more.shelves[0]).len(), 100);
    assert!(more.shelves[0].continuation.is_some());
}

#[test]
fn a_playlist_page_continues_both_its_tracks_and_its_shelves() {
    let playlist = page(&fixture!("playlist_large"));
    assert_eq!(playlist.header, PageHeader::Detail);
    let list = &playlist.shelves[0];
    assert_eq!(tracks(list).len(), 100);
    assert!(list.continuation.is_some(), "track continuation");
    assert!(
        playlist.continuation.is_some(),
        "related shelves continuation"
    );
}

#[test]
fn podcast() {
    let podcast = page(&fixture!("podcast"));
    assert_eq!(podcast.header, PageHeader::Detail);
    assert!(!podcast.title.is_empty() && podcast.description.is_some());
    let episodes = &podcast.shelves[0];
    assert!(episodes.items.len() >= 10);
    for item in &episodes.items {
        let DiscoverItem::Episode {
            track,
            browse_id,
            published,
        } = item
        else {
            panic!("{item:?}");
        };
        assert_track(track);
        assert!(track.duration > 0, "{}", track.title);
        assert!(published.is_some(), "{} has no release date", track.title);
        assert_eq!(browse_id, &format!("MPED{}", track.id.key()));
    }
    assert!(episodes.continuation.is_some());
}

/// An episode card's subtitle is "6 min 18 sec • Channel": its length is the
/// first part, not left at zero.
#[test]
fn episode_cards_carry_their_length() {
    let mut seen = 0;
    for page in [
        page(&fixture!("explore")),
        page(&fixture!("mood_category")),
        page(&fixture!("new_releases")),
    ] {
        for item in page.shelves.iter().flat_map(|shelf| &shelf.items) {
            let DiscoverItem::Episode { track, .. } = item else {
                continue;
            };
            seen += 1;
            assert!(track.duration > 0, "{} has no length", track.title);
            assert!(!track.artist.is_empty(), "{} has no channel", track.title);
        }
    }
    assert!(seen > 0, "no episode cards in the fixtures");
}

#[test]
fn episode() {
    let episode = page(&fixture!("episode"));
    assert_eq!(episode.header, PageHeader::Detail);
    assert!(episode.description.is_some());
    assert!(!episode.title.is_empty());
}

#[test]
fn related() {
    let related = page(&fixture!("related"));
    assert_shelves(&related);
    assert_eq!(
        titled(&related, "You might also like").layout,
        ShelfLayout::TrackGrid
    );
    assert!(
        titled(&related, "Similar artists")
            .items
            .iter()
            .all(|item| matches!(item, DiscoverItem::Artist { .. }))
    );
}

#[test]
fn the_watch_page_names_the_related_tab_and_the_mix() {
    let watch = fixture!("next_radio");
    assert!(
        browse::related_browse_id(&watch).is_some_and(|id| id.starts_with("MPTR")),
        "related tab"
    );
    let mix = super::mix::walk_queue(&watch);
    assert!(mix.len() >= 10, "{} tracks", mix.len());
    mix.iter().for_each(assert_track);
}

/// Saved library rows are keyed by this id, so its output is pinned on a
/// real album, and every track any parser builds from the fixtures keys its
/// album the same way: by browse id where the row links one, else by name.
#[test]
fn album_ids_are_what_saved_rows_hold() {
    assert_eq!(
        synthesize_album_id("Random Access Memories", "Daft Punk"),
        "ytmusic:album:72616e646f6d20616363657373206d656d6f726965737c646166742070756e6b"
    );
    assert_eq!(
        synthesize_album_id("", "Daft Punk"),
        "ytmusic:album:singles"
    );
    let album = discover::parse_album("MPREb_K8qWMWVqXGi", &fixture!("album"));
    let solo: Vec<&Track> = album
        .tracks
        .iter()
        .filter(|t| t.artist == "Daft Punk")
        .collect();
    assert!(!solo.is_empty());
    assert!(solo.iter().all(|t| t.album_id
        == "ytmusic:album:72616e646f6d20616363657373206d656d6f726965737c646166742070756e6b"));

    let pages = [
        page(&fixture!("home")),
        parse_continuation(&fixture!("home_continuation")),
        page(&fixture!("explore")),
        page(&fixture!("charts")),
        page(&fixture!("mood_category")),
        page(&fixture!("new_releases")),
        page(&fixture!("podcast")),
        page(&fixture!("related")),
        page(&fixture!("playlist")),
    ];
    let legacy: Vec<Track> = discover::parse_artist("UC", &fixture!("artist"))
        .sections
        .into_iter()
        .chain(discover::parse_initial(&fixture!("home")).shelves)
        .flat_map(|shelf| shelf.items)
        .filter_map(|item| match item {
            DiscoverItem::Song(track) => Some(*track),
            _ => None,
        })
        .chain(album.tracks.iter().cloned())
        .chain(super::search::walk_playlist_shelf(&fixture!("playlist")).0)
        .collect();
    let built: Vec<&Track> = pages.iter().flat_map(all_tracks).chain(&legacy).collect();
    assert!(built.len() > 300, "{} tracks", built.len());
    for track in built {
        let by_name = synthesize_album_id(&track.album, &track.artist);
        assert!(
            track.album_id == by_name || track.album_id.starts_with("ytmusic:album:MPRE"),
            "{:?} by {:?} on {:?} keyed {}",
            track.title,
            track.artist,
            track.album,
            track.album_id
        );
    }
}
