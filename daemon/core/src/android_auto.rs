//! What Android Auto lists and plays. The car asks for the children of a
//! media id and later plays one it picked, so every id carries enough to
//! rebuild its queue on its own: the car may ask for it long after the list it
//! came from was read, and the list may have changed since.
//!
//! This runs on the core's runtime rather than in a frontend, so the car is
//! answered whatever UI the process hosts and however asleep that UI is.

// Off Android only the tests reach this module.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use api::{AlbumQuery, ApiError, ArtistQuery, KopuzApi, Page, QueueContext, QueueMode, TrackInfo};

const ROOT: &str = "root";
const RECENT: &str = "recent";
const ALBUMS: &str = "albums";
const ARTISTS: &str = "artists";
const PLAYLISTS: &str = "playlists";

/// The car pages long lists itself, but every row crosses a binder call, so
/// a huge library is cut here rather than shipped whole.
const LIST_LIMIT: u32 = 500;
const RECENT_LIMIT: u32 = 100;
const SEARCH_LIMIT: usize = 50;

/// One row of a car browse screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CarItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub browsable: bool,
    pub playable: bool,
}

impl CarItem {
    fn folder(id: String, title: impl Into<String>, subtitle: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            subtitle: subtitle.into(),
            browsable: true,
            playable: false,
        }
    }

    fn track(id: String, track: &TrackInfo) -> Self {
        Self {
            id,
            title: track.title.clone(),
            subtitle: track.artist.clone(),
            browsable: false,
            playable: true,
        }
    }
}

/// Which list a track was picked from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Container {
    Album,
    Artist,
    Playlist,
}

impl Container {
    fn name(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "album" => Some(Self::Album),
            "artist" => Some(Self::Artist),
            "playlist" => Some(Self::Playlist),
            _ => None,
        }
    }
}

/// A parsed media id. Album ids, artist keys and track keys are free-form and
/// can hold the separator, so a track id length-prefixes its key and the
/// container id always comes last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaId {
    Root,
    Recent,
    Albums,
    Artists,
    Playlists,
    List(Container, String),
    /// A track picked from a list, by key, so it is found again even after the
    /// list was reordered or grew.
    Track {
        container: Container,
        id: String,
        key: String,
    },
    RecentTrack(String),
}

impl MediaId {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            ROOT => return Some(Self::Root),
            RECENT => return Some(Self::Recent),
            ALBUMS => return Some(Self::Albums),
            ARTISTS => return Some(Self::Artists),
            PLAYLISTS => return Some(Self::Playlists),
            _ => {}
        }
        let (kind, rest) = raw.split_once('/')?;
        if let Some(container) = Container::from_name(kind) {
            return Some(Self::List(container, rest.to_owned()));
        }
        if kind != "play" {
            return None;
        }
        let (what, rest) = rest.split_once('/')?;
        if what == "recent" {
            return Some(Self::RecentTrack(rest.to_owned()));
        }
        let container = Container::from_name(what)?;
        let (len, rest) = rest.split_once('/')?;
        let len: usize = len.parse().ok()?;
        let key = rest.get(..len)?;
        let id = rest.get(len..)?;
        Some(Self::Track {
            container,
            id: id.to_owned(),
            key: key.to_owned(),
        })
    }

    pub(crate) fn encode(&self) -> String {
        match self {
            Self::Root => ROOT.to_owned(),
            Self::Recent => RECENT.to_owned(),
            Self::Albums => ALBUMS.to_owned(),
            Self::Artists => ARTISTS.to_owned(),
            Self::Playlists => PLAYLISTS.to_owned(),
            Self::List(container, id) => format!("{}/{id}", container.name()),
            Self::Track { container, id, key } => {
                format!("play/{}/{}/{key}{id}", container.name(), key.len())
            }
            Self::RecentTrack(key) => format!("play/recent/{key}"),
        }
    }
}

fn list(limit: u32) -> Page {
    Page { offset: 0, limit }
}

fn count(n: usize, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// The tracks of a list, read the way the browse screen read them.
async fn list_tracks(
    api: &dyn KopuzApi,
    container: Container,
    id: &str,
) -> Result<Vec<TrackInfo>, ApiError> {
    match container {
        Container::Album => Ok(api
            .album_tracks(id.to_owned(), list(LIST_LIMIT))
            .await?
            .items),
        Container::Artist => Ok(api
            .artist_tracks(id.to_owned(), list(LIST_LIMIT))
            .await?
            .items),
        Container::Playlist => {
            let catalog = api.playlists().await?;
            let Some(playlist) = catalog
                .playlists
                .into_iter()
                .find(|playlist| playlist.id == id)
            else {
                return Ok(Vec::new());
            };
            let mut keys = playlist.track_keys;
            keys.truncate(LIST_LIMIT as usize);
            api.tracks_by_keys(keys).await
        }
    }
}

/// The rows under `parent`. An id the car made up, or one from an older
/// build, lists nothing rather than failing the whole screen.
pub(crate) async fn children(api: &dyn KopuzApi, parent: &str) -> Result<Vec<CarItem>, ApiError> {
    let Some(parent) = MediaId::parse(parent) else {
        return Ok(Vec::new());
    };
    let items = match parent {
        MediaId::Root => vec![
            CarItem::folder(MediaId::Recent.encode(), "Recently played", ""),
            CarItem::folder(MediaId::Albums.encode(), "Albums", ""),
            CarItem::folder(MediaId::Artists.encode(), "Artists", ""),
            CarItem::folder(MediaId::Playlists.encode(), "Playlists", ""),
        ],
        MediaId::Recent => api
            .recent_tracks(list(RECENT_LIMIT))
            .await?
            .items
            .iter()
            .map(|track| CarItem::track(MediaId::RecentTrack(track.key.clone()).encode(), track))
            .collect(),
        MediaId::Albums => api
            .albums(AlbumQuery::default(), list(LIST_LIMIT))
            .await?
            .albums
            .into_iter()
            .map(|album| {
                let id = MediaId::List(Container::Album, album.id).encode();
                CarItem::folder(id, album.title, album.artist)
            })
            .collect(),
        MediaId::Artists => api
            .artists(ArtistQuery::default(), list(LIST_LIMIT))
            .await?
            .artists
            .into_iter()
            .map(|artist| {
                let id = MediaId::List(Container::Artist, artist.key).encode();
                let albums = count(artist.album_count as usize, "album", "albums");
                CarItem::folder(id, artist.name, albums)
            })
            .collect(),
        MediaId::Playlists => api
            .playlists()
            .await?
            .playlists
            .into_iter()
            .map(|playlist| {
                let tracks = count(playlist.track_keys.len(), "track", "tracks");
                let id = MediaId::List(Container::Playlist, playlist.id).encode();
                CarItem::folder(id, playlist.name, tracks)
            })
            .collect(),
        MediaId::List(container, id) => list_tracks(api, container, &id)
            .await?
            .iter()
            .map(|track| {
                let media_id = MediaId::Track {
                    container,
                    id: id.clone(),
                    key: track.key.clone(),
                };
                CarItem::track(media_id.encode(), track)
            })
            .collect(),
        MediaId::Track { .. } | MediaId::RecentTrack(_) => Vec::new(),
    };
    Ok(items)
}

async fn set_queue(
    api: &dyn KopuzApi,
    context: QueueContext,
    start_index: Option<u32>,
) -> Result<(), ApiError> {
    api.set_queue(api::SetQueueRequest {
        mode: QueueMode::Replace,
        context,
        start_index,
        shuffle: None,
    })
    .await
    .map(|_| ())
}

/// Play `keys` from `key`, the way the app plays a tapped row: the list as it
/// is now, starting at the track the car showed. A track no longer in the list
/// plays on its own.
async fn play_from(api: &dyn KopuzApi, keys: Vec<String>, key: String) -> Result<(), ApiError> {
    match keys.iter().position(|candidate| *candidate == key) {
        Some(index) => set_queue(api, QueueContext::Tracks { keys }, Some(index as u32)).await,
        None => set_queue(api, QueueContext::Tracks { keys: vec![key] }, None).await,
    }
}

fn keys_of(tracks: Vec<TrackInfo>) -> Vec<String> {
    tracks.into_iter().map(|track| track.key).collect()
}

/// Replace the queue with what `media_id` names and start it.
pub(crate) async fn play(api: &dyn KopuzApi, media_id: &str) -> Result<(), ApiError> {
    let Some(id) = MediaId::parse(media_id) else {
        tracing::debug!(media_id, "car asked to play an unknown media id");
        return Ok(());
    };
    match id {
        MediaId::Track { container, id, key } => {
            let keys = keys_of(list_tracks(api, container, &id).await?);
            play_from(api, keys, key).await
        }
        MediaId::RecentTrack(key) => {
            let keys = keys_of(api.recent_tracks(list(RECENT_LIMIT)).await?.items);
            play_from(api, keys, key).await
        }
        MediaId::List(Container::Album, id) => {
            set_queue(api, QueueContext::Album { id }, None).await
        }
        MediaId::List(Container::Artist, artist) => {
            set_queue(api, QueueContext::Artist { artist }, None).await
        }
        MediaId::List(Container::Playlist, id) => {
            set_queue(api, QueueContext::Playlist { id }, None).await
        }
        MediaId::Root
        | MediaId::Recent
        | MediaId::Albums
        | MediaId::Artists
        | MediaId::Playlists => Ok(()),
    }
}

/// A spoken "play ... on Kopuz": the search's tracks, best match first. An
/// empty query is the car's "just play something", which resumes the queue.
pub(crate) async fn play_search(api: &dyn KopuzApi, query: &str) -> Result<(), ApiError> {
    if query.trim().is_empty() {
        return api
            .player_command(api::PlayerCommand::Play)
            .await
            .map(|_| ());
    }
    let results = api.search(api::SearchRequest::new(query)).await?;
    let keys: Vec<String> = results
        .tracks
        .into_iter()
        .take(SEARCH_LIMIT)
        .map(|track| track.key)
        .collect();
    if keys.is_empty() {
        return Ok(());
    }
    set_queue(api, QueueContext::Tracks { keys }, Some(0)).await
}

/// Answer the car from `api`. Browsing runs concurrently, so a slow list
/// never holds up another; plays run one after another in the order the car
/// sent them, so a slow voice search cannot land after, and replace, a track
/// picked later.
#[cfg(target_os = "android")]
pub fn spawn(api: std::sync::Arc<dyn KopuzApi>) {
    use player::systemint::{BrowseItem, CarRequest};

    fn rows(items: Vec<CarItem>) -> Vec<BrowseItem> {
        items
            .into_iter()
            .map(|item| BrowseItem {
                id: item.id,
                title: item.title,
                subtitle: item.subtitle,
                browsable: item.browsable,
                playable: item.playable,
            })
            .collect()
    }

    enum PlayRequest {
        MediaId(String),
        Search(String),
    }

    let (browse_tx, mut browse_rx) = tokio::sync::mpsc::unbounded_channel();
    let (play_tx, mut play_rx) = tokio::sync::mpsc::unbounded_channel();
    player::systemint::set_car_handler(move |request| match request {
        CarRequest::Children { parent, reply } => {
            let _ = browse_tx.send((parent, reply));
        }
        CarRequest::Play { media_id } => {
            let _ = play_tx.send(PlayRequest::MediaId(media_id));
        }
        CarRequest::PlaySearch { query } => {
            let _ = play_tx.send(PlayRequest::Search(query));
        }
    });

    let browse_api = api.clone();
    tokio::spawn(async move {
        while let Some((parent, reply)) = browse_rx.recv().await {
            let api = browse_api.clone();
            tokio::spawn(async move {
                let items = children(api.as_ref(), &parent)
                    .await
                    .unwrap_or_else(|error| {
                        tracing::warn!(%error, parent, "car browse failed");
                        Vec::new()
                    });
                let _ = reply.send(rows(items));
            });
        }
    });

    tokio::spawn(async move {
        while let Some(request) = play_rx.recv().await {
            let outcome = match &request {
                PlayRequest::MediaId(media_id) => play(api.as_ref(), media_id).await,
                PlayRequest::Search(query) => play_search(api.as_ref(), query).await,
            };
            if let Err(error) = outcome {
                tracing::warn!(%error, "car playback request failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{Container, MediaId};

    #[test]
    fn media_ids_round_trip_with_separators_in_keys() {
        let ids = [
            MediaId::Root,
            MediaId::Recent,
            MediaId::Albums,
            MediaId::Artists,
            MediaId::Playlists,
            MediaId::List(Container::Album, "jellyfin/abc/def".into()),
            MediaId::List(Container::Artist, "a/b".into()),
            MediaId::List(Container::Playlist, "pl:1/2".into()),
            MediaId::Track {
                container: Container::Album,
                id: "x/y".into(),
                key: "local:/music/a.flac".into(),
            },
            MediaId::Track {
                container: Container::Artist,
                id: "artist/with/slashes".into(),
                key: "yt/abc".into(),
            },
            MediaId::Track {
                container: Container::Playlist,
                id: "p".into(),
                key: "ключ/ü".into(),
            },
            MediaId::Track {
                container: Container::Playlist,
                id: String::new(),
                key: String::new(),
            },
            MediaId::RecentTrack("local:/music/a.flac".into()),
        ];
        for id in ids {
            assert_eq!(MediaId::parse(&id.encode()), Some(id));
        }
    }

    #[test]
    fn unknown_media_ids_do_not_parse() {
        for raw in [
            "",
            "nope",
            "play/album/x/id",
            "play/album/3",
            "play/album/9/short",
            "play/other/1/x",
            // A length that would split a multi-byte character.
            "play/album/1/üid",
        ] {
            assert_eq!(MediaId::parse(raw), None, "{raw}");
        }
    }
}
