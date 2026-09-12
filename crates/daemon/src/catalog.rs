//! The source's browse catalog, as the API serves it.
//!
//! Every network call the discover surface used to make from the UI process
//! happens here, and every song it returns is registered with the library, so
//! a client can queue, heart or ask for the artwork of a browse row by key
//! exactly as it would a library track.
//!
//! Tile images are public URLs, but they are not handed over: the daemon
//! remembers them and serves the bytes through `ArtworkApi`, so a frontend has
//! exactly one way to get a picture and one that cannot fetch a URL itself
//! still works.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use api::{
    ApiError, ArtworkRef, ArtworkTarget, CatalogDetail, CatalogDetailRequest, CatalogItem,
    CatalogItemKind, CatalogPage, CatalogShelf,
};
use server::ytmusic::discover::{DiscoverHome, DiscoverItem};

use crate::library::LibraryService;
use crate::session::SessionHandle;

/// Roughly a few pages of browsing; the oldest tiles fall out.
const MAX_REMEMBERED_TILES: usize = 4096;

pub struct CatalogService {
    db: db::Db,
    session: SessionHandle,
    library: Arc<LibraryService>,
    thumbnails: Mutex<Thumbnails>,
}

#[derive(Default)]
struct Thumbnails {
    by_id: HashMap<String, String>,
    order: VecDeque<String>,
}

fn source_error(error: server::source::SourceError) -> ApiError {
    use api::ErrorCode;
    use server::source::SourceError;
    match &error {
        SourceError::Unsupported(what) => ApiError::unsupported(*what),
        SourceError::Auth => ApiError::new(ErrorCode::SourceAuthExpired, error.to_string()),
        SourceError::Connectivity => ApiError::new(ErrorCode::SourceUnreachable, error.to_string()),
        SourceError::InvalidInput(message) => ApiError::invalid_input(message.clone()),
        SourceError::Backend(message) => ApiError::internal(message.clone()),
    }
}

impl CatalogService {
    pub fn new(db: db::Db, session: SessionHandle, library: Arc<LibraryService>) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            library,
            thumbnails: Mutex::new(Thumbnails::default()),
        })
    }

    fn config(&self) -> config::AppConfig {
        self.session.config_watch().borrow().clone()
    }

    fn source(&self) -> server::source::ActiveSource {
        Arc::from(server::source::active(self.db.clone(), &self.config()))
    }

    /// The image for a tile, if one was remembered from a page this session
    /// served. Used by the artwork service, which is why it is public.
    pub fn thumbnail(&self, id: &str) -> Option<String> {
        self.thumbnails.lock().ok()?.by_id.get(id).cloned()
    }

    fn remember_thumbnail(&self, id: &str, url: Option<&str>) -> Option<ArtworkRef> {
        let url = url.filter(|url| !url.is_empty())?;
        let mut cache = self.thumbnails.lock().ok()?;
        cache.order.retain(|saved| saved != id);
        cache.order.push_back(id.to_string());
        cache.by_id.insert(id.to_string(), url.to_string());
        while cache.order.len() > MAX_REMEMBERED_TILES {
            if let Some(oldest) = cache.order.pop_front() {
                cache.by_id.remove(&oldest);
            }
        }
        Some(crate::artwork::url_ref(
            ArtworkTarget::Catalog(id.to_string()),
            url,
        ))
    }

    /// Convert a source's browse page, registering the songs so their keys
    /// resolve later and the tile images so their bytes can be served.
    fn page(&self, home: DiscoverHome, config: &config::AppConfig) -> CatalogPage {
        let songs: Vec<reader::Track> = home
            .shelves
            .iter()
            .flat_map(|shelf| shelf.items.iter())
            .filter_map(|item| match item {
                DiscoverItem::Song(track) => Some((**track).clone()),
                _ => None,
            })
            .collect();
        self.library.register_transient(&songs);
        CatalogPage {
            shelves: home
                .shelves
                .into_iter()
                .map(|shelf| CatalogShelf {
                    title: shelf.title,
                    strapline: shelf.strapline,
                    more_ref: shelf.more_browse_id,
                    list: shelf.is_song_list,
                    items: shelf
                        .items
                        .into_iter()
                        .map(|item| self.item(item, config))
                        .collect(),
                })
                .collect(),
            continuation: home.continuation,
        }
    }

    fn item(&self, item: DiscoverItem, config: &config::AppConfig) -> CatalogItem {
        match item {
            // A song's artwork is the track's own, so the tile and the queue
            // row cannot disagree about which picture belongs to it.
            DiscoverItem::Song(track) => CatalogItem {
                kind: CatalogItemKind::Track,
                id: track.id.key().into_owned(),
                title: track.title.clone(),
                subtitle: Some(track.artist.clone()),
                artwork: crate::artwork::track_ref(&track),
                track: Some(crate::wire::track_info(&track, config)),
            },
            DiscoverItem::Playlist {
                playlist_id,
                title,
                subtitle,
                thumbnail,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&playlist_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Playlist,
                id: playlist_id,
                title,
                subtitle: Some(subtitle),
                track: None,
            },
            DiscoverItem::Album {
                browse_id,
                title,
                subtitle,
                thumbnail,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&browse_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Album,
                id: browse_id,
                title,
                subtitle: Some(subtitle),
                track: None,
            },
            DiscoverItem::Artist {
                channel_id,
                name,
                thumbnail,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&channel_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Artist,
                id: channel_id,
                title: name,
                subtitle: None,
                track: None,
            },
            DiscoverItem::Mood {
                browse_id,
                title,
                thumbnail,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&browse_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Mood,
                id: browse_id,
                title,
                subtitle: None,
                track: None,
            },
        }
    }

    pub async fn catalog(&self, continuation: Option<&str>) -> Result<CatalogPage, ApiError> {
        let config = self.config();
        let source = self.source();
        let home = match continuation {
            Some(token) => source.discover_continuation(token).await,
            None => source.discover_home().await,
        }
        .map_err(source_error)?;
        Ok(self.page(home, &config))
    }

    pub async fn detail(&self, request: CatalogDetailRequest) -> Result<CatalogDetail, ApiError> {
        if request.id.trim().is_empty() {
            return Err(ApiError::invalid_input("catalog id is required"));
        }
        let config = self.config();
        let source = self.source();
        match request.kind {
            CatalogItemKind::Album => {
                // An album reached by its own browse id resolves directly; one
                // reached by a library ref needs the lookup first; and a saved
                // album from a source that stores no browse id is found by what
                // it is called, which is why the id alone is enough here.
                let album = match source
                    .fetch_album_by_ref(&request.id)
                    .await
                    .map_err(source_error)?
                {
                    Some(album) => album,
                    None => match self.saved_album(&request.id).await {
                        Some((title, artist)) => match source
                            .fetch_album_by_meta(&title, &artist)
                            .await
                            .map_err(source_error)?
                        {
                            Some(album) => album,
                            None => return Err(ApiError::not_found("no such catalog album")),
                        },
                        None => source
                            .fetch_album(&request.id)
                            .await
                            .map_err(source_error)?,
                    },
                };
                self.library.register_transient(&album.tracks);
                let artwork = self.remember_thumbnail(&album.browse_id, album.thumbnail.as_deref());
                Ok(CatalogDetail {
                    kind: CatalogItemKind::Album,
                    id: album.browse_id,
                    title: album.title,
                    subtitle: album.artist,
                    artwork,
                    playback_id: album.audio_playlist_id,
                    year: album.year,
                    tracks: album
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, &config))
                        .collect(),
                    ..Default::default()
                })
            }
            CatalogItemKind::Playlist => {
                let page = source
                    .fetch_playlist_entries_page(&request.id, request.continuation)
                    .await
                    .map_err(source_error)?;
                self.library.register_transient(&page.tracks);
                let artwork = self
                    .thumbnail(&request.id)
                    .map(|url| {
                        crate::artwork::url_ref(ArtworkTarget::Catalog(request.id.clone()), &url)
                    })
                    .or_else(|| page.tracks.first().and_then(crate::artwork::track_ref));
                Ok(CatalogDetail {
                    kind: CatalogItemKind::Playlist,
                    id: request.id.clone(),
                    title: request.id,
                    artwork,
                    tracks: page
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, &config))
                        .collect(),
                    continuation: page.next,
                    ..Default::default()
                })
            }
            CatalogItemKind::Artist => {
                let channel_id = if request.id.starts_with("UC") {
                    request.id
                } else {
                    source
                        .resolve_artist_channel_id(request.id.trim())
                        .await
                        .map_err(source_error)?
                        .ok_or_else(|| ApiError::not_found("catalog artist not found"))?
                };
                let artist = source
                    .fetch_artist(&channel_id)
                    .await
                    .map_err(source_error)?;
                let page = self.page(
                    DiscoverHome {
                        shelves: artist.sections,
                        continuation: None,
                    },
                    &config,
                );
                let artwork =
                    self.remember_thumbnail(&artist.channel_id, artist.banner_thumbnail.as_deref());
                Ok(CatalogDetail {
                    kind: CatalogItemKind::Artist,
                    id: artist.channel_id,
                    title: artist.name,
                    subtitle: artist.subscribers,
                    description: artist.description,
                    artwork,
                    playback_id: artist.shuffle_playlist_id,
                    shelves: page.shelves,
                    continuation: page.continuation,
                    ..Default::default()
                })
            }
            CatalogItemKind::Track | CatalogItemKind::Mood | CatalogItemKind::Unknown => Err(
                ApiError::unsupported("this catalog kind has no detail page"),
            ),
        }
    }

    /// A source's mix seeded by one track, with the seed pinned to the front.
    ///
    /// Sources return the seed somewhere in the list, or not at all; a caller
    /// that asked to start from this track means it should play first.
    pub async fn track_radio(&self, key: &str) -> Result<Vec<reader::Track>, ApiError> {
        let mut tracks = self.source().start_radio(key).await.map_err(source_error)?;
        self.library.register_transient(&tracks);
        let seed = match take_seed(&mut tracks, key) {
            Some(seed) => seed,
            None => self.seed_track(key).await?,
        };
        tracks.insert(0, seed);
        Ok(tracks)
    }

    pub async fn playlist_radio(&self, id: &str) -> Result<Vec<reader::Track>, ApiError> {
        let tracks = self
            .source()
            .start_playlist_radio(id)
            .await
            .map_err(source_error)?;
        self.library.register_transient(&tracks);
        Ok(tracks)
    }

    /// The title and artist of a saved album, for a source whose albums are
    /// stored without a browse id and can only be looked up by name.
    async fn saved_album(&self, id: &str) -> Option<(String, String)> {
        let album = self.library.album(id).await.ok().flatten()?;
        (!album.title.trim().is_empty()).then_some((album.title, album.artist))
    }

    /// The seed itself, when the mix did not include it.
    async fn seed_track(&self, key: &str) -> Result<reader::Track, ApiError> {
        if let Some(track) = self
            .db
            .tracks_by_keys(&self.config().active_source, &[key.to_string()])
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?
            .into_iter()
            .next()
        {
            return Ok(track);
        }
        self.library
            .transient_track(key)
            .ok_or_else(|| ApiError::not_found("unknown radio seed"))
    }
}

/// Lift the seed out of a mix, if the source put it there. Removing rather
/// than copying is what keeps it from appearing twice once it is pinned.
fn take_seed(tracks: &mut Vec<reader::Track>, key: &str) -> Option<reader::Track> {
    let index = tracks.iter().position(|track| track.id.key() == key)?;
    Some(tracks.remove(index))
}

#[cfg(test)]
mod tests {
    use super::take_seed;

    fn track(key: &str) -> reader::Track {
        reader::Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(key)),
            cover: None,
            album_id: String::new(),
            title: key.to_string(),
            artist: String::new(),
            album: String::new(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: Vec::new(),
        }
    }

    #[test]
    fn the_seed_leaves_the_mix_so_pinning_it_cannot_duplicate_it() {
        let mut mix = vec![track("a"), track("seed"), track("b")];

        let seed = take_seed(&mut mix, "seed").expect("the mix held the seed");

        assert_eq!(seed.title, "seed");
        assert_eq!(
            mix.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["a", "b"],
            "the rest of the mix keeps the order the source gave it"
        );
    }

    #[test]
    fn a_mix_without_the_seed_says_so_rather_than_picking_one() {
        let mut mix = vec![track("a"), track("b")];

        assert!(take_seed(&mut mix, "seed").is_none());
        assert_eq!(mix.len(), 2, "nothing was taken");
    }
}
