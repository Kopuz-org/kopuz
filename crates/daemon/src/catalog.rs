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
    ApiError, ArtworkRef, ArtworkTarget, CatalogChip, CatalogDetail, CatalogDetailRequest,
    CatalogHeader, CatalogItem, CatalogItemKind, CatalogPage, CatalogShelf, ShelfLayout,
};
use server::ytmusic::discover::{
    BrowsePage, DiscoverHome, DiscoverItem, DiscoverShelf, LinkKind, PageHeader,
};

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

pub(crate) fn source_error(error: server::source::SourceError) -> ApiError {
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

    /// Convert a browse page `listed` served, registering its songs and tile images so later requests resolve them.
    fn page(
        &self,
        home: DiscoverHome,
        listed: &config::Source,
        config: &config::AppConfig,
    ) -> CatalogPage {
        CatalogPage {
            shelves: self.shelves(home.shelves, listed, config),
            continuation: home.continuation,
        }
    }

    fn shelves(
        &self,
        mut shelves: Vec<DiscoverShelf>,
        listed: &config::Source,
        config: &config::AppConfig,
    ) -> Vec<CatalogShelf> {
        let songs = shelves
            .iter_mut()
            .flat_map(|shelf| shelf.items.iter_mut())
            .filter_map(|item| match item {
                DiscoverItem::Song(track)
                | DiscoverItem::Video(track)
                | DiscoverItem::Episode { track, .. } => Some(&mut **track),
                _ => None,
            });
        crate::wire::listed_by(listed, songs);
        let songs: Vec<reader::Track> = shelves
            .iter()
            .flat_map(|shelf| shelf.items.iter())
            .filter_map(|item| match item {
                DiscoverItem::Song(track)
                | DiscoverItem::Video(track)
                | DiscoverItem::Episode { track, .. } => Some((**track).clone()),
                _ => None,
            })
            .collect();
        self.library.register_transient(&songs);
        shelves
            .into_iter()
            .map(|shelf| {
                let layout = layout(shelf.layout);
                let (more_ref, more_kind) = match shelf.more {
                    Some(link) => (Some(link.id), link_kind(link.kind)),
                    None => (None, CatalogItemKind::Unknown),
                };
                CatalogShelf {
                    title: shelf.title,
                    strapline: shelf.strapline,
                    more_ref,
                    more_kind,
                    list: layout == ShelfLayout::List,
                    layout,
                    continuation: shelf.continuation,
                    search_filter: shelf.search_filter.map(str::to_string),
                    items: shelf
                        .items
                        .into_iter()
                        .map(|item| self.item(item, config))
                        .collect(),
                }
            })
            .collect()
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
                accent: None,
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
                accent: None,
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
                accent: None,
            },
            DiscoverItem::Artist {
                channel_id,
                name,
                subtitle,
                thumbnail,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&channel_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Artist,
                id: channel_id,
                title: name,
                subtitle,
                track: None,
                accent: None,
            },
            DiscoverItem::Mood {
                browse_id,
                title,
                thumbnail,
                accent,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&browse_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Mood,
                id: browse_id,
                title,
                subtitle: None,
                track: None,
                accent: accent.map(|argb| format!("#{:06x}", argb & 0x00ff_ffff)),
            },
            DiscoverItem::Video(track) => CatalogItem {
                kind: CatalogItemKind::Video,
                ..self.item(DiscoverItem::Song(track), config)
            },
            DiscoverItem::Episode {
                track,
                browse_id,
                published,
            } => {
                let item = self.item(DiscoverItem::Song(track), config);
                let subtitle: Vec<&str> = [published.as_deref(), item.subtitle.as_deref()]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect();
                CatalogItem {
                    kind: CatalogItemKind::Episode,
                    id: browse_id,
                    subtitle: (!subtitle.is_empty()).then(|| subtitle.join(" • ")),
                    ..item
                }
            }
            DiscoverItem::Podcast {
                browse_id,
                title,
                subtitle,
                thumbnail,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&browse_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Podcast,
                id: browse_id,
                title,
                subtitle: Some(subtitle),
                track: None,
                accent: None,
            },
            DiscoverItem::Page { page_id, title } => CatalogItem {
                kind: CatalogItemKind::Page,
                id: page_id,
                title,
                ..CatalogItem::default()
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
        Ok(self.page(home, source.source(), &config))
    }

    pub async fn detail(&self, request: CatalogDetailRequest) -> Result<CatalogDetail, ApiError> {
        if request.id.is_empty() {
            return Err(ApiError::invalid_input(
                "a catalog entity is opened by its id",
            ));
        }
        let config = self.config();
        let source = self.source();
        match request.kind {
            CatalogItemKind::Album => {
                // An album reached by its own browse id resolves directly; one
                // reached by a library ref needs the lookup first; and a saved
                // album from a source that stores no browse id is found by what
                // it is called, which is why the id alone is enough here.
                let mut album = match source
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
                crate::wire::listed_by(source.source(), &mut album.tracks);
                self.library.register_transient(&album.tracks);
                let artwork = self.remember_thumbnail(&album.browse_id, album.thumbnail.as_deref());
                let artist_key = album.artist_id.as_deref().map(str::to_string);
                Ok(CatalogDetail {
                    kind: CatalogItemKind::Album,
                    id: album.browse_id,
                    title: album.title,
                    artist_key,
                    subtitle: album.artist,
                    artwork,
                    playback_id: album.audio_playlist_id,
                    year: album.year,
                    header: CatalogHeader::Detail,
                    tracks: album
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, &config))
                        .collect(),
                    ..Default::default()
                })
            }
            CatalogItemKind::Playlist => {
                let mut page = source
                    .fetch_playlist_entries_page(&request.id, request.continuation)
                    .await
                    .map_err(source_error)?;
                crate::wire::listed_by(source.source(), &mut page.tracks);
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
                    header: CatalogHeader::Detail,
                    ..Default::default()
                })
            }
            CatalogItemKind::Artist => {
                let key = request.id;
                let filed = crate::artist_row::find(&self.db, source.source(), &key).await?;
                // An artist the library hasn't filed is one the source just listed, by the id it issued.
                let channel_id = match filed {
                    Some(row) => match row.source_id {
                        Some(id) => id,
                        None => return self.library_artist(&key).await,
                    },
                    None => key,
                };
                let artist = source
                    .fetch_artist(&channel_id)
                    .await
                    .map_err(source_error)?;
                if source
                    .name_artist(&channel_id, &artist.name)
                    .await
                    .unwrap_or(false)
                {
                    self.session.invalidate(api::Table::Tracks);
                }
                let page = self.page(
                    DiscoverHome {
                        shelves: artist.sections,
                        continuation: None,
                    },
                    source.source(),
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
                    header: CatalogHeader::Artist,
                    ..Default::default()
                })
            }
            // Each of these is a page of the source's own, opened by the id it handed out.
            CatalogItemKind::Page
            | CatalogItemKind::Mood
            | CatalogItemKind::Podcast
            | CatalogItemKind::Episode => {
                let page = source
                    .browse_page(&request.id, request.continuation.as_deref())
                    .await
                    .map_err(source_error)?;
                Ok(self.browse_detail(request, page, source.source(), &config))
            }
            // A song's own page is what the source relates to it.
            CatalogItemKind::Track | CatalogItemKind::Video => {
                let page = source.related(&request.id).await.map_err(source_error)?;
                Ok(self.browse_detail(request, page, source.source(), &config))
            }
            CatalogItemKind::Unknown => Err(ApiError::unsupported(
                "this catalog kind has no detail page",
            )),
        }
    }

    fn browse_detail(
        &self,
        request: CatalogDetailRequest,
        page: BrowsePage,
        listed: &config::Source,
        config: &config::AppConfig,
    ) -> CatalogDetail {
        let id = request.id;
        let artwork = self.remember_thumbnail(&id, page.thumbnail.as_deref());
        CatalogDetail {
            kind: request.kind,
            title: page.title,
            subtitle: page.subtitle,
            description: page.description,
            artwork,
            playback_id: page.playback_id,
            header: match page.header {
                PageHeader::None => CatalogHeader::None,
                PageHeader::Title => CatalogHeader::Title,
                PageHeader::Detail => CatalogHeader::Detail,
                PageHeader::Artist => CatalogHeader::Artist,
            },
            chips: page
                .chips
                .into_iter()
                .map(|chip| CatalogChip {
                    id: chip.page_id,
                    label: chip.title,
                    selected: chip.selected,
                })
                .collect(),
            shelves: self.shelves(page.shelves, listed, config),
            continuation: page.continuation,
            id,
            ..Default::default()
        }
    }

    /// A filtered search, or more of one. The plain search is the library's.
    pub async fn search(
        &self,
        request: api::SearchRequest,
    ) -> Result<api::SearchResults, ApiError> {
        let config = self.config();
        let source = self.source();
        // A continuation names what it continues, so it needs no filter beside it.
        let filter = request.filter.as_deref().unwrap_or_default();
        let page = source
            .search_shelves(&request.query, filter, request.continuation.as_deref())
            .await
            .map_err(source_error)?;
        Ok(api::SearchResults {
            shelves: self.shelves(page.shelves, source.source(), &config),
            continuation: page.continuation,
            correction: page.correction,
            ..Default::default()
        })
    }

    pub async fn search_suggestions(
        &self,
        query: &str,
    ) -> Result<Vec<api::SearchSuggestion>, ApiError> {
        use server::ytmusic::browse::search::Suggestion;
        let config = self.config();
        let source = self.source();
        let suggestions = source
            .search_suggestions(query)
            .await
            .map_err(source_error)?;
        // Hits go through the same conversion as a shelf, so their songs are
        // registered and their pictures remembered like any browse row's.
        let mut hits = Vec::new();
        let mut order = Vec::new();
        for suggestion in suggestions {
            match suggestion {
                Suggestion::Query { text, from_history } => order.push(Some((text, from_history))),
                Suggestion::Item(item) => {
                    hits.push(item);
                    order.push(None);
                }
            }
        }
        let shelf = DiscoverShelf {
            title: String::new(),
            strapline: None,
            more: None,
            items: hits,
            layout: server::ytmusic::discover::ShelfLayout::List,
            continuation: None,
            search_filter: None,
        };
        let mut hits = self
            .shelves(vec![shelf], source.source(), &config)
            .into_iter()
            .flat_map(|shelf| shelf.items);
        Ok(order
            .into_iter()
            .filter_map(|entry| match entry {
                Some((text, from_history)) => Some(api::SearchSuggestion {
                    text,
                    from_history,
                    item: None,
                }),
                None => hits.next().map(|item| api::SearchSuggestion {
                    text: item.title.clone(),
                    from_history: false,
                    item: Some(item),
                }),
            })
            .collect())
    }

    /// An artist the source issued no id for has no catalog page, so it is the tracks the library files under it.
    async fn library_artist(&self, key: &str) -> Result<CatalogDetail, ApiError> {
        let artist = self.library.artist(key).await?.info;
        let every = api::Page {
            offset: 0,
            limit: u32::MAX,
        };
        let tracks = self.library.artist_tracks(key, every).await?.items;
        Ok(CatalogDetail {
            kind: CatalogItemKind::Artist,
            id: key.to_string(),
            title: artist.name,
            artwork: artist.artwork,
            tracks,
            ..Default::default()
        })
    }

    /// A source's mix seeded by one track, with the seed pinned to the front.
    ///
    /// Sources return the seed somewhere in the list, or not at all; a caller
    /// that asked to start from this track means it should play first.
    pub async fn track_radio(&self, key: &str) -> Result<Vec<reader::Track>, ApiError> {
        let source = self.source();
        let mut tracks = source.start_radio(key).await.map_err(source_error)?;
        crate::wire::listed_by(source.source(), &mut tracks);
        self.library.register_transient(&tracks);
        let seed = match take_seed(&mut tracks, key) {
            Some(seed) => seed,
            None => self.seed_track(key).await?,
        };
        tracks.insert(0, seed);
        Ok(tracks)
    }

    pub async fn playlist_radio(&self, id: &str) -> Result<Vec<reader::Track>, ApiError> {
        let source = self.source();
        let mut tracks = source
            .start_playlist_radio(id)
            .await
            .map_err(source_error)?;
        crate::wire::listed_by(source.source(), &mut tracks);
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

fn layout(layout: server::ytmusic::discover::ShelfLayout) -> ShelfLayout {
    use server::ytmusic::discover::ShelfLayout as Source;
    match layout {
        Source::Carousel => ShelfLayout::Carousel,
        Source::Grid => ShelfLayout::Grid,
        Source::List => ShelfLayout::List,
        Source::TrackGrid => ShelfLayout::TrackGrid,
        Source::Hero => ShelfLayout::Hero,
    }
}

fn link_kind(kind: LinkKind) -> CatalogItemKind {
    match kind {
        LinkKind::Album => CatalogItemKind::Album,
        LinkKind::Artist => CatalogItemKind::Artist,
        LinkKind::Playlist => CatalogItemKind::Playlist,
        LinkKind::Page => CatalogItemKind::Page,
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
            credits: Vec::new(),
            artists: Vec::new(),
            replay_gain: config::ReplayGainInfo::default(),
            counterpart: None,
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
