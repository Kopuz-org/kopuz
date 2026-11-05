//! The library: one page, four facets, one filter set. What was four pages and four sort controls.

mod bar;
mod columns;
mod facet;
mod filters;
mod grid;
mod playlists;
mod settings;
mod states;
mod tracks;

use std::collections::{HashMap, HashSet};
use std::ops::{Deref, Range};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{
    AlbumInfo, AlbumPage, AlbumQuery, ArtistPage, ArtworkRef, KopuzApi, Page, StatsView, Table,
    TrackInfo, TrackPage,
};
use components::format::t;
use components::{skeleton, style};
use config::{AlbumSortField, SortCriterion, SortDirection};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, ParentElement as _, Render,
    ScrollStrategy, SharedString, Styled as _, Subscription, Task, UniformListScrollHandle, Window,
    actions, div, px,
};
use store::layout::{Column, Density, TileSize, ViewMode};
use store::{
    DownloadsModel, LayoutModel, Link, Navigator, Pager, PlayerModel, PlaylistsModel, Query,
    QueueModel, Route, SourcesModel, Target,
};

pub use facet::Facet;
pub use filters::{Chip, Filters, Sort};

/// Typing settles before the listing is asked for again; the Dioxus filter used the same delay.
const FILTER_DELAY: Duration = Duration::from_millis(180);

/// How long a type-ahead run stays open before the next letter starts a new one.
const TYPE_AHEAD_GAP: Duration = Duration::from_millis(900);

actions!(
    library,
    [
        FocusFilter,
        CursorUp,
        CursorDown,
        Activate,
        SelectEverything,
        ClearSelection,
        NextFacet,
        PreviousFacet
    ]
);

/// Register what the library contributes to the rest of the app; call once at startup.
pub fn register(cx: &mut App) {
    settings::register(cx);
    cx.bind_keys([
        KeyBinding::new("/", FocusFilter, Some("Library")),
        KeyBinding::new("down", CursorDown, Some("Library")),
        KeyBinding::new("up", CursorUp, Some("Library")),
        KeyBinding::new("enter", Activate, Some("Library")),
        KeyBinding::new("ctrl-a", SelectEverything, Some("Library")),
        KeyBinding::new("cmd-a", SelectEverything, Some("Library")),
        KeyBinding::new("escape", ClearSelection, Some("Library")),
        KeyBinding::new("ctrl-tab", NextFacet, Some("Library")),
        KeyBinding::new("ctrl-shift-tab", PreviousFacet, Some("Library")),
        // The deeper Input context wins, so "/" typed into a filter box stays a character.
        KeyBinding::new("/", gpui_kit::NoAction, Some("Input")),
        KeyBinding::new("down", gpui_kit::NoAction, Some("Input")),
        KeyBinding::new("up", gpui_kit::NoAction, Some("Input")),
    ]);
}

/// One tile of a grid facet, flattened so a band builds from cached strings rather than a model.
#[derive(Clone)]
pub(crate) struct Tile {
    pub id: SharedString,
    pub title: SharedString,
    pub subtitle: SharedString,
    pub artwork: Option<ArtworkRef>,
    pub sort_key: String,
    /// What a numeric order runs on: a year, or how many tracks the entry holds.
    pub sort_rank: u64,
    pub playing: bool,
    pub target: Target,
    pub route: Route,
    pub pictured: Option<store::Pictured>,
    pub placeholder: &'static str,
    /// A playlist folder, which leads the playlists facet.
    pub folder: bool,
    /// A playlist's keys and what the downloads say of them, when the source downloads.
    pub offline: Option<(Rc<[String]>, store::Offline)>,
}

/// A line of the tracks table: a group heading, or one row of the loaded listing.
#[derive(Clone)]
pub(crate) enum Line {
    Group(SharedString),
    Track(usize),
}

/// What one album contributes to a chip or a group that the track rows do not carry.
#[derive(Clone, Default)]
struct AlbumMeta {
    year: u16,
    genre: String,
}

/// How much the library holds, for the stat cards; a count the source could not give is left out.
#[derive(Clone, Copy, Default)]
struct Counts {
    tracks: Option<u32>,
    albums: Option<u32>,
    artists: Option<u32>,
}

async fn counts(api: Arc<dyn KopuzApi>) -> Result<Counts, api::ApiError> {
    let one = store::paging::page(0, 1);
    let total = |what: &'static str, read: Result<u32, api::ApiError>| match read {
        Ok(total) => Some(total),
        Err(error) => {
            tracing::debug!(%error, what, "could not count the library");
            None
        }
    };
    let tracks = api
        .tracks(Default::default(), one)
        .await
        .map(|page| page.total);
    let albums = api
        .albums(Default::default(), one)
        .await
        .map(|page| page.total);
    let artists = api
        .artists(Default::default(), one)
        .await
        .map(|page| page.total);
    Ok(Counts {
        tracks: total("tracks", tracks),
        albums: total("albums", albums),
        artists: total("artists", artists),
    })
}

#[derive(Clone)]
struct CatalogWindow<T> {
    offset: usize,
    page: T,
    /// Asked for with a chip or a search on, so it is not the whole catalog.
    narrowed: bool,
}

impl<T> Deref for CatalogWindow<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.page
    }
}

/// The library page, or the Favorites page: the same listing held to liked tracks.
pub struct Library {
    filters: Filters,
    /// The Favorites page: tracks only, liked only, and none of it written to the library's route.
    liked: bool,
    /// The Activity page: tracks only, in a layout of its own, and none of it written to the library's route.
    activity: bool,
    // What the route last said, so adopting a route change cannot fight the page's own edit.
    encoded: String,
    columns: Vec<Column>,
    sort: Sort,
    group_by: Option<String>,
    mode: ViewMode,
    density: Density,
    tile: TileSize,

    search: Entity<InputState>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    // The tiles share the table's scroller, so a jump lands the same index in either.
    grid: components::TileGrid,

    tracks: Entity<Query<TrackPage>>,
    albums: Entity<Query<CatalogWindow<AlbumPage>>>,
    artists: Entity<Query<CatalogWindow<ArtistPage>>>,
    genres: Entity<Query<Vec<String>>>,
    stats: Entity<Query<StatsView>>,
    counts: Entity<Query<Counts>>,
    years: Entity<Query<Option<(u16, u16)>>>,
    meta_asked: MetaAsks,

    // Derived on change, never in render: a notify from the playhead would otherwise rebuild these.
    rows: Vec<TrackInfo>,
    lines: Vec<Line>,
    tiles: Vec<Tile>,
    // The letters the tiles start with, only while every tile is held: a paged window cannot say.
    rail_letters: Option<std::collections::HashSet<char>>,
    albums_meta: HashMap<String, AlbumMeta>,
    plays: HashMap<String, u64>,
    playlist_offline: HashMap<String, (Rc<[String]>, store::Offline)>,
    // Artists already sent for a photo; the daemon remembers its misses, this saves the round trip.
    asked_artwork: HashSet<String>,
    playing: store::Playing,
    // The playing row lights only while the queue is this listing, judged as the Dioxus table did: by length.
    queued: bool,
    pager: Pager,
    window_request: Option<Range<usize>>,
    whole: bool,
    // Where the viewport starts, for the pinned group heading.
    visible_from: usize,

    selection: components::selection::Selection,
    type_ahead: String,
    type_ahead_at: Option<Instant>,

    // The last frame's reads of the queries and the source, which a frozen page keeps drawing.
    shared: Shared,
    frozen: bool,

    filter_delay: store::Debounce,
    _resolve: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// What a frame reads outside the page's own derived state.
#[derive(Clone, Default)]
struct Shared {
    stale: bool,
    settled: bool,
    failure: Option<api::ApiError>,
    active: states::Active,
}

impl store::Freeze for Library {
    fn freeze(&mut self, _: &mut Context<Self>) {
        self.frozen = true;
        self._subscriptions.clear();
        self.filter_delay.cancel();
    }
}

impl Focusable for Library {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Library {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(false, false, window, cx)
    }

    /// The Favorites page.
    pub fn favorites(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(true, false, window, cx)
    }

    /// The Activity page: the library's tracks, most played first.
    pub fn activity(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(false, true, window, cx)
    }

    fn build(liked: bool, activity: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let standalone = liked || activity;
        let opening = match standalone {
            true => String::new(),
            false => opening_view(cx),
        };
        let filters = match (liked, activity) {
            (true, _) => Filters::default().held_to_liked(),
            (_, true) => Filters::on(Facet::Tracks),
            _ => Filters::decode(&opening),
        };
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t("library_filter_placeholder"))
                .default_value(filters.search.clone())
        });

        let tracks = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Favorites],
                |_| async move { Ok(TrackPage::default()) },
                cx,
            )
        });
        let first = store::paging::page(0, store::paging::CHUNK);
        let albums = cx.new(|cx| {
            Query::new(
                &[Table::Albums],
                move |api| async move {
                    Ok(CatalogWindow {
                        offset: 0,
                        page: api.albums(Default::default(), first).await?,
                        narrowed: false,
                    })
                },
                cx,
            )
        });
        let artists = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Artists],
                move |api| async move {
                    Ok(CatalogWindow {
                        offset: 0,
                        page: api.artists(Default::default(), first).await?,
                        narrowed: false,
                    })
                },
                cx,
            )
        });
        let genres = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Albums],
                |api| async move { api.genres().await },
                cx,
            )
        });
        let stats = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Recents],
                |api| async move { api.stats().await },
                cx,
            )
        });
        let years = cx.new(|cx| Query::new(&[Table::Albums], year_span, cx));
        let counts = cx.new(|cx| match standalone {
            true => Query::new(&[], |_| async move { Ok(Counts::default()) }, cx),
            false => Query::new(&[Table::Tracks, Table::Albums, Table::Artists], counts, cx),
        });

        let navigator = Navigator::global(cx);
        let playlists = PlaylistsModel::global(cx);
        let downloads = DownloadsModel::global(cx);
        let player = PlayerModel::global(cx);
        let layout = LayoutModel::global(cx);
        let sources = SourcesModel::global(cx);
        let subscriptions = vec![
            cx.subscribe(&search, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let typed = state.read(cx).value().to_string();
                    this.delay_filter(typed, cx);
                }
            }),
            cx.observe(&tracks, |this, _, cx| this.adopt_tracks(cx)),
            cx.observe(&albums, |this, _, cx| this.adopt_albums(cx)),
            cx.observe(&artists, |this, _, cx| this.adopt_artists(cx)),
            cx.observe(&genres, |_, _, cx| cx.notify()),
            cx.observe(&stats, |this, _, cx| this.adopt_stats(cx)),
            cx.observe(&counts, |this, _, cx| {
                this.auto_sync(cx);
                cx.notify();
            }),
            cx.observe(&years, |_, _, cx| cx.notify()),
            // A retag can move an album's genre; only the albums on screen are asked again.
            cx.subscribe(
                &store::Invalidation::global(cx),
                |this, _, event: &store::TablesChanged, cx| {
                    if meta_stale(event) {
                        this.meta_asked.invalidate();
                        this.request_meta(cx);
                    }
                },
            ),
            cx.observe(&playlists, |this, _, cx| {
                this.rekey_offline(cx);
                this.rebuild(cx);
            }),
            cx.observe(&downloads, |this, _, cx| this.on_downloads(cx)),
            // The playing mark sits on every row and tile, so it is derived here, not in render.
            cx.observe(&player, |this, _, cx| this.adopt_playing(cx)),
            cx.observe(&QueueModel::global(cx), |this, _, cx| this.adopt_queue(cx)),
            cx.observe(&layout, |this, _, cx| this.adopt_layout(cx)),
            cx.observe(&store::ConfigModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&sources, |this, _, cx| this.on_sources(cx)),
            cx.observe(&store::JobsModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe_in(&navigator, window, |this, _, window, cx| {
                this.adopt_route(window, cx)
            }),
        ];

        let scroll = UniformListScrollHandle::new();
        let mut page = Self {
            filters,
            liked,
            activity,
            encoded: opening,
            columns: Vec::new(),
            sort: Sort::default(),
            group_by: None,
            mode: ViewMode::Grid,
            density: Density::default(),
            tile: TileSize::default(),
            search,
            focus: components::focus::stop_handle(cx),
            grid: components::TileGrid::new("library-grid").sharing(scroll.clone()),
            scroll,
            tracks,
            albums,
            artists,
            genres,
            stats,
            counts,
            years,
            meta_asked: MetaAsks::default(),
            rows: Vec::new(),
            lines: Vec::new(),
            tiles: Vec::new(),
            rail_letters: None,
            albums_meta: HashMap::new(),
            plays: HashMap::new(),
            playlist_offline: HashMap::new(),
            asked_artwork: HashSet::new(),
            playing: store::Playing::default(),
            queued: false,
            pager: Pager::new(),
            window_request: None,
            whole: false,
            visible_from: 0,
            selection: components::selection::Selection::new(),
            type_ahead: String::new(),
            type_ahead_at: None,
            shared: Shared::default(),
            frozen: false,
            filter_delay: store::Debounce::default(),
            _resolve: None,
            _subscriptions: subscriptions,
        };
        page.read_layout(cx);
        page.rekey_offline(cx);
        page.refetch(cx);
        page
    }

    /// The page's title.
    pub(crate) fn title(&self) -> SharedString {
        match (self.liked, self.activity) {
            (true, _) => t("favorites"),
            (_, true) => t("listening_logs"),
            _ => t("your_library"),
        }
    }

    pub(crate) fn is_activity(&self) -> bool {
        self.activity
    }

    pub(crate) fn is_liked(&self) -> bool {
        self.liked
    }

    /// Favorites or Activity: a page of its own, with no facets and nothing written to the library's route.
    pub(crate) fn standalone(&self) -> bool {
        self.liked || self.activity
    }

    /// How this facet was left: its columns, its order, its density.
    fn read_layout(&mut self, cx: &App) {
        let held = store::layout(cx).library_facet(self.layout_name());
        self.columns = columns::resolve(&held.columns);
        self.sort = Sort::from_layout(&held.sort);
        self.group_by = held.group_by.clone();
        self.mode = held.mode;
        self.density = held.density;
        self.tile = held.tile;
    }

    /// A layout edit is a re-read; only a change of order or grouping costs a daemon call.
    fn adopt_layout(&mut self, cx: &mut Context<Self>) {
        let before = (self.sort.clone(), self.group_by.clone());
        let held = (self.columns.clone(), self.mode, self.density, self.tile);
        self.read_layout(cx);
        let after = (self.sort.clone(), self.group_by.clone());
        if before != after {
            self.refetch(cx);
            return;
        }
        if held != (self.columns.clone(), self.mode, self.density, self.tile) {
            self.request_meta(cx);
            cx.notify();
        }
    }

    /// Back, forward, or a command that named a view: take what the route says and show it.
    fn adopt_route(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let route = Navigator::global(cx).read(cx).current().clone();
        if self.standalone() || !matches!(route, Route::Library | Route::LibraryView(_)) {
            return;
        }
        let encoded = route
            .library_view()
            .map(|view| view.to_string())
            .unwrap_or_default();
        if encoded == self.encoded {
            return;
        }
        self.encoded = encoded.clone();
        let filters = Filters::decode(&encoded);
        if filters == self.filters {
            return;
        }
        let typed = filters.search.clone();
        self.filters = filters;
        self.selection.clear();
        self.search
            .update(cx, |state, cx| state.set_value(typed, window, cx));
        self.read_layout(cx);
        self.refetch(cx);
    }

    /// Put the current view in the route, so back, forward and a restart all land back on it.
    fn publish(&mut self, push: bool, cx: &mut Context<Self>) {
        // Favorites and Activity have routes of their own; their search must not become the library's view.
        if self.standalone() {
            return;
        }
        let encoded = self.filters.encode();
        if encoded == self.encoded {
            return;
        }
        self.encoded = encoded.clone();
        LayoutModel::edit(cx, {
            let encoded = encoded.clone();
            move |layout| layout.library_view = Some(encoded)
        });
        let route = Route::LibraryView(SharedString::from(encoded));
        Navigator::global(cx).update(cx, |navigator, cx| match push {
            true => navigator.go(route, cx),
            false => navigator.replace(route, cx),
        });
    }

    /// Change the filter set: a chip or a facet is a place to come back to, a keystroke is not.
    pub(crate) fn set_filters(&mut self, filters: Filters, push: bool, cx: &mut Context<Self>) {
        let filters = match (self.liked, self.activity) {
            (true, _) => filters.held_to_liked(),
            (_, true) => Filters {
                facet: Facet::Tracks,
                ..filters
            },
            _ => filters,
        };
        if filters == self.filters {
            return;
        }
        let facet_changed = filters.facet != self.filters.facet;
        self.filters = filters;
        self.selection.clear();
        if facet_changed {
            self.read_layout(cx);
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
        self.publish(push, cx);
        self.refetch(cx);
    }

    pub(crate) fn filters(&self) -> &Filters {
        &self.filters
    }

    pub(crate) fn year_span(&self, cx: &App) -> Option<(u16, u16)> {
        self.years.read(cx).value().copied().flatten()
    }

    pub(crate) fn known_genres(&self, cx: &App) -> Vec<String> {
        self.genres.read(cx).value().cloned().unwrap_or_default()
    }

    /// Edit this facet's remembered layout; everything the page draws follows from it.
    pub(crate) fn write_facet(&self, cx: &mut App, edit: impl FnOnce(&mut store::layout::Facet)) {
        let name = self.layout_name().to_string();
        LayoutModel::edit(cx, move |layout| layout.edit_library_facet(&name, edit));
    }

    /// Where this page keeps its columns and order.
    fn layout_name(&self) -> &'static str {
        match (self.liked, self.activity) {
            (true, _) => store::layout::FAVORITES,
            (_, true) => store::layout::ACTIVITY,
            _ => self.filters.facet.name(),
        }
    }

    fn delay_filter(&mut self, typed: String, cx: &mut Context<Self>) {
        self.filter_delay.run(FILTER_DELAY, cx, |this, cx| {
            let mut filters = this.filters.clone();
            filters.search = typed;
            this.set_filters(filters, false, cx);
        });
    }

    /// Whether grouping, a column the daemon cannot order by, or the jump rail needs the whole listing.
    fn needs_whole(&self) -> bool {
        match self.filters.facet {
            Facet::Tracks => {
                self.group_by.is_some() || !filters::is_server_sortable(&self.sort.field)
            }
            // A playlist catalog is bounded by what a person made and arrives in one read anyway.
            Facet::Playlists => true,
            // The daemon narrows it; a narrowed set is small enough to hold whole and keep the rail.
            _ => self.filters.is_narrowed(),
        }
    }

    /// Point every read this facet needs at the filter set that is showing.
    fn refetch(&mut self, cx: &mut Context<Self>) {
        self.pager.reset();
        self.window_request = None;
        self.whole = self.needs_whole();
        let page = match self.whole {
            true => store::all(),
            false => self.pager.first(),
        };
        match self.filters.facet {
            Facet::Tracks => self.fetch_tracks(page, false, cx),
            Facet::Albums => self.fetch_albums(page, false, cx),
            Facet::Artists => self.fetch_artists(page, false, cx),
            Facet::Playlists => self.rebuild(cx),
        }
        self.request_meta(cx);
        self.auto_sync(cx);
    }

    /// Read the albums the loaded rows belong to, once the table shows something only an album knows.
    fn request_meta(&mut self, cx: &mut Context<Self>) {
        let genre = self
            .columns
            .iter()
            .any(|column| column.id == "genre" && column.visible);
        let year = self.group_by.as_deref() == Some("year");
        if self.filters.facet != Facet::Tracks || !(genre || year) {
            return;
        }
        for batch in self.meta_asked.take(&self.rows, Instant::now()) {
            let query = AlbumQuery {
                ids: Some(batch.clone()),
                ..Default::default()
            };
            let call = Link::global(cx)
                .call(move |api| async move { api.albums(query, store::all()).await });
            cx.spawn(async move |this, cx| {
                let reply = call.await;
                this.update(cx, |this, cx| match reply {
                    Ok(page) => {
                        merge_meta(&mut this.albums_meta, &batch, &page.albums);
                        this.rebuild(cx);
                    }
                    Err(error) => {
                        tracing::debug!(%error, "could not read the rows' albums");
                        this.meta_asked.failed(&batch, Instant::now());
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// An empty library on a source that syncs fills itself once, and a flat playlist catalog asks the source on sight.
    fn auto_sync(&mut self, cx: &mut Context<Self>) {
        if self.standalone() || self.frozen {
            return;
        }
        let sources = SourcesModel::global(cx).read(cx);
        let capabilities = sources.capabilities();
        let Some(source) = sources.active().cloned() else {
            return;
        };
        if !capabilities.sync || sources.offline_reads() || !source.authenticated {
            return;
        }
        let empty = self
            .counts
            .read(cx)
            .value()
            .is_some_and(|counts| counts.tracks == Some(0));
        let jobs = store::JobsModel::global(cx).read(cx);
        let library = empty
            && !capabilities.scan_folders
            && jobs.progress(api::JobKind::LibrarySync).is_none();
        let playlists = self.filters.facet == Facet::Playlists
            && !capabilities.folders
            && jobs.progress(api::JobKind::PlaylistSync).is_none();
        if library {
            playlists::start_once(api::JobKind::LibrarySync, &source.id, cx);
        }
        if playlists {
            playlists::start_once(api::JobKind::PlaylistSync, &source.id, cx);
        }
    }

    fn on_sources(&mut self, cx: &mut Context<Self>) {
        if self.frozen {
            return;
        }
        self.rekey_offline(cx);
        if self.filters.facet == Facet::Playlists {
            self.rebuild(cx);
        }
        self.auto_sync(cx);
        cx.notify();
    }

    fn fetch_tracks(&mut self, page: Page, paging: bool, cx: &mut Context<Self>) {
        let filter = self.filters.track_filter(&self.sort);
        let folder = self.filters.folder.clone();
        let fetch = move |api: Arc<dyn KopuzApi>| {
            let filter = filter.clone();
            let folder = folder.clone();
            async move {
                match folder {
                    Some(prefix) => api.folder_tracks(prefix, page).await,
                    None => api.tracks(filter, page).await,
                }
            }
        };
        self.tracks.update(cx, |query, cx| match paging {
            true => query.set_page(fetch, cx),
            false => query.set_fetch(fetch, cx),
        });
    }

    fn fetch_albums(&mut self, page: Page, paging: bool, cx: &mut Context<Self>) {
        let query = self.filters.album_query(&self.sort);
        let narrowed = self.filters.is_narrowed();
        let fetch = move |api: Arc<dyn KopuzApi>| {
            let query = query.clone();
            async move {
                Ok(CatalogWindow {
                    offset: page.offset as usize,
                    page: api.albums(query, page).await?,
                    narrowed,
                })
            }
        };
        self.albums.update(cx, |query, cx| match paging {
            true => query.set_page(fetch, cx),
            false => query.set_fetch(fetch, cx),
        });
    }

    fn fetch_artists(&mut self, page: Page, paging: bool, cx: &mut Context<Self>) {
        let query = self.filters.artist_query(&self.sort);
        let narrowed = self.filters.is_narrowed();
        let fetch = move |api: Arc<dyn KopuzApi>| {
            let query = query.clone();
            async move {
                Ok(CatalogWindow {
                    offset: page.offset as usize,
                    page: api.artists(query, page).await?,
                    narrowed,
                })
            }
        };
        self.artists.update(cx, |query, cx| match paging {
            true => query.set_page(fetch, cx),
            false => query.set_fetch(fetch, cx),
        });
    }

    /// Ask for the window the viewport needs; once per window, not once per frame.
    pub(crate) fn ensure_window(
        &mut self,
        range: &std::ops::Range<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.whole {
            return;
        }
        // Measurement asks for the first band too; only the final visible range should fetch.
        if self.window_request.replace(range.clone()).is_some() {
            return;
        }
        cx.defer_in(window, move |this, _, cx| {
            let Some(range) = this.window_request.take() else {
                return;
            };
            let Some(page) = this.pager.needs(&range) else {
                return;
            };
            match this.filters.facet {
                Facet::Tracks => this.fetch_tracks(page, true, cx),
                Facet::Albums => this.fetch_albums(page, true, cx),
                Facet::Artists => this.fetch_artists(page, true, cx),
                Facet::Playlists => {}
            }
        });
    }

    fn adopt_tracks(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.tracks.read(cx).value().cloned() else {
            cx.notify();
            return;
        };
        self.pager
            .adopt(page.offset as usize, page.items.len(), page.total as usize);
        self.rows = page.items;
        if self.whole {
            self.narrow_rows();
        }
        let loaded: HashSet<String> = self.rows.iter().map(|row| row.key.clone()).collect();
        self.selection.retain_loaded(&loaded, self.whole);
        self.queued = self.listing_queued(cx);
        self.request_meta(cx);
        self.rebuild(cx);
    }

    /// A column the daemon cannot order by, put in order over what came back whole.
    fn narrow_rows(&mut self) {
        if !filters::is_server_sortable(&self.sort.field) {
            let field = self.sort.field.clone();
            let descending = self.sort.descending;
            let plays = self.plays.clone();
            self.rows.sort_by(|left, right| {
                let order = compare(&field, left, right, &plays);
                match descending {
                    true => order.reverse(),
                    false => order,
                }
            });
        }
    }

    fn adopt_albums(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.albums.read(cx).value() else {
            cx.notify();
            return;
        };
        // A window or a narrowed set is not the whole catalog, so the chip menus only learn from a whole read.
        if !page.narrowed && page.albums.len() as u32 >= page.total {
            self.albums_meta.extend(page.albums.iter().map(album_meta));
        }
        if self.filters.facet == Facet::Albums {
            let total = page.total.max(page.albums.len() as u32) as usize;
            self.pager.adopt(page.offset, page.albums.len(), total);
        }
        self.rebuild(cx);
    }

    fn adopt_artists(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.artists.read(cx).value()
            && self.filters.facet == Facet::Artists
        {
            let total = page.total.max(page.artists.len() as u32) as usize;
            self.pager.adopt(page.offset, page.artists.len(), total);
            let missing: Vec<String> = page
                .artists
                .iter()
                .filter(|artist| artist.artwork.is_none())
                .map(|artist| artist.key.clone())
                .filter(|key| self.asked_artwork.insert(key.clone()))
                .collect();
            // Photos found arrive as a tracks invalidation, which reads this window again.
            if !missing.is_empty() {
                Link::global(cx).fire(cx, "refresh artist artwork", move |api| async move {
                    api.refresh_artist_artwork(missing).await
                });
            }
        }
        self.rebuild(cx);
    }

    fn adopt_stats(&mut self, cx: &mut Context<Self>) {
        self.plays = self
            .stats
            .read(cx)
            .value()
            .map(|stats| stats.listen_counts.clone())
            .unwrap_or_default();
        cx.notify();
    }

    fn adopt_playing(&mut self, cx: &mut Context<Self>) {
        let player = PlayerModel::global(cx);
        let playing = player
            .read(cx)
            .track()
            .map(store::Playing::of)
            .unwrap_or_default();
        if playing == self.playing {
            return;
        }
        self.playing = playing;
        self.rebuild(cx);
    }

    fn adopt_queue(&mut self, cx: &mut Context<Self>) {
        let queued = self.listing_queued(cx);
        if queued != self.queued {
            self.queued = queued;
            cx.notify();
        }
    }

    fn on_downloads(&mut self, cx: &mut Context<Self>) {
        self.restate_offline(cx);
        match (self.filters.downloaded, self.filters.facet) {
            (true, _) => self.refetch(cx),
            (false, Facet::Playlists) => self.rebuild(cx),
            (false, _) => cx.notify(),
        }
    }

    /// Everything a frame reads, computed once per change: lines, tiles and the playing mark.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        match self.filters.facet {
            Facet::Tracks => {
                self.tiles.clear();
                self.rebuild_lines();
            }
            _ => {
                self.lines.clear();
                self.rebuild_tiles(cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn density(&self) -> Density {
        self.density
    }

    pub(crate) fn sort(&self) -> &Sort {
        &self.sort
    }

    pub(crate) fn group_by(&self) -> Option<&str> {
        self.group_by.as_deref()
    }

    pub(crate) fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// How long the listing is: the filtered total under paging, what is held when read whole.
    pub(crate) fn total(&self) -> usize {
        match (self.whole, self.filters.facet) {
            (true, Facet::Tracks) => self.rows.len(),
            (true, _) => self.tiles.len(),
            (false, Facet::Tracks) => self.pager.total(),
            (false, _) => self.pager.total().max(self.tiles.len()),
        }
    }

    /// The track at an index into the whole listing, if that part of it is loaded.
    pub(crate) fn track_at(&self, index: usize) -> Option<&TrackInfo> {
        match self.whole {
            true => self.rows.get(index),
            false => index
                .checked_sub(self.pager.offset())
                .and_then(|slot| self.rows.get(slot)),
        }
    }

    fn tile_offset(&self) -> usize {
        if self.whole { 0 } else { self.pager.offset() }
    }

    fn tile_at(&self, index: usize) -> Option<&Tile> {
        window_item(&self.tiles, self.tile_offset(), index)
    }

    pub(crate) fn plays_for(&self, track: &TrackInfo) -> u64 {
        self.plays.get(&track.uid).copied().unwrap_or_default()
    }

    pub(crate) fn is_playing(&self, key: &str) -> bool {
        components::NowPlaying::WhenQueued.lights(self.queued) && self.playing.is_track(key)
    }

    /// The genre column's text: a track row carries none, so it is its album's.
    pub(crate) fn genre_of(&self, track: &TrackInfo) -> &str {
        self.albums_meta
            .get(&track.album_id)
            .map_or("", |meta| meta.genre.as_str())
    }

    /// Whether the first read has answered, whatever it answered with.
    fn settled(&self, cx: &App) -> bool {
        match self.filters.facet {
            Facet::Tracks => self.tracks.read(cx).is_settled(),
            Facet::Albums => self.albums.read(cx).is_settled(),
            Facet::Artists => self.artists.read(cx).is_settled(),
            Facet::Playlists => PlaylistsModel::global(cx).read(cx).is_loaded(),
        }
    }

    fn stale(&self, cx: &App) -> bool {
        match self.filters.facet {
            Facet::Tracks => self.tracks.read(cx).is_stale(),
            Facet::Albums => self.albums.read(cx).is_stale(),
            Facet::Artists => self.artists.read(cx).is_stale(),
            Facet::Playlists => false,
        }
    }

    fn failure(&self, cx: &App) -> Option<api::ApiError> {
        match self.filters.facet {
            Facet::Tracks => self.tracks.read(cx).error().cloned(),
            Facet::Albums => self.albums.read(cx).error().cloned(),
            Facet::Artists => self.artists.read(cx).error().cloned(),
            Facet::Playlists => None,
        }
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        match self.filters.facet {
            Facet::Tracks => self.tracks.update(cx, |query, cx| query.refresh(cx)),
            Facet::Albums => self.albums.update(cx, |query, cx| query.refresh(cx)),
            Facet::Artists => self.artists.update(cx, |query, cx| query.refresh(cx)),
            Facet::Playlists => {
                PlaylistsModel::global(cx).update(cx, |model, cx| model.refetch(cx))
            }
        }
    }

    fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Type a letter with a grid focused and it jumps to the first title that starts with the run.
    fn type_ahead(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        // A letter belongs to the filter box while it has the caret; the grid only gets the rest.
        let mine = window.focused(cx).is_some_and(|held| held == self.focus);
        if !mine || !self.filters.facet.is_grid() || event.keystroke.modifiers.secondary() {
            cx.propagate();
            return;
        }
        let Some(letter) = event
            .keystroke
            .key_char
            .as_ref()
            .filter(|key| key.chars().count() == 1 && !key.trim().is_empty())
        else {
            cx.propagate();
            return;
        };
        let fresh = self
            .type_ahead_at
            .is_none_or(|at| at.elapsed() > TYPE_AHEAD_GAP);
        if fresh {
            self.type_ahead.clear();
        }
        self.type_ahead.push_str(&letter.to_lowercase());
        self.type_ahead_at = Some(Instant::now());
        let run = self.type_ahead.clone();
        if let Some(index) = self
            .tiles
            .iter()
            .position(|tile| tile.sort_key.starts_with(&run))
        {
            let index = index + self.tile_offset();
            self.selection.set_cursor(Some(index));
            self.jump_to(index);
            cx.notify();
        }
    }

    /// Put `index` on screen, in whichever unit the facet virtualizes by.
    pub(crate) fn jump_to(&self, index: usize) {
        let band = match self.filters.facet.is_grid() && self.mode == ViewMode::Grid {
            true => index / self.grid.columns(),
            false => index,
        };
        self.scroll.scroll_to_item(band, ScrollStrategy::Top);
    }

    fn move_cursor(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.total();
        if count == 0 {
            return;
        }
        let last = count - 1;
        let next = match self.selection.cursor() {
            Some(current) => current.saturating_add_signed(step).min(last),
            None if step < 0 => last,
            None => 0,
        };
        self.selection.set_cursor(Some(next));
        self.jump_to(next);
        cx.notify();
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selection.is_active() {
            cx.propagate();
            return;
        }
        self.selection.clear();
        cx.notify();
    }

    fn select_everything(&mut self, cx: &mut Context<Self>) {
        self.selection.select_everything();
        cx.notify();
    }

    fn cycle_facet(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.standalone() {
            return;
        }
        let at = Facet::ALL
            .iter()
            .position(|facet| *facet == self.filters.facet)
            .unwrap_or(0);
        let next = (at as isize + step).rem_euclid(Facet::ALL.len() as isize) as usize;
        let mut filters = self.filters.clone();
        filters.facet = Facet::ALL[next];
        self.set_filters(filters, true, cx);
    }

    /// Enter on the keyboard cursor: play the row, or open the tile.
    fn activate(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selection.cursor() else {
            cx.propagate();
            return;
        };
        match self.filters.facet {
            Facet::Tracks => self.play_from(index, cx),
            _ => {
                if let Some(tile) = self.tile_at(index) {
                    let route = tile.route.clone();
                    Navigator::navigate(route, cx);
                }
            }
        }
    }
}

fn window_item<T>(items: &[T], offset: usize, index: usize) -> Option<&T> {
    index.checked_sub(offset).and_then(|slot| items.get(slot))
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("library");
        if !self.frozen {
            self.shared = Shared {
                stale: self.stale(cx),
                settled: self.settled(cx),
                failure: self.failure(cx),
                active: states::Active::read(cx),
            };
        }
        let dimmed = self.shared.stale;
        let chrome = self.chrome(window, cx);
        let body = self.body(window, cx);
        let bar = self.selection_bar(cx);
        components::page()
            .relative()
            .track_focus(&self.focus)
            .key_context("Library")
            .tab_index(0)
            .on_action(
                cx.listener(|this, _: &FocusFilter, window, cx| this.focus_filter(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CursorDown, _, cx| this.move_cursor(1, cx)))
            .on_action(cx.listener(|this, _: &CursorUp, _, cx| this.move_cursor(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectEverything, _, cx| this.select_everything(cx)))
            .on_action(cx.listener(|this, _: &ClearSelection, _, cx| this.clear_selection(cx)))
            .on_action(cx.listener(|this, _: &Activate, window, cx| this.activate(window, cx)))
            .on_action(cx.listener(|this, _: &NextFacet, _, cx| this.cycle_facet(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousFacet, _, cx| this.cycle_facet(-1, cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.type_ahead(event, window, cx)
            }))
            .child(chrome)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .pb_8()
                    // A held value that answers the previous filters is dimmed, never blanked.
                    .when(dimmed, |body| body.opacity(0.55))
                    .child(body),
            )
            .children(bar)
    }
}

impl Library {
    /// The listing, or whichever state stands in for it.
    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let empty = match self.filters.facet {
            Facet::Tracks => self.rows.is_empty(),
            _ => self.tiles.is_empty(),
        };
        if let Some(error) = self.shared.failure.clone()
            && empty
        {
            let page = cx.entity().downgrade();
            return states::failed(&error, move |_, _, cx| {
                page.update(cx, |this, cx| this.retry(cx)).ok();
            })
            .into_any_element();
        }
        if self.total() == 0 {
            if !self.shared.settled {
                return self.skeleton_body(cx);
            }
            let liked = Filters::default().held_to_liked();
            if self.liked && self.filters == liked {
                return states::no_favorites(&self.shared.active).into_any_element();
            }
            if self.filters.is_narrowed() {
                let page = cx.entity().downgrade();
                let clear = move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                    page.update(cx, |this, cx| {
                        let cleared = this.filters.cleared();
                        this.search
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        this.set_filters(cleared, true, cx);
                    })
                    .ok();
                };
                let searched_only = Filters {
                    search: String::new(),
                    ..self.filters.clone()
                } == liked;
                if self.liked && searched_only {
                    return states::no_results(self.filters.search.trim(), clear)
                        .into_any_element();
                }
                return states::no_matches(clear).into_any_element();
            }
            return states::empty_library(self.filters.facet, &self.shared.active)
                .into_any_element();
        }
        match self.filters.facet {
            Facet::Tracks => self.table(window, cx),
            _ => self.grid(window, cx),
        }
    }

    /// What stands where the rows will be, sized like the rows so the page does not jump.
    fn skeleton_body(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.filters.facet {
            Facet::Tracks => {
                let shown = components::columns::visible(&self.columns).len();
                let height = style::density_row(self.density);
                let covers = self.density == Density::Comfortable;
                skeleton::rows(14, height, covers, shown, cx).into_any_element()
            }
            facet => v_flex()
                .w_full()
                .gap_6()
                .children((0..3).map(|_| {
                    skeleton::band(
                        self.grid.columns(),
                        px(self.tile.width()),
                        facet.round_tiles(),
                        cx,
                    )
                }))
                .into_any_element(),
        }
    }
}

/// The page's own default, before the route or a chip has said anything.
fn opening_view(cx: &App) -> String {
    let route = Navigator::global(cx).read(cx).current().clone();
    if let Some(view) = route.library_view() {
        return view.to_string();
    }
    store::layout(cx).library_view.clone().unwrap_or_default()
}

/// How many album ids one read asks for, so a reply stays far under the client's message limit.
const META_BATCH: usize = 200;

/// The album ids `rows` reference that nobody has asked about yet, deduplicated, in batches.
fn unasked_albums(rows: &[TrackInfo], asked: &HashSet<String>) -> Vec<Vec<String>> {
    let mut seen = HashSet::new();
    let fresh: Vec<String> = rows
        .iter()
        .map(|row| row.album_id.as_str())
        .filter(|id| !id.is_empty() && !asked.contains(*id) && seen.insert(*id))
        .map(str::to_string)
        .collect();
    fresh.chunks(META_BATCH).map(<[String]>::to_vec).collect()
}

fn album_meta(album: &AlbumInfo) -> (String, AlbumMeta) {
    let meta = AlbumMeta {
        year: album.year,
        genre: album.genre.clone(),
    };
    (album.id.clone(), meta)
}

/// The catalog's first and last year, for the year chip's menu, from one album at each end.
async fn year_span(api: Arc<dyn KopuzApi>) -> Result<Option<(u16, u16)>, api::ApiError> {
    let one = store::paging::page(0, 1);
    let first = api.albums(year_end(SortDirection::Asc), one).await?;
    let last = api.albums(year_end(SortDirection::Desc), one).await?;
    Ok(first
        .albums
        .first()
        .zip(last.albums.first())
        .map(|(first, last)| (first.year, last.year)))
}

fn year_end(direction: SortDirection) -> AlbumQuery {
    AlbumQuery {
        // An album with no year has year 0 and would otherwise open the span.
        year_from: Some(1),
        sort: vec![SortCriterion::new(AlbumSortField::Year, direction)],
        ..Default::default()
    }
}

/// Album ids already read for their genre and year; a page of rows asks only about the new ones.
#[derive(Debug, Default)]
struct MetaAsks {
    asked: HashSet<String>,
    // A failed read gives its ids back, and the next page-in after this asks for them again.
    retry_at: Option<Instant>,
}

impl MetaAsks {
    fn take(&mut self, rows: &[TrackInfo], now: Instant) -> Vec<Vec<String>> {
        if self.retry_at.is_some_and(|at| now < at) {
            return Vec::new();
        }
        let batches = unasked_albums(rows, &self.asked);
        for batch in &batches {
            self.asked.extend(batch.iter().cloned());
        }
        batches
    }

    fn failed(&mut self, batch: &[String], now: Instant) {
        for id in batch {
            self.asked.remove(id);
        }
        self.retry_at = Some(now + META_BACKOFF);
    }

    fn invalidate(&mut self) {
        self.asked.clear();
        self.retry_at = None;
    }
}

const META_BACKOFF: Duration = Duration::from_secs(5);

/// A resync reports every table changed, so it lands here as well as a retag does.
fn meta_stale(event: &store::TablesChanged) -> bool {
    event.touches(&[Table::Albums])
}

/// A reply answers for every id it was asked about: one it leaves out has no album any more.
fn merge_meta(meta: &mut HashMap<String, AlbumMeta>, asked: &[String], albums: &[AlbumInfo]) {
    for id in asked {
        meta.remove(id);
    }
    meta.extend(albums.iter().map(album_meta));
}

/// The order a column the daemon cannot sort by is put in here.
fn compare(
    field: &str,
    left: &TrackInfo,
    right: &TrackInfo,
    plays: &HashMap<String, u64>,
) -> std::cmp::Ordering {
    match field {
        "artist" => text(&left.artist, &right.artist),
        "album" => text(&left.album, &right.album),
        "duration" => left.duration_ms.cmp(&right.duration_ms),
        "plays" => plays
            .get(&left.uid)
            .copied()
            .unwrap_or_default()
            .cmp(&plays.get(&right.uid).copied().unwrap_or_default()),
        "format" => text(
            left.format.as_deref().unwrap_or_default(),
            right.format.as_deref().unwrap_or_default(),
        ),
        _ => text(&left.title, &right.title),
    }
}

fn text(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

#[cfg(test)]
mod paging_tests {
    use super::*;

    #[test]
    fn a_large_listing_asks_for_each_new_album_once_in_small_batches() {
        let rows: Vec<TrackInfo> = (0..50_000)
            .map(|index| TrackInfo {
                album_id: match index % 7 {
                    0 => String::new(),
                    _ => format!("album-{}", index % 5_000),
                },
                ..Default::default()
            })
            .collect();
        let asked: HashSet<String> = (0..1_000).map(|index| format!("album-{index}")).collect();
        let batches = unasked_albums(&rows, &asked);
        assert!(batches.iter().all(|batch| batch.len() <= META_BATCH));
        let ids: Vec<&String> = batches.iter().flatten().collect();
        let unique: HashSet<&String> = ids.iter().copied().collect();
        assert_eq!(ids.len(), 4_000);
        assert_eq!(unique.len(), ids.len());
        assert!(ids.iter().all(|id| !asked.contains(*id)));

        let all: HashSet<String> = unique.into_iter().cloned().chain(asked).collect();
        assert!(unasked_albums(&rows, &all).is_empty());
    }

    fn rows_on(albums: &[&str]) -> Vec<TrackInfo> {
        albums
            .iter()
            .map(|album| TrackInfo {
                album_id: album.to_string(),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn a_failed_read_gives_its_albums_back_after_a_backoff() {
        let rows = rows_on(&["a", "b"]);
        let mut asks = MetaAsks::default();
        let now = Instant::now();
        let batch = asks.take(&rows, now).concat();
        assert_eq!(batch.len(), 2);
        asks.failed(&batch, now);
        assert!(asks.take(&rows, now + Duration::from_secs(1)).is_empty());
        assert_eq!(asks.take(&rows, now + META_BACKOFF).concat().len(), 2);
        assert!(asks.take(&rows, now + META_BACKOFF).is_empty());
    }

    #[test]
    fn a_resync_asks_every_album_again_without_waiting_out_a_failure() {
        assert!(meta_stale(&store::TablesChanged::everything()));
        assert!(!meta_stale(&store::TablesChanged::default()));
        let rows = rows_on(&["a", "b", "c"]);
        let mut asks = MetaAsks::default();
        let now = Instant::now();
        let batch = asks.take(&rows, now).concat();
        asks.failed(&batch[..1], now);
        asks.invalidate();
        assert_eq!(asks.take(&rows, now).concat().len(), 3);
    }

    #[test]
    fn the_year_span_reads_one_dated_album_from_each_end() {
        for (direction, first) in [(SortDirection::Asc, true), (SortDirection::Desc, false)] {
            let query = year_end(direction);
            assert_eq!(query.year_from, Some(1));
            assert_eq!(query.sort.len(), 1);
            assert_eq!(query.sort[0].field, AlbumSortField::Year);
            assert_eq!(query.sort[0].direction == SortDirection::Asc, first);
        }
    }

    #[test]
    fn a_reply_drops_the_albums_it_no_longer_has() {
        let album = |id: &str, genre: &str| AlbumInfo {
            id: id.to_string(),
            genre: genre.to_string(),
            ..Default::default()
        };
        let mut meta = HashMap::from([
            album_meta(&album("a", "Rock")),
            album_meta(&album("b", "Jazz")),
        ]);
        meta.insert("c".to_string(), album_meta(&album("c", "Folk")).1);
        let asked = ["a".to_string(), "b".to_string()];
        merge_meta(&mut meta, &asked, &[album("a", "Folk")]);
        assert_eq!(meta.get("a").map(|meta| meta.genre.as_str()), Some("Folk"));
        assert!(!meta.contains_key("b"));
        assert!(meta.contains_key("c"));
    }

    #[test]
    fn catalog_windows_keep_the_same_indexes_in_both_scroll_directions() {
        let mut pager = Pager::new();
        let total = 347;
        for range in [0..24, 96..120, 204..228, 330..347, 180..204, 0..24] {
            pager.adopt(pager.offset(), 0, total);
            let request = pager.needs(&range).unwrap();
            let end = (request.offset + request.limit).min(total as u32);
            let response = CatalogWindow {
                offset: request.offset as usize,
                page: AlbumPage {
                    albums: (request.offset..end)
                        .map(|index| AlbumInfo {
                            id: format!("album-{index}"),
                            ..Default::default()
                        })
                        .collect(),
                    total: total as u32,
                },
                narrowed: false,
            };
            pager.adopt(
                response.offset,
                response.albums.len(),
                response.total as usize,
            );
            assert_eq!(pager.total().div_ceil(6), 58);
            for index in range {
                assert_eq!(
                    window_item(&response.albums, pager.offset(), index)
                        .unwrap()
                        .id,
                    format!("album-{index}")
                );
            }
            if response.offset > 0 {
                assert!(window_item(&response.albums, pager.offset(), 0).is_none());
            }
            assert!(window_item(&response.albums, pager.offset(), total).is_none());
        }
    }
}
