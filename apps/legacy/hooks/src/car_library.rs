//! What Android Auto lists and plays. The car asks for the children of a
//! media id and later plays one it picked, so every id here has to carry enough
//! to rebuild its queue on its own: the car may ask for it long after the list
//! it came from was read.

// Off Android only the tests reach this module.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use api::{AlbumQuery, ApiError, ArtistQuery, KopuzApi, Page, QueueContext, QueueMode, TrackInfo};

pub(crate) const ROOT: &str = "root";
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

/// A parsed media id. The free-form part (an album id, an artist key, a
/// track key) always comes last, because those can hold the separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaId {
    Root,
    Recent,
    Albums,
    Artists,
    Playlists,
    Album(String),
    Artist(String),
    Playlist(String),
    PlayAlbum { index: u32, id: String },
    PlayArtist { index: u32, key: String },
    PlayPlaylist { index: u32, id: String },
    PlayRecent(String),
    PlayTrack(String),
}

impl MediaId {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let simple = match raw {
            ROOT => Some(Self::Root),
            RECENT => Some(Self::Recent),
            ALBUMS => Some(Self::Albums),
            ARTISTS => Some(Self::Artists),
            PLAYLISTS => Some(Self::Playlists),
            _ => None,
        };
        if simple.is_some() {
            return simple;
        }
        let (kind, rest) = raw.split_once('/')?;
        match kind {
            "album" => Some(Self::Album(rest.to_owned())),
            "artist" => Some(Self::Artist(rest.to_owned())),
            "playlist" => Some(Self::Playlist(rest.to_owned())),
            "play" => {
                let (what, rest) = rest.split_once('/')?;
                match what {
                    "recent" => Some(Self::PlayRecent(rest.to_owned())),
                    "track" => Some(Self::PlayTrack(rest.to_owned())),
                    _ => {
                        let (index, id) = rest.split_once('/')?;
                        let index = index.parse().ok()?;
                        let id = id.to_owned();
                        match what {
                            "album" => Some(Self::PlayAlbum { index, id }),
                            "artist" => Some(Self::PlayArtist { index, key: id }),
                            "playlist" => Some(Self::PlayPlaylist { index, id }),
                            _ => None,
                        }
                    }
                }
            }
            _ => None,
        }
    }

    pub(crate) fn encode(&self) -> String {
        match self {
            Self::Root => ROOT.to_owned(),
            Self::Recent => RECENT.to_owned(),
            Self::Albums => ALBUMS.to_owned(),
            Self::Artists => ARTISTS.to_owned(),
            Self::Playlists => PLAYLISTS.to_owned(),
            Self::Album(id) => format!("album/{id}"),
            Self::Artist(key) => format!("artist/{key}"),
            Self::Playlist(id) => format!("playlist/{id}"),
            Self::PlayAlbum { index, id } => format!("play/album/{index}/{id}"),
            Self::PlayArtist { index, key } => format!("play/artist/{index}/{key}"),
            Self::PlayPlaylist { index, id } => format!("play/playlist/{index}/{id}"),
            Self::PlayRecent(key) => format!("play/recent/{key}"),
            Self::PlayTrack(key) => format!("play/track/{key}"),
        }
    }
}

fn list(limit: u32) -> Page {
    Page { offset: 0, limit }
}

fn tracks_in(tracks: &[TrackInfo], id: impl Fn(u32, &TrackInfo) -> MediaId) -> Vec<CarItem> {
    tracks
        .iter()
        .enumerate()
        .map(|(index, track)| CarItem::track(id(index as u32, track).encode(), track))
        .collect()
}

/// The rows under `parent`; an id the car made up, or one from an older
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
        MediaId::Recent => {
            let page = api.recent_tracks(list(RECENT_LIMIT)).await?;
            tracks_in(&page.items, |_, track| {
                MediaId::PlayRecent(track.key.clone())
            })
        }
        MediaId::Albums => api
            .albums(AlbumQuery::default(), list(LIST_LIMIT))
            .await?
            .albums
            .into_iter()
            .map(|album| {
                CarItem::folder(MediaId::Album(album.id).encode(), album.title, album.artist)
            })
            .collect(),
        MediaId::Artists => api
            .artists(ArtistQuery::default(), list(LIST_LIMIT))
            .await?
            .artists
            .into_iter()
            .map(|artist| {
                let subtitle = match artist.album_count {
                    1 => "1 album".to_owned(),
                    count => format!("{count} albums"),
                };
                CarItem::folder(MediaId::Artist(artist.key).encode(), artist.name, subtitle)
            })
            .collect(),
        MediaId::Playlists => api
            .playlists()
            .await?
            .playlists
            .into_iter()
            .map(|playlist| {
                let subtitle = match playlist.track_keys.len() {
                    1 => "1 track".to_owned(),
                    count => format!("{count} tracks"),
                };
                CarItem::folder(
                    MediaId::Playlist(playlist.id).encode(),
                    playlist.name,
                    subtitle,
                )
            })
            .collect(),
        MediaId::Album(id) => {
            let page = api.album_tracks(id.clone(), list(LIST_LIMIT)).await?;
            tracks_in(&page.items, |index, _| MediaId::PlayAlbum {
                index,
                id: id.clone(),
            })
        }
        MediaId::Artist(key) => {
            let page = api.artist_tracks(key.clone(), list(LIST_LIMIT)).await?;
            tracks_in(&page.items, |index, _| MediaId::PlayArtist {
                index,
                key: key.clone(),
            })
        }
        MediaId::Playlist(id) => {
            let tracks = playlist_tracks(api, &id).await?;
            tracks_in(&tracks, |index, _| MediaId::PlayPlaylist {
                index,
                id: id.clone(),
            })
        }
        MediaId::PlayAlbum { .. }
        | MediaId::PlayArtist { .. }
        | MediaId::PlayPlaylist { .. }
        | MediaId::PlayRecent(_)
        | MediaId::PlayTrack(_) => Vec::new(),
    };
    Ok(items)
}

async fn playlist_tracks(api: &dyn KopuzApi, id: &str) -> Result<Vec<TrackInfo>, ApiError> {
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

/// Tracks matching `query`, each played on its own when picked.
pub(crate) async fn search(api: &dyn KopuzApi, query: &str) -> Result<Vec<CarItem>, ApiError> {
    let results = api.search(api::SearchRequest::new(query)).await?;
    Ok(results
        .tracks
        .iter()
        .take(SEARCH_LIMIT)
        .map(|track| CarItem::track(MediaId::PlayTrack(track.key.clone()).encode(), track))
        .collect())
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

/// Replace the queue with what `media_id` names and start it.
pub(crate) async fn play(api: &dyn KopuzApi, media_id: &str) -> Result<(), ApiError> {
    let Some(id) = MediaId::parse(media_id) else {
        tracing::debug!(media_id, "car asked to play an unknown media id");
        return Ok(());
    };
    match id {
        MediaId::PlayAlbum { index, id } => {
            set_queue(api, QueueContext::Album { id }, Some(index)).await
        }
        MediaId::PlayArtist { index, key } => {
            set_queue(api, QueueContext::Artist { artist: key }, Some(index)).await
        }
        MediaId::PlayPlaylist { index, id } => {
            set_queue(api, QueueContext::Playlist { id }, Some(index)).await
        }
        MediaId::Album(id) => set_queue(api, QueueContext::Album { id }, None).await,
        MediaId::Artist(artist) => set_queue(api, QueueContext::Artist { artist }, None).await,
        MediaId::Playlist(id) => set_queue(api, QueueContext::Playlist { id }, None).await,
        // Playing a track moves it to the top of the history, so the list the
        // car showed is read again and the track found by key, not position.
        MediaId::PlayRecent(key) => {
            let page = api.recent_tracks(list(RECENT_LIMIT)).await?;
            let keys: Vec<String> = page.items.into_iter().map(|track| track.key).collect();
            match keys.iter().position(|candidate| *candidate == key) {
                Some(index) => {
                    set_queue(api, QueueContext::Tracks { keys }, Some(index as u32)).await
                }
                None => set_queue(api, QueueContext::Tracks { keys: vec![key] }, None).await,
            }
        }
        MediaId::PlayTrack(key) => {
            set_queue(api, QueueContext::Tracks { keys: vec![key] }, None).await
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

#[cfg(test)]
mod tests {
    use super::MediaId;

    #[test]
    fn media_ids_round_trip_with_separators_in_keys() {
        let ids = [
            MediaId::Root,
            MediaId::Recent,
            MediaId::Albums,
            MediaId::Artists,
            MediaId::Playlists,
            MediaId::Album("jellyfin/abc/def".into()),
            MediaId::Artist("a/b".into()),
            MediaId::Playlist("pl:1/2".into()),
            MediaId::PlayAlbum {
                index: 3,
                id: "x/y".into(),
            },
            MediaId::PlayArtist {
                index: 0,
                key: "artist/with/slashes".into(),
            },
            MediaId::PlayPlaylist {
                index: 12,
                id: "p".into(),
            },
            MediaId::PlayRecent("local:/music/a.flac".into()),
            MediaId::PlayTrack("yt/abc".into()),
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
            "play/other/1/x",
        ] {
            assert_eq!(MediaId::parse(raw), None, "{raw}");
        }
    }
}
