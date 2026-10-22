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

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use api::{
    ApiError, ArtworkRef, ArtworkTarget, CatalogActions, CatalogChip, CatalogDetail,
    CatalogDetailRequest, CatalogHeader, CatalogItem, CatalogItemKind, CatalogPage, CatalogShelf,
    DiscoverRows, ForYou, ForYouUnavailable, ShelfLayout, Table,
};
use rand::seq::SliceRandom;
use server::source::RadioPage;
use server::ytmusic::discover::{
    BrowsePage, DiscoverHome, DiscoverItem, DiscoverShelf, ItemActions, LinkKind, PageHeader,
};
use tokio::sync::OnceCell;

use server::source::MediaSource;

use crate::library::LibraryService;
use crate::session::SessionHandle;
use crate::sources::source_error;

/// Roughly a few pages of browsing; the oldest tiles fall out.
const MAX_REMEMBERED_TILES: usize = 4096;

/// How many liked tracks seed the "For you" mix.
const FOR_YOU_SEEDS: usize = 5;
/// How many songs from the top of each seed's radio join it.
const FOR_YOU_PER_RADIO: usize = 3;

/// How many tiles a Discover row picks from the library.
const DISCOVER_ROW_LIMIT: usize = 24;
/// A favorite neither played nor liked for this long is forgotten.
const FORGOTTEN_AFTER: Duration = Duration::from_secs(90 * 24 * 60 * 60);
/// How many of the most played tracks count as favorites beside the liked ones.
const MOST_PLAYED: usize = 100;

/// One build per source, shared by every caller that asks while it runs.
type Build<T> = Arc<OnceCell<Result<T, ApiError>>>;
type Builds<T> = Mutex<HashMap<config::Source, Build<T>>>;
type ForYouMixes = Builds<Vec<reader::Track>>;
type DiscoverPicks = Builds<DiscoverPick>;

/// A source's Discover rows as the source and the library gave them, made wire rows on each ask.
#[derive(Debug, Clone, Default)]
struct DiscoverPick {
    mixes: Vec<DiscoverItem>,
    never_played: Vec<reader::Album>,
    forgotten: Vec<reader::Track>,
}

pub struct CatalogService {
    db: db::Db,
    session: SessionHandle,
    library: Arc<LibraryService>,
    thumbnails: Mutex<Thumbnails>,
    for_you: ForYouMixes,
    discover: DiscoverPicks,
}

#[derive(Default)]
struct Thumbnails {
    by_id: HashMap<String, String>,
    order: VecDeque<String>,
}

impl CatalogService {
    pub fn new(db: db::Db, session: SessionHandle, library: Arc<LibraryService>) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            library,
            thumbnails: Mutex::new(Thumbnails::default()),
            for_you: Mutex::default(),
            discover: Mutex::default(),
        })
    }

    /// The active source to read the catalog from; offline mode on a source across the internet refuses every such read.
    fn source(&self) -> Result<server::source::ActiveSource, ApiError> {
        if self.library.offline_reads() {
            return Err(offline_refusal());
        }
        Ok(Arc::from(server::source::active(
            self.db.clone(),
            &self.session.sources(),
        )))
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

    /// Convert a browse page `source` served, registering its songs and tile images so later requests resolve them.
    fn page(&self, home: DiscoverHome, source: &dyn MediaSource) -> CatalogPage {
        CatalogPage {
            shelves: self.shelves(home.shelves, source),
            continuation: home.continuation,
        }
    }

    fn shelves(
        &self,
        mut shelves: Vec<DiscoverShelf>,
        source: &dyn MediaSource,
    ) -> Vec<CatalogShelf> {
        let songs = shelves
            .iter_mut()
            .flat_map(|shelf| shelf.items.iter_mut())
            .filter_map(|item| match item {
                DiscoverItem::Song(track, _)
                | DiscoverItem::Video(track, _)
                | DiscoverItem::Episode { track, .. } => Some(&mut **track),
                _ => None,
            });
        crate::wire::listed_by(source.source(), songs);
        let songs: Vec<reader::Track> = shelves
            .iter()
            .flat_map(|shelf| shelf.items.iter())
            .filter_map(|item| match item {
                DiscoverItem::Song(track, _)
                | DiscoverItem::Video(track, _)
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
                    layout,
                    continuation: shelf.continuation,
                    search_filter: shelf.search_filter.map(str::to_string),
                    items: shelf
                        .items
                        .into_iter()
                        .map(|item| self.item(item, source))
                        .collect(),
                }
            })
            .collect()
    }

    fn item(&self, item: DiscoverItem, source: &dyn MediaSource) -> CatalogItem {
        match item {
            // A song's artwork is the track's own, so the tile and the queue
            // row cannot disagree about which picture belongs to it.
            DiscoverItem::Song(track, item_actions) => CatalogItem {
                kind: CatalogItemKind::Track,
                id: track.id.key().into_owned(),
                title: track.title.clone(),
                subtitle: Some(track.artist.clone()),
                artwork: crate::artwork::track_ref(&track),
                explicit: track.explicit,
                web_url: source.web_url(&track),
                track: Some(crate::wire::track_info(&track, self.session.offline())),
                accent: None,
                actions: actions(item_actions),
            },
            DiscoverItem::Playlist {
                playlist_id,
                title,
                subtitle,
                thumbnail,
                actions: item_actions,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&playlist_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Playlist,
                web_url: source.playlist_web_url(&playlist_id),
                id: playlist_id,
                title,
                subtitle: Some(subtitle),
                track: None,
                accent: None,
                actions: actions(item_actions),
                ..CatalogItem::default()
            },
            DiscoverItem::Album {
                browse_id,
                title,
                subtitle,
                thumbnail,
                actions: item_actions,
                explicit,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&browse_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Album,
                web_url: source.album_web_url(&browse_id),
                id: browse_id,
                title,
                subtitle: Some(subtitle),
                track: None,
                accent: None,
                actions: actions(item_actions),
                explicit,
            },
            DiscoverItem::Artist {
                channel_id,
                name,
                subtitle,
                thumbnail,
                actions: item_actions,
            } => CatalogItem {
                artwork: self.remember_thumbnail(&channel_id, thumbnail.as_deref()),
                kind: CatalogItemKind::Artist,
                web_url: source.artist_web_url(&channel_id),
                id: channel_id,
                title: name,
                subtitle,
                track: None,
                accent: None,
                actions: actions(item_actions),
                ..CatalogItem::default()
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
                accent: accent.map(|argb| format!("#{:06x}", argb & 0x00ff_ffff)),
                actions: CatalogActions::default(),
                ..CatalogItem::default()
            },
            DiscoverItem::Video(track, item_actions) => CatalogItem {
                kind: CatalogItemKind::Video,
                ..self.item(DiscoverItem::Song(track, item_actions), source)
            },
            DiscoverItem::Episode {
                track,
                browse_id,
                published,
            } => {
                let item = self.item(DiscoverItem::Song(track, ItemActions::default()), source);
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
                web_url: source.album_web_url(&browse_id),
                id: browse_id,
                title,
                subtitle: Some(subtitle),
                track: None,
                accent: None,
                actions: CatalogActions::default(),
                ..CatalogItem::default()
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
        let source = self.source()?;
        let home = match continuation {
            Some(token) => source.discover_continuation(token).await,
            None => source.discover_home().await,
        }
        .map_err(source_error)?;
        Ok(self.page(home, &*source))
    }

    pub async fn detail(&self, request: CatalogDetailRequest) -> Result<CatalogDetail, ApiError> {
        if request.id.is_empty() {
            return Err(ApiError::invalid_input(
                "a catalog entity is opened by its id",
            ));
        }
        if request.kind == CatalogItemKind::Artist && self.library.offline_reads() {
            return self.offline_artist(&request.id).await;
        }
        let offline = self.session.offline();
        let source = self.source()?;
        match request.kind {
            CatalogItemKind::Album => {
                let album = self.remote_album(&*source, &request.id).await?;
                let artwork = self.remember_thumbnail(&album.browse_id, album.thumbnail.as_deref());
                let artist_key = album.artist_id.as_deref().map(str::to_string);
                Ok(CatalogDetail {
                    kind: CatalogItemKind::Album,
                    // A source with albums but no album pages shares the first track's.
                    web_url: source
                        .album_web_url(&album.browse_id)
                        .or_else(|| album.tracks.first().and_then(|track| source.web_url(track))),
                    id: album.browse_id,
                    title: album.title,
                    artist_key,
                    subtitle: album.artist,
                    description: album.description,
                    artwork,
                    playback_id: album.audio_playlist_id,
                    year: album.year,
                    header: CatalogHeader::Detail,
                    actions: actions(album.actions),
                    release_kind: album.kind.map(|kind| match kind {
                        server::source::ReleaseKind::Album => api::ReleaseKind::Album,
                        server::source::ReleaseKind::Single => api::ReleaseKind::Single,
                        server::source::ReleaseKind::Ep => api::ReleaseKind::Ep,
                        server::source::ReleaseKind::Audiobook => api::ReleaseKind::Audiobook,
                    }),
                    tracks: album
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, offline))
                        .collect(),
                    ..Default::default()
                })
            }
            CatalogItemKind::Playlist => {
                let page = self
                    .playlist_page(&*source, &request.id, request.continuation)
                    .await?;
                let header = page.header.unwrap_or_default();
                let artwork = self
                    .remember_thumbnail(&request.id, header.thumbnail.as_deref())
                    .or_else(|| {
                        self.thumbnail(&request.id).map(|url| {
                            crate::artwork::url_ref(
                                ArtworkTarget::Catalog(request.id.clone()),
                                &url,
                            )
                        })
                    })
                    .or_else(|| page.tracks.first().and_then(crate::artwork::track_ref));
                let title = match header.title.trim() {
                    "" => request.id.clone(),
                    _ => header.title,
                };
                Ok(CatalogDetail {
                    kind: CatalogItemKind::Playlist,
                    web_url: source.playlist_web_url(&request.id),
                    id: request.id,
                    title,
                    owner: header.owner,
                    description: header.description,
                    plays: header.plays,
                    actions: actions(header.actions),
                    privacy: header.privacy.map(privacy),
                    artwork,
                    tracks: page
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, offline))
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
                    self.session.invalidate(api::Table::Artists);
                    self.session.invalidate(api::Table::Tracks);
                }
                let page = self.page(
                    DiscoverHome {
                        shelves: artist.sections,
                        continuation: None,
                    },
                    &*source,
                );
                let artwork =
                    self.remember_thumbnail(&artist.channel_id, artist.banner_thumbnail.as_deref());
                Ok(CatalogDetail {
                    kind: CatalogItemKind::Artist,
                    web_url: source.artist_web_url(&artist.channel_id),
                    id: artist.channel_id,
                    title: artist.name,
                    subtitle: artist.subscribers,
                    monthly_listeners: artist.monthly_listeners,
                    description: artist.description,
                    artwork,
                    shuffle_radio: artist.shuffle_ref,
                    mix_radio: artist.mix_ref,
                    shelves: page.shelves,
                    continuation: page.continuation,
                    header: CatalogHeader::Artist,
                    actions: actions(artist.actions),
                    ..Default::default()
                })
            }
            CatalogItemKind::Page
            | CatalogItemKind::Mood
            | CatalogItemKind::Podcast
            | CatalogItemKind::Episode => {
                let page = source
                    .browse_page(&request.id, request.continuation.as_deref())
                    .await
                    .map_err(source_error)?;
                Ok(self.browse_detail(request, page, &*source))
            }
            // A song's own page is what the source relates to it.
            CatalogItemKind::Track | CatalogItemKind::Video => {
                let page = source.related(&request.id).await.map_err(source_error)?;
                Ok(self.browse_detail(request, page, &*source))
            }
            CatalogItemKind::Unknown => Err(ApiError::unsupported(
                "this catalog kind has no detail page",
            )),
        }
    }

    /// A saved album from a source that stores no browse id is found by name,
    /// which is why the id alone is enough here.
    async fn remote_album(
        &self,
        source: &dyn MediaSource,
        id: &str,
    ) -> Result<server::source::RemoteAlbum, ApiError> {
        let mut album = match source.fetch_album_by_ref(id).await.map_err(source_error)? {
            Some(album) => album,
            None => source.fetch_album(id).await.map_err(source_error)?,
        };
        crate::wire::listed_by(source.source(), &mut album.tracks);
        self.library.register_transient(&album.tracks);
        Ok(album)
    }

    async fn playlist_page(
        &self,
        source: &dyn MediaSource,
        id: &str,
        continuation: Option<String>,
    ) -> Result<server::source::PlaylistPage, ApiError> {
        let mut page = source
            .fetch_playlist_entries_page(id, continuation)
            .await
            .map_err(source_error)?;
        crate::wire::listed_by(source.source(), &mut page.tracks);
        self.library.register_transient(&page.tracks);
        Ok(page)
    }

    pub async fn album_tracks(&self, id: &str) -> Result<Vec<reader::Track>, ApiError> {
        let tracks = self.remote_album(&*self.source()?, id).await?.tracks;
        match tracks.is_empty() {
            true => Err(ApiError::not_found("that album has no tracks to play")),
            false => Ok(tracks),
        }
    }

    /// Pages repeat rows at their seams, so a page with nothing new ends the
    /// walk even when it hands out another token.
    pub async fn playlist_tracks(&self, id: &str) -> Result<Vec<reader::Track>, ApiError> {
        let source = self.source()?;
        let mut seen = std::collections::HashSet::new();
        let mut tracks = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.playlist_page(&*source, id, cursor).await?;
            let before = tracks.len();
            tracks.extend(
                page.tracks
                    .into_iter()
                    .filter(|track| seen.insert(track.id.key().into_owned())),
            );
            match page.next {
                Some(next) if tracks.len() > before => cursor = Some(next),
                _ => break,
            }
        }
        match tracks.is_empty() {
            true => Err(ApiError::not_found("that playlist has no tracks to play")),
            false => Ok(tracks),
        }
    }

    /// Looks up a key no listing this session served.
    pub async fn track(&self, key: &str) -> Result<Option<reader::Track>, ApiError> {
        let source = self.source()?;
        let Some(mut track) = source.fetch_track(key).await.map_err(source_error)? else {
            return Ok(None);
        };
        crate::wire::listed_by(source.source(), [&mut track]);
        self.library
            .register_transient(std::slice::from_ref(&track));
        Ok(Some(track))
    }

    fn browse_detail(
        &self,
        request: CatalogDetailRequest,
        page: BrowsePage,
        source: &dyn MediaSource,
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
            web_url: match request.kind {
                CatalogItemKind::Podcast => source.album_web_url(&id),
                _ => None,
            },
            owner: page.owner,
            plays: page.plays,
            shelves: self.shelves(page.shelves, source),
            continuation: page.continuation,
            actions: actions(page.actions),
            privacy: page.privacy.map(privacy),
            id,
            ..Default::default()
        }
    }

    /// Filtered search only; the plain search is the library's.
    pub async fn search(
        &self,
        request: api::SearchRequest,
    ) -> Result<api::SearchResults, ApiError> {
        let source = self.source()?;
        // A continuation page is parsed as its filter's kind, so it travels with it.
        let filter = request
            .filter
            .as_deref()
            .ok_or_else(|| ApiError::invalid_input("a search continuation needs its filter"))?;
        let page = source
            .search_shelves(&request.query, filter, request.continuation.as_deref())
            .await
            .map_err(source_error)?;
        Ok(api::SearchResults {
            shelves: self.shelves(page.shelves, &*source),
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
        let source = self.source()?;
        let suggestions = source
            .search_suggestions(query)
            .await
            .map_err(source_error)?;
        // Through the shelf conversion, so their songs are registered like any browse row's.
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
            .shelves(vec![shelf], &*source)
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

    /// In offline mode an artist is the library's page of the songs with a copy here, and one the library does not file has none.
    async fn offline_artist(&self, key: &str) -> Result<CatalogDetail, ApiError> {
        let source = self.session.sources().active.clone();
        match crate::artist_row::find(&self.db, &source, key).await? {
            Some(row) => self.library_artist(&row.key).await,
            None => Err(offline_refusal()),
        }
    }

    pub async fn follow(&self, artist_ref: &str, follow: bool) -> Result<(), ApiError> {
        if artist_ref.trim().is_empty() {
            return Err(ApiError::invalid_input("no artist to follow"));
        }
        // Follow state is read fresh from the source, so no table to invalidate.
        let source = self.source()?;
        if !source.capabilities().library_actions.follow {
            return Err(ApiError::unsupported("following"));
        }
        source
            .follow(artist_ref, follow)
            .await
            .map_err(source_error)
    }

    pub async fn save(&self, item_ref: &str, saved: bool) -> Result<(), ApiError> {
        if item_ref.trim().is_empty() {
            return Err(ApiError::invalid_input("nothing to save"));
        }
        let source = self.source()?;
        if !source.capabilities().library_actions.save {
            return Err(ApiError::unsupported("saving to the library"));
        }
        source.save(item_ref, saved).await.map_err(source_error)?;
        // A saved album or playlist joins the source's library listing.
        self.session.invalidate(Table::Albums);
        self.session.invalidate(Table::Playlists);
        Ok(())
    }

    pub async fn remove_from_history(&self, token: &str) -> Result<(), ApiError> {
        if token.trim().is_empty() {
            return Err(ApiError::invalid_input("no history token"));
        }
        let source = self.source()?;
        if !source.capabilities().library_actions.remove_from_history {
            return Err(ApiError::unsupported("history removal"));
        }
        source
            .remove_from_history(token)
            .await
            .map_err(source_error)?;
        self.session.invalidate(Table::Recents);
        Ok(())
    }

    /// A source's mix seeded by one track, with the seed pinned to the front.
    ///
    /// Sources return the seed somewhere in the list, or not at all; a caller
    /// that asked to start from this track means it should play first.
    pub async fn track_radio(&self, key: &str) -> Result<RadioPage, ApiError> {
        let source = self.source()?;
        let mut page = source.start_radio(key).await.map_err(source_error)?;
        crate::wire::listed_by(source.source(), &mut page.tracks);
        self.library.register_transient(&page.tracks);
        let seed = match take_seed(&mut page.tracks, key) {
            Some(seed) => seed,
            None => self.seed_track(key).await?,
        };
        page.tracks.insert(0, seed);
        Ok(page)
    }

    pub async fn playlist_radio(&self, id: &str) -> Result<RadioPage, ApiError> {
        let source = self.source()?;
        let mut page = source
            .start_playlist_radio(id)
            .await
            .map_err(source_error)?;
        crate::wire::listed_by(source.source(), &mut page.tracks);
        self.library.register_transient(&page.tracks);
        Ok(page)
    }

    pub async fn more_radio(&self, cursor: &str) -> Result<RadioPage, ApiError> {
        let source = self.source()?;
        let mut page = source.more_radio(cursor).await.map_err(source_error)?;
        crate::wire::listed_by(source.source(), &mut page.tracks);
        self.library.register_transient(&page.tracks);
        Ok(page)
    }

    /// The seed itself, when the mix did not include it.
    async fn seed_track(&self, key: &str) -> Result<reader::Track, ApiError> {
        if let Some(track) = self
            .db
            .tracks_by_keys(&self.session.sources().active, &[key.to_string()])
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

    pub async fn for_you(&self) -> Result<ForYou, ApiError> {
        self.for_you_mix(false).await
    }

    pub async fn refresh_for_you(&self) -> Result<ForYou, ApiError> {
        self.for_you_mix(true).await
    }

    async fn for_you_mix(&self, fresh: bool) -> Result<ForYou, ApiError> {
        let source = self.source()?;
        let track_radio = source.capabilities().radio.track;
        let likes = match track_radio {
            true => source.favorites().await.map_err(source_error)?,
            false => Vec::new(),
        };
        if let Some(reason) = for_you_unavailable(track_radio, &likes) {
            return Ok(ForYou::Unavailable(reason));
        }
        let build = held_build(&self.for_you, source.source(), fresh);
        let built = build
            .get_or_init(|| {
                mix_for_you(likes, |seed| {
                    let source = &source;
                    async move { self.radio_songs(source.as_ref(), &seed).await }
                })
            })
            .await
            .clone();
        match built {
            Ok(tracks) => {
                self.library.register_transient(&tracks);
                let offline = self.session.offline();
                Ok(ForYou::Mix(
                    tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, offline))
                        .collect(),
                ))
            }
            Err(error) => {
                forget_failed(&self.for_you, source.source(), &build);
                Err(error)
            }
        }
    }

    pub async fn discover_rows(&self) -> Result<DiscoverRows, ApiError> {
        self.discover(false).await
    }

    pub async fn refresh_discover_rows(&self) -> Result<DiscoverRows, ApiError> {
        self.discover(true).await
    }

    async fn discover(&self, fresh: bool) -> Result<DiscoverRows, ApiError> {
        let source = self.source()?;
        let build = held_build(&self.discover, source.source(), fresh);
        let built = build
            .get_or_init(|| pick_discover(source.as_ref(), &self.db, unix_now()))
            .await
            .clone();
        match built {
            Ok(pick) => {
                let offline = self.session.offline();
                Ok(DiscoverRows {
                    mixes: pick
                        .mixes
                        .into_iter()
                        .map(|item| self.item(item, source.as_ref()))
                        .collect(),
                    never_played: pick
                        .never_played
                        .iter()
                        .map(crate::library::album_info)
                        .collect(),
                    forgotten_favorites: pick
                        .forgotten
                        .iter()
                        .map(|track| crate::wire::track_info(track, offline))
                        .collect(),
                })
            }
            Err(error) => {
                forget_failed(&self.discover, source.source(), &build);
                Err(error)
            }
        }
    }

    /// A seed's radio as its source ranks it; unlike [`Self::track_radio`] the seed is not pinned, since the mix drops it.
    async fn radio_songs(
        &self,
        source: &dyn MediaSource,
        seed: &str,
    ) -> Result<Vec<reader::Track>, ApiError> {
        let mut page = source.start_radio(seed).await.map_err(source_error)?;
        crate::wire::listed_by(source.source(), &mut page.tracks);
        Ok(page.tracks)
    }
}

fn offline_refusal() -> ApiError {
    ApiError::new(
        api::ErrorCode::SourceUnreachable,
        "offline mode is on, and this needs the source across the internet",
    )
    .because(api::ErrorReason::OfflineMode)
}

fn for_you_unavailable(track_radio: bool, likes: &[String]) -> Option<ForYouUnavailable> {
    match (track_radio, likes.is_empty()) {
        (false, _) => Some(ForYouUnavailable::NoTrackRadio),
        (true, true) => Some(ForYouUnavailable::NoLikes),
        (true, false) => None,
    }
}

/// The build a request for `source`'s rows joins: the held one, or a new one when there is none or `fresh` asks to drop it.
fn held_build<T>(builds: &Builds<T>, source: &config::Source, fresh: bool) -> Build<T> {
    let mut builds = builds.lock().unwrap_or_else(PoisonError::into_inner);
    if fresh {
        builds.remove(source);
    }
    builds.entry(source.clone()).or_default().clone()
}

/// A failed build is not one to keep: the next request tries again, unless a refresh already replaced it.
fn forget_failed<T>(builds: &Builds<T>, source: &config::Source, build: &Build<T>) {
    let mut builds = builds.lock().unwrap_or_else(PoisonError::into_inner);
    if builds
        .get(source)
        .is_some_and(|held| Arc::ptr_eq(held, build))
    {
        builds.remove(source);
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

/// The source's mixes, and where kopuz's play counts are its whole history, a pick of unplayed albums and forgotten favorites as of `now`.
async fn pick_discover(
    source: &dyn MediaSource,
    db: &db::Db,
    now: i64,
) -> Result<DiscoverPick, ApiError> {
    let mixes = source.personal_mixes().await.map_err(source_error)?;
    if !source.capabilities().complete_play_history {
        return Ok(DiscoverPick {
            mixes,
            ..DiscoverPick::default()
        });
    }
    let key = source.source();
    let mut never_played = db.unplayed_albums(key).await.map_err(db_error)?;
    never_played.shuffle(&mut rand::rng());
    never_played.truncate(DISCOVER_ROW_LIMIT);
    let likes = db
        .favorites_liked_at(key.as_str())
        .await
        .map_err(db_error)?;
    let plays = db.play_history(key).await.map_err(db_error)?;
    let cutoff = now.saturating_sub(FORGOTTEN_AFTER.as_secs() as i64);
    let keys = forgotten_keys(&likes, &plays, cutoff);
    let mut forgotten = db.tracks_by_keys(key, &keys).await.map_err(db_error)?;
    forgotten.shuffle(&mut rand::rng());
    forgotten.truncate(DISCOVER_ROW_LIMIT);
    Ok(DiscoverPick {
        mixes,
        never_played,
        forgotten,
    })
}

/// The liked tracks and the [`MOST_PLAYED`] most played whose last play or like came before `cutoff`.
fn forgotten_keys(likes: &[(String, i64)], plays: &[db::PlayRecord], cutoff: i64) -> Vec<String> {
    let played: HashMap<&str, Option<i64>> = plays
        .iter()
        .map(|record| (record.track_key.as_str(), record.last_played_at))
        .collect();
    let liked: HashMap<&str, i64> = likes.iter().map(|(key, at)| (key.as_str(), *at)).collect();
    let candidates = likes.iter().map(|(key, _)| key.as_str()).chain(
        plays
            .iter()
            .take(MOST_PLAYED)
            .map(|record| record.track_key.as_str()),
    );
    let mut seen = HashSet::new();
    let mut forgotten = Vec::new();
    for key in candidates {
        if !seen.insert(key) {
            continue;
        }
        // An undated play had left the recent history before times were kept, so it counts as long ago.
        let last_play = played.get(key).copied().flatten();
        let touched = last_play.max(liked.get(key).copied());
        if touched.is_none_or(|at| at < cutoff) {
            forgotten.push(key.to_string());
        }
    }
    forgotten
}

/// Radios from up to [`FOR_YOU_SEEDS`] random likes, the top [`FOR_YOU_PER_RADIO`] of each that are not liked, without repeats, shuffled.
async fn mix_for_you<F, Fut>(
    mut likes: Vec<String>,
    radio: F,
) -> Result<Vec<reader::Track>, ApiError>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Vec<reader::Track>, ApiError>>,
{
    let liked: HashSet<String> = likes.iter().cloned().collect();
    likes.shuffle(&mut rand::rng());
    likes.truncate(FOR_YOU_SEEDS);
    let radios = futures_util::future::join_all(likes.iter().cloned().map(radio)).await;
    let mut seen = HashSet::new();
    let mut tracks = Vec::new();
    let mut failure = None;
    let mut answered = false;
    for (seed, outcome) in likes.iter().zip(radios) {
        match outcome {
            Ok(songs) => {
                answered = true;
                let top = songs
                    .into_iter()
                    .filter(|track| !liked.contains(track.id.key().as_ref()))
                    .take(FOR_YOU_PER_RADIO);
                tracks.extend(top.filter(|track| seen.insert(track.id.key().into_owned())));
            }
            Err(error) => {
                tracing::warn!(%error, seed, "a For you radio failed");
                failure.get_or_insert(error);
            }
        }
    }
    if let (false, Some(error)) = (answered, failure) {
        return Err(error);
    }
    tracks.shuffle(&mut rand::rng());
    Ok(tracks)
}

/// The library's way to the catalog, so tests can build a queue from a catalog id offline.
#[async_trait::async_trait]
pub trait CatalogQueue: Send + Sync {
    async fn album_tracks(&self, id: &str) -> Result<Vec<reader::Track>, ApiError>;
    async fn playlist_tracks(&self, id: &str) -> Result<Vec<reader::Track>, ApiError>;
    async fn track(&self, key: &str) -> Result<Option<reader::Track>, ApiError>;
    async fn track_radio(&self, key: &str) -> Result<RadioPage, ApiError>;
    async fn playlist_radio(&self, id: &str) -> Result<RadioPage, ApiError>;
    async fn more_radio(&self, cursor: &str) -> Result<RadioPage, ApiError>;
}

#[async_trait::async_trait]
impl CatalogQueue for CatalogService {
    async fn album_tracks(&self, id: &str) -> Result<Vec<reader::Track>, ApiError> {
        CatalogService::album_tracks(self, id).await
    }

    async fn playlist_tracks(&self, id: &str) -> Result<Vec<reader::Track>, ApiError> {
        CatalogService::playlist_tracks(self, id).await
    }

    async fn track(&self, key: &str) -> Result<Option<reader::Track>, ApiError> {
        CatalogService::track(self, key).await
    }

    async fn track_radio(&self, key: &str) -> Result<RadioPage, ApiError> {
        CatalogService::track_radio(self, key).await
    }

    async fn playlist_radio(&self, id: &str) -> Result<RadioPage, ApiError> {
        CatalogService::playlist_radio(self, id).await
    }

    async fn more_radio(&self, cursor: &str) -> Result<RadioPage, ApiError> {
        CatalogService::more_radio(self, cursor).await
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

fn rating(rating: server::ytmusic::discover::Rating) -> api::Rating {
    use server::ytmusic::discover::Rating;
    match rating {
        Rating::Indifferent => api::Rating::None,
        Rating::Like => api::Rating::Like,
        Rating::Dislike => api::Rating::Dislike,
    }
}

fn privacy(privacy: server::ytmusic::discover::Privacy) -> api::PlaylistPrivacy {
    use server::ytmusic::discover::Privacy;
    match privacy {
        Privacy::Private => api::PlaylistPrivacy::Private,
        Privacy::Unlisted => api::PlaylistPrivacy::Unlisted,
        Privacy::Public => api::PlaylistPrivacy::Public,
    }
}

fn actions(actions: ItemActions) -> CatalogActions {
    CatalogActions {
        rate_ref: actions.rate_ref,
        rating: actions.rating.map(rating),
        save_ref: actions.save_ref,
        saved: actions.saved,
        follow_ref: actions.follow_ref,
        followed: actions.followed,
        history_token: actions.history_token,
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
            explicit: false,
            plays: None,
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

    mod for_you {
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        use api::{ApiError, ErrorCode, ForYouUnavailable};

        use super::super::{
            FOR_YOU_PER_RADIO, FOR_YOU_SEEDS, ForYouMixes, for_you_unavailable, forget_failed,
            held_build, mix_for_you,
        };
        use super::track;

        fn likes(keys: &[&str]) -> Vec<String> {
            keys.iter().map(|key| key.to_string()).collect()
        }

        fn sorted(tracks: &[reader::Track]) -> Vec<String> {
            let mut keys: Vec<String> = tracks.iter().map(|t| t.id.key().into_owned()).collect();
            keys.sort();
            keys
        }

        type Radio = std::future::Ready<Result<Vec<reader::Track>, ApiError>>;
        type Asked = Arc<Mutex<Vec<String>>>;

        /// A radio per seed from a fixed table; a seed missing from it fails as unreachable.
        fn radios(table: &[(&str, &[&str])]) -> (impl Fn(String) -> Radio, Asked) {
            let table: HashMap<String, Vec<String>> = table
                .iter()
                .map(|(seed, songs)| (seed.to_string(), likes(songs)))
                .collect();
            let asked = Arc::new(Mutex::new(Vec::new()));
            let log = asked.clone();
            let radio = move |seed: String| {
                log.lock().unwrap().push(seed.clone());
                std::future::ready(match table.get(&seed) {
                    Some(songs) => Ok(songs.iter().map(|key| track(key)).collect()),
                    None => Err(ApiError::new(ErrorCode::SourceUnreachable, "offline")),
                })
            };
            (radio, asked)
        }

        #[tokio::test]
        async fn each_radio_adds_its_top_three_without_the_seed() {
            let (radio, _) = radios(&[("a", &["a", "r1", "r2", "r3", "r4"])]);

            let mix = mix_for_you(likes(&["a"]), radio).await.unwrap();

            assert_eq!(FOR_YOU_PER_RADIO, 3);
            assert_eq!(sorted(&mix), ["r1", "r2", "r3"]);
        }

        #[tokio::test]
        async fn liked_tracks_are_left_out_and_the_walk_goes_past_them() {
            let (radio, _) = radios(&[("a", &["b", "r1", "a", "b", "r2", "r3", "r4"])]);

            let mix = mix_for_you(likes(&["a", "b"]), radio).await.unwrap();

            assert_eq!(sorted(&mix), ["r1", "r2", "r3"]);
        }

        #[tokio::test]
        async fn a_radio_of_only_liked_tracks_adds_none() {
            let (radio, _) = radios(&[("a", &["a", "b", "c"]), ("b", &["a", "c"]), ("c", &["b"])]);

            let mix = mix_for_you(likes(&["a", "b", "c"]), radio).await.unwrap();

            assert!(mix.is_empty(), "{:?}", sorted(&mix));
        }

        #[tokio::test]
        async fn a_song_two_radios_share_is_mixed_in_once() {
            let (radio, _) = radios(&[("a", &["x", "y", "z"]), ("b", &["y", "z", "w"])]);

            let mix = mix_for_you(likes(&["a", "b"]), radio).await.unwrap();

            assert_eq!(sorted(&mix), ["w", "x", "y", "z"]);
        }

        #[tokio::test]
        async fn five_random_likes_seed_it_and_fewer_all_do() {
            let songs: &[&str] = &["s1", "s2", "s3"];
            let many = ["l1", "l2", "l3", "l4", "l5", "l6", "l7", "l8"];
            let table: Vec<(&str, &[&str])> = many.iter().map(|seed| (*seed, songs)).collect();
            let (radio, asked) = radios(&table);

            mix_for_you(likes(&many), radio).await.unwrap();

            let mut asked = asked.lock().unwrap().clone();
            asked.sort();
            asked.dedup();
            assert_eq!(FOR_YOU_SEEDS, 5);
            assert_eq!(asked.len(), FOR_YOU_SEEDS, "{asked:?}");
            assert!(asked.iter().all(|seed| many.contains(&seed.as_str())));

            let (radio, asked) = radios(&table);
            mix_for_you(likes(&many[..3]), radio).await.unwrap();
            let mut asked = asked.lock().unwrap().clone();
            asked.sort();
            assert_eq!(asked, ["l1", "l2", "l3"]);
        }

        #[tokio::test]
        async fn the_radios_that_answered_make_the_mix_when_others_fail() {
            let (radio, _) = radios(&[("b", &["x", "y"])]);

            let mix = mix_for_you(likes(&["a", "b", "c"]), radio).await.unwrap();

            assert_eq!(sorted(&mix), ["x", "y"]);
        }

        #[tokio::test]
        async fn every_radio_failing_is_the_radios_error() {
            let (radio, _) = radios(&[]);

            let error = mix_for_you(likes(&["a", "b"]), radio).await.unwrap_err();

            assert_eq!(error.code, ErrorCode::SourceUnreachable);
        }

        #[test]
        fn no_track_radio_or_no_likes_hides_the_row() {
            assert_eq!(
                for_you_unavailable(false, &likes(&["a"])),
                Some(ForYouUnavailable::NoTrackRadio)
            );
            assert_eq!(
                for_you_unavailable(true, &[]),
                Some(ForYouUnavailable::NoLikes)
            );
            assert_eq!(for_you_unavailable(true, &likes(&["a"])), None);
        }

        fn server(id: &str) -> config::Source {
            config::Source::Server(id.into())
        }

        async fn build_on(
            mixes: &ForYouMixes,
            source: &config::Source,
            fresh: bool,
            builds: &Mutex<u32>,
        ) -> Result<Vec<reader::Track>, ApiError> {
            let build = held_build(mixes, source, fresh);
            build
                .get_or_init(|| async {
                    tokio::task::yield_now().await;
                    let mut count = builds.lock().unwrap();
                    *count += 1;
                    Ok(vec![track(&format!("build-{count}"))])
                })
                .await
                .clone()
        }

        #[tokio::test]
        async fn the_mix_holds_until_a_refresh_builds_another() {
            let mixes = ForYouMixes::default();
            let builds = Mutex::new(0);
            let yt = server("yt");

            let first = build_on(&mixes, &yt, false, &builds).await.unwrap();
            let again = build_on(&mixes, &yt, false, &builds).await.unwrap();
            assert_eq!(sorted(&first), sorted(&again));
            assert_eq!(*builds.lock().unwrap(), 1);

            let refreshed = build_on(&mixes, &yt, true, &builds).await.unwrap();
            assert_eq!(sorted(&refreshed), ["build-2"]);
            let after = build_on(&mixes, &yt, false, &builds).await.unwrap();
            assert_eq!(
                sorted(&after),
                ["build-2"],
                "the refreshed mix is the one held"
            );

            let other = build_on(&mixes, &server("other"), false, &builds)
                .await
                .unwrap();
            assert_eq!(
                sorted(&other),
                ["build-3"],
                "each source has a mix of its own"
            );
        }

        #[tokio::test]
        async fn requests_while_it_builds_share_the_one_build() {
            let mixes = ForYouMixes::default();
            let builds = Mutex::new(0);
            let yt = server("yt");

            let (a, b) = tokio::join!(
                build_on(&mixes, &yt, false, &builds),
                build_on(&mixes, &yt, false, &builds)
            );

            assert_eq!(sorted(&a.unwrap()), sorted(&b.unwrap()));
            assert_eq!(*builds.lock().unwrap(), 1);
        }

        #[tokio::test]
        async fn a_failed_build_is_not_kept() {
            let mixes = ForYouMixes::default();
            let yt = server("yt");
            let build = held_build(&mixes, &yt, false);
            build
                .get_or_init(|| async { Err(ApiError::internal("down")) })
                .await;

            forget_failed(&mixes, &yt, &build);

            assert!(!held_build(&mixes, &yt, false).initialized());
        }

        #[tokio::test]
        async fn a_late_failure_leaves_a_refreshed_build_alone() {
            let mixes = ForYouMixes::default();
            let yt = server("yt");
            let stale = held_build(&mixes, &yt, false);
            let refreshed = held_build(&mixes, &yt, true);

            forget_failed(&mixes, &yt, &stale);

            assert!(Arc::ptr_eq(&refreshed, &held_build(&mixes, &yt, false)));
        }
    }

    mod discover {
        use std::sync::Arc;

        use super::super::{
            DISCOVER_ROW_LIMIT, DiscoverPicks, FORGOTTEN_AFTER, MOST_PLAYED, forgotten_keys,
            held_build, pick_discover,
        };
        use super::track;

        const DAY: i64 = 24 * 60 * 60;
        const NOW: i64 = 1_800_000_000;

        fn cutoff() -> i64 {
            NOW - FORGOTTEN_AFTER.as_secs() as i64
        }

        fn played(key: &str, count: u64, at: Option<i64>) -> db::PlayRecord {
            db::PlayRecord {
                track_key: key.to_string(),
                count,
                last_played_at: at,
            }
        }

        fn sorted(mut keys: Vec<String>) -> Vec<String> {
            keys.sort();
            keys
        }

        #[test]
        fn the_cutoff_is_three_months() {
            assert_eq!(FORGOTTEN_AFTER.as_secs() as i64, 90 * DAY);
        }

        #[test]
        fn a_like_or_a_top_play_untouched_since_the_cutoff_is_forgotten() {
            let old = cutoff() - DAY;
            let recent = cutoff() + DAY;
            let likes = vec![
                ("liked-unplayed".to_string(), old),
                ("liked-lately".to_string(), recent),
                ("liked-played-lately".to_string(), old),
                ("liked-played-long-ago".to_string(), old),
                ("liked-played-undated".to_string(), old),
                ("liked-lately-played-undated".to_string(), recent),
            ];
            let plays = vec![
                played("top-long-ago", 50, Some(old)),
                played("top-lately", 40, Some(recent)),
                played("top-undated", 35, None),
                played("liked-played-lately", 30, Some(recent)),
                played("liked-played-long-ago", 20, Some(old)),
                played("liked-played-undated", 10, None),
                played("liked-lately-played-undated", 5, None),
            ];

            let forgotten = forgotten_keys(&likes, &plays, cutoff());

            assert_eq!(
                sorted(forgotten),
                [
                    "liked-played-long-ago",
                    "liked-played-undated",
                    "liked-unplayed",
                    "top-long-ago",
                    "top-undated",
                ]
            );
        }

        /// What the upgrade left: a play still in the recent history was stamped then, every older one is undated.
        #[test]
        fn a_recent_play_is_not_forgotten_but_an_undated_one_is_and_a_new_like_is_not() {
            let upgraded = NOW - DAY;
            let plays = vec![
                played("in-the-recent-history", 9, Some(upgraded)),
                played("older-than-the-history", 8, None),
            ];
            let likes = vec![("just-liked".to_string(), NOW)];

            let forgotten = forgotten_keys(&likes, &plays, cutoff());

            assert_eq!(forgotten, ["older-than-the-history"]);
        }

        #[test]
        fn only_the_most_played_count_beside_the_likes() {
            let old = cutoff() - DAY;
            let plays: Vec<db::PlayRecord> = (0..=MOST_PLAYED)
                .map(|n| played(&format!("t{n:03}"), (1000 - n) as u64, Some(old)))
                .collect();

            let forgotten = forgotten_keys(&[], &plays, cutoff());

            assert_eq!(forgotten.len(), MOST_PLAYED);
            let last = format!("t{MOST_PLAYED:03}");
            assert!(!forgotten.contains(&last), "{last} is past the top");
            let liked_too = forgotten_keys(&[(last.clone(), old)], &plays, cutoff());
            assert!(
                liked_too.contains(&last),
                "a like counts whatever its plays"
            );
        }

        struct Library {
            _dir: tempfile::TempDir,
            db: db::Db,
        }

        fn album(id: &str) -> reader::Album {
            reader::Album {
                id: id.to_string(),
                title: id.to_string(),
                artist: "Ada".into(),
                genre: String::new(),
                year: 0,
                cover_path: None,
                manual_cover: false,
                artist_id: None,
                artist_key: None,
            }
        }

        /// One track per album on `source`: `/a` on al-a and so on; `/a` played and liked, `/b` liked.
        async fn library(source: &config::Source, albums: &[&str]) -> Library {
            let dir = tempfile::tempdir().expect("tempdir");
            let db = db::init(&dir.path().join("discover.db")).await.expect("db");
            let tracks: Vec<reader::Track> = albums
                .iter()
                .map(|name| reader::Track {
                    album_id: format!("al-{name}"),
                    ..track(&format!("/{name}"))
                })
                .collect();
            db.upsert_tracks(source, &tracks).await.expect("tracks");
            let rows: Vec<reader::Album> = albums
                .iter()
                .map(|name| album(&format!("al-{name}")))
                .collect();
            db.upsert_albums(source, &rows).await.expect("albums");
            db.bump_listen_count(source, "/a").await.expect("play");
            db.set_favorite(source.as_str(), "/a", true)
                .await
                .expect("like");
            db.set_favorite(source.as_str(), "/b", true)
                .await
                .expect("like");
            Library { _dir: dir, db }
        }

        fn ids(albums: &[reader::Album]) -> Vec<String> {
            sorted(albums.iter().map(|album| album.id.clone()).collect())
        }

        fn keys(tracks: &[reader::Track]) -> Vec<String> {
            sorted(
                tracks
                    .iter()
                    .map(|track| track.id.key().into_owned())
                    .collect(),
            )
        }

        #[tokio::test]
        async fn a_folder_library_offers_its_unplayed_albums_and_its_forgotten_favorites() {
            let local = config::Source::default();
            let library = library(&local, &["a", "b", "c"]).await;
            let source =
                server::source::active(library.db.clone(), &server::sources::Sources::default());
            assert!(source.capabilities().complete_play_history);

            let today = pick_discover(source.as_ref(), &library.db, super::super::unix_now())
                .await
                .expect("rows");
            assert!(today.mixes.is_empty(), "a folder makes no mixes");
            assert_eq!(ids(&today.never_played), ["al-b", "al-c"]);
            assert!(
                today.forgotten.is_empty(),
                "everything was touched just now"
            );

            let later = super::super::unix_now() + FORGOTTEN_AFTER.as_secs() as i64 + DAY;
            let then = pick_discover(source.as_ref(), &library.db, later)
                .await
                .expect("rows");
            assert_eq!(keys(&then.forgotten), ["/a", "/b"]);
        }

        #[tokio::test]
        async fn a_server_whose_plays_kopuz_only_partly_sees_gets_no_play_stat_rows() {
            let id = "navi";
            let mut server = config::MusicServer::new_with_service(
                "Navidrome".into(),
                "http://127.0.0.1:9".into(),
                config::MusicService::Subsonic,
            );
            server.id = Some(id.into());
            server.access_token = Some("token".into());
            server.user_id = Some("ada".into());
            let sources = server::sources::Sources {
                server: Some(server),
                active: config::Source::Server(id.into()),
                ..Default::default()
            };
            let library = library(&sources.active, &["a", "b", "c"]).await;
            let source = server::source::active(library.db.clone(), &sources);
            assert!(!source.capabilities().complete_play_history);

            let later = super::super::unix_now() + FORGOTTEN_AFTER.as_secs() as i64 + DAY;
            let rows = pick_discover(source.as_ref(), &library.db, later)
                .await
                .expect("rows");

            assert!(rows.mixes.is_empty());
            assert!(
                rows.never_played.is_empty(),
                "its stats are not the whole history"
            );
            assert!(rows.forgotten.is_empty());
        }

        #[tokio::test]
        async fn a_big_library_is_cut_to_one_row() {
            let local = config::Source::default();
            let names: Vec<String> = (0..DISCOVER_ROW_LIMIT + 10)
                .map(|n| format!("n{n}"))
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let library = library(&local, &names).await;
            let source =
                server::source::active(library.db.clone(), &server::sources::Sources::default());

            let rows = pick_discover(source.as_ref(), &library.db, super::super::unix_now())
                .await
                .expect("rows");

            assert_eq!(rows.never_played.len(), DISCOVER_ROW_LIMIT);
        }

        #[tokio::test]
        async fn the_rows_hold_until_a_refresh_picks_again() {
            let picks = DiscoverPicks::default();
            let local = config::Source::default();
            let built = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let build = |fresh: bool| {
                let built = built.clone();
                let held = held_build(&picks, &local, fresh);
                async move {
                    held.get_or_init(|| async {
                        built.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(super::super::DiscoverPick::default())
                    })
                    .await
                    .is_ok()
                }
            };

            assert!(build(false).await && build(false).await);
            assert_eq!(built.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert!(build(true).await && build(false).await);
            assert_eq!(built.load(std::sync::atomic::Ordering::SeqCst), 2);
        }
    }
}
