//! Home: the queue to pick back up, and the shelves the layout puts under it.

mod cards;
mod data;
mod for_you;
mod ranked;
mod resume;
mod settings;
mod shelf;

pub use settings::register;

use std::collections::HashMap;
use std::rc::Rc;

use api::{AlbumPage, ArtistPage, Page, StatsView, Table, TrackFilter, TrackPage, TrackSort};
use components::format::t;
use components::{EmptyState, PageHeader, TitleSize, icons};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window,
};
use store::layout::Ordered as _;
use store::{DownloadsModel, ForYouModel, PlaylistsModel, Query, SourcesModel};

use data::{RECENT_TRACKS_WINDOW, RECENTLY_ADDED_WINDOW, Shelf};

/// A source's feed for Home, and its chips, which Home draws over its first row rather than in the feed.
#[derive(Clone)]
pub struct CatalogViews {
    pub chips: AnyView,
    pub feed: AnyView,
    /// Whether For you wears the feed's opening line, or the feed keeps it.
    pub strapline_to_host: HandStrapline,
}

pub type HandStrapline = Rc<dyn Fn(bool, &mut App)>;

type BuildCatalog =
    Rc<dyn Fn(Option<SharedString>, ScrollHandle, &mut Window, &mut App) -> CatalogViews>;

struct CatalogBuilder(BuildCatalog);

impl Global for CatalogBuilder {}

/// Register what draws a source's own feed as Home's catalog section: its first page by id, or its plain feed with none.
pub fn register_catalog(
    build: impl Fn(Option<SharedString>, ScrollHandle, &mut Window, &mut App) -> CatalogViews + 'static,
    cx: &mut App,
) {
    cx.set_global(CatalogBuilder(Rc::new(build)));
}

/// What the page draws from shared models, read each frame until the page is frozen.
#[derive(Clone, Default)]
pub(crate) struct Shared {
    resume: Option<resume::Resume>,
    pub(crate) now: cards::Now,
    pub(crate) for_you: for_you::Status,
    offline: bool,
    catalog: bool,
    settled: bool,
    // The feed's opening line, which For you wears instead.
    pub(crate) strapline: Option<SharedString>,
}

impl Shared {
    fn read(home: &Home, cx: &App) -> Self {
        let sources = SourcesModel::global(cx).read(cx);
        let strapline = sources
            .has_catalog()
            .then(|| {
                let first = sources
                    .capabilities()
                    .pages
                    .first()
                    .map(|page| page.id.clone());
                store::CatalogCache::strapline(&first.unwrap_or_default(), cx)
            })
            .flatten()
            .map(SharedString::from);
        Self {
            strapline,
            resume: resume::Resume::read(cx),
            now: cards::Now::read(cx),
            for_you: for_you::Status::read(cx),
            offline: sources.offline_reads(),
            catalog: sources.has_catalog(),
            settled: home.albums.read(cx).is_settled(),
        }
    }
}

/// One row of the page, top to bottom.
enum Drawn {
    Catalog,
    Shelf(usize),
}

pub struct Home {
    albums: Entity<Query<AlbumPage>>,
    recently_added: Entity<Query<AlbumPage>>,
    artists: Entity<Query<ArtistPage>>,
    recents: Entity<Query<TrackPage>>,
    // A ranking needs both the counts and the tracks they name.
    stats: Entity<Query<StatsView>>,
    played: Entity<Query<TrackPage>>,
    genre: Entity<Query<Option<String>>>,
    shelves: Vec<Shelf>,
    // One scroller per shelf, kept across rebuilds so new data does not throw a row back to its start.
    scroll: HashMap<&'static str, ScrollHandle>,
    page: ScrollHandle,
    // The catalog section, and the source and page it was built for.
    catalog: Option<(String, CatalogViews)>,
    // The feed the strapline was last handed over for, and whether it was.
    handed: Option<(gpui_kit::EntityId, bool)>,
    seed: u64,
    // The For you row's fade over its placeholder, kept here so a revisit finds it already in.
    for_you: components::motion::Fade,
    // The last mix drawn, for the row to fade out once a refresh lets it go.
    for_you_last: Rc<[data::Tile]>,
    // The last frame's reads of shared models, which a frozen page keeps drawing.
    shared: Shared,
    frozen: bool,
    _subscriptions: Vec<Subscription>,
}

impl store::Freeze for Home {
    fn freeze(&mut self, _: &mut Context<Self>) {
        self.frozen = true;
        self._subscriptions.clear();
    }
}

impl Home {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let albums = cx.new(|cx| {
            Query::new(
                &[Table::Albums],
                |client| async move { client.albums(Default::default(), store::all()).await },
                cx,
            )
        });
        let recently_added = cx.new(|cx| {
            Query::new(
                &[Table::Albums, Table::Tracks],
                |client| async move {
                    client
                        .albums_recently_added(page(RECENTLY_ADDED_WINDOW))
                        .await
                },
                cx,
            )
        });
        let artists = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Artists],
                |client| async move { client.artists(Default::default(), store::all()).await },
                cx,
            )
        });
        let recents = cx.new(|cx| {
            Query::new(
                &[Table::Recents],
                |client| async move { client.recent_tracks(page(RECENT_TRACKS_WINDOW)).await },
                cx,
            )
        });
        let stats = cx.new(|cx| {
            Query::new(
                &[Table::Recents],
                |client| async move { client.stats().await },
                cx,
            )
        });
        // The daemon can order by play count, so the most-played tracks are one bounded read.
        let played = cx.new(|cx| {
            Query::new(
                &[Table::Recents, Table::Tracks],
                |client| async move {
                    let filter = TrackFilter {
                        sort: TrackSort::PlayCount,
                        ..Default::default()
                    };
                    client.tracks(filter, page(PLAYED_WINDOW)).await
                },
                cx,
            )
        });
        let genre = cx.new(|cx| {
            Query::new(
                &[Table::Tracks],
                |client| async move { client.top_genre().await },
                cx,
            )
        });

        // Only what the shelves are derived from is observed: any notify at all redraws the page.
        let subscriptions = vec![
            cx.observe(&albums, |this, _, cx| this.rebuild(cx)),
            cx.observe(&recently_added, |this, _, cx| this.rebuild(cx)),
            cx.observe(&artists, |this, _, cx| this.rebuild(cx)),
            cx.observe(&recents, |this, _, cx| this.rebuild(cx)),
            cx.observe(&stats, |this, _, cx| this.rebuild(cx)),
            cx.observe(&played, |this, _, cx| this.rebuild(cx)),
            cx.observe(&genre, |this, _, cx| this.rebuild(cx)),
            cx.observe(&PlaylistsModel::global(cx), |this, _, cx| this.rebuild(cx)),
            cx.observe(&SourcesModel::global(cx), |this, _, cx| this.rebuild(cx)),
            cx.observe(&ForYouModel::global(cx), |this, _, cx| this.rebuild(cx)),
            cx.observe(&DownloadsModel::global(cx), |this, _, cx| this.rebuild(cx)),
        ];

        Self {
            albums,
            recently_added,
            artists,
            recents,
            stats,
            played,
            genre,
            shelves: Vec::new(),
            scroll: store::layout::SHELVES
                .iter()
                .map(|name| (*name, ScrollHandle::default()))
                .collect(),
            page: ScrollHandle::default(),
            catalog: None,
            handed: None,
            seed: seed(),
            for_you: components::motion::Fade::hidden(),
            for_you_last: Rc::from([]),
            shared: Shared::default(),
            frozen: false,
            _subscriptions: subscriptions,
        }
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let mut playlists = PlaylistsModel::global(cx).read(cx).playlists().to_vec();
        if SourcesModel::global(cx).read(cx).offline_reads() {
            let downloads = DownloadsModel::global(cx);
            let downloads = downloads.read(cx);
            playlists.retain(|playlist| {
                playlist
                    .track_keys
                    .iter()
                    .any(|key| downloads.is_stored(key))
            });
        }
        let for_you = ForYouModel::global(cx);
        let for_you = for_you.read(cx).shown_mix().unwrap_or(&[]);
        self.shelves = data::build(data::Inputs {
            albums: self.albums.read(cx).value(),
            recently_added: self.recently_added.read(cx).value(),
            artists: self.artists.read(cx).value(),
            recents: self.recents.read(cx).value(),
            played: self.played.read(cx).value(),
            plays: self
                .stats
                .read(cx)
                .value()
                .map(|stats| &stats.listen_counts),
            genre: self.genre.read(cx).value().and_then(|genre| genre.as_ref()),
            playlists: &playlists,
            for_you,
            seed: self.seed,
        });
        if let Some(mix) = self
            .shelves
            .iter()
            .find(|shelf| shelf.name == "for_you" && !shelf.tiles.is_empty())
        {
            self.for_you_last = mix.tiles.clone();
        }
        cx.notify();
    }

    /// The rows the layout asks for, in its order, skipping the shelves with nothing in them.
    fn drawn(&self, cx: &Context<Self>) -> Vec<Drawn> {
        // The mix is radio from the source, which offline mode never reaches.
        let for_you = self.shared.for_you.shown && !self.shared.offline;
        let catalog = self.shared.catalog;
        let layout = store::layout(cx);
        let names = match catalog {
            true => layout.catalog_home.visible(),
            false => layout.home.visible(),
        };
        names
            .into_iter()
            .filter_map(|name| match name {
                "catalog" => catalog.then_some(Drawn::Catalog),
                _ => self
                    .shelves
                    .iter()
                    .position(|shelf| shelf.name == name)
                    .map(Drawn::Shelf),
            })
            .filter(|drawn| match drawn {
                Drawn::Catalog => true,
                // Waiting and failing are both said in the row, so it stands before it has tiles.
                Drawn::Shelf(index) if self.shelves[*index].name == "for_you" => for_you,
                Drawn::Shelf(index) => !self.shelves[*index].tiles.is_empty(),
            })
            .collect()
    }

    /// The source's own feed, built once per source and first page.
    fn catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<CatalogViews> {
        if self.frozen {
            return self.catalog.as_ref().map(|(_, views)| views.clone());
        }
        let build = cx.try_global::<CatalogBuilder>()?.0.clone();
        let sources = SourcesModel::global(cx).read(cx);
        let source = sources.active()?.id.clone();
        let first = sources
            .capabilities()
            .pages
            .first()
            .map(|page| SharedString::from(page.id.clone()));
        let key = format!("{source}\u{0}{}", first.as_deref().unwrap_or_default());
        if let Some((held, views)) = &self.catalog
            && *held == key
        {
            return Some(views.clone());
        }
        let views = build(first, self.page.clone(), window, cx);
        self.catalog = Some((key, views.clone()));
        Some(views)
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("home");
        components::scroll::smooth("home-scroll", &self.page, window, cx);
        if !self.frozen {
            self.shared = Shared::read(self, cx);
        }
        let shared = self.shared.clone();
        let banner = shared.resume.as_ref().map(resume::banner);
        let drawn = self.drawn(cx);
        let asks = drawn.iter().any(
            |drawn| matches!(drawn, Drawn::Shelf(index) if self.shelves[*index].name == "for_you"),
        );
        let catalog = match drawn.iter().any(|drawn| matches!(drawn, Drawn::Catalog)) {
            true => self.catalog(window, cx),
            false => None,
        };
        if let Some(views) = &catalog {
            let handed = (views.feed.entity_id(), asks);
            if self.handed != Some(handed) {
                self.handed = Some(handed);
                let hand = views.strapline_to_host.clone();
                cx.defer(move |cx| hand(asks, cx));
            }
        }
        let mut rows: Vec<AnyElement> = Vec::with_capacity(drawn.len());
        // The chips head For you, or the feed where For you is not shown, whichever comes first.
        let mut chips = catalog.as_ref().map(|views| views.chips.clone());
        let headed = |chips: &mut Option<AnyView>, row: AnyElement| match chips.take() {
            Some(chips) => v_flex().child(chips).child(row).into_any_element(),
            None => row,
        };
        for drawn in drawn {
            let index = match drawn {
                Drawn::Catalog => {
                    if let Some(views) = &catalog {
                        rows.push(headed(&mut chips, views.feed.clone().into_any_element()));
                    }
                    continue;
                }
                Drawn::Shelf(index) => index,
            };
            let shelf = &self.shelves[index];
            let Some(handle) = self.scroll.get(shelf.name).cloned() else {
                continue;
            };
            // A ranking is a list and a shelf is a row; `Form` is what says which a name gets.
            rows.push(match shelf.form {
                data::Form::Ranked => ranked::ranked(shelf, cx),
                _ if shelf.name == "for_you" => headed(
                    &mut chips,
                    for_you::row(
                        shelf,
                        &shared,
                        &self.for_you_last,
                        &handle,
                        &mut self.for_you,
                        window,
                        cx,
                    ),
                ),
                _ => shelf::shelf(shelf, &handle, &shared.now),
            });
        }
        // Asked only once the row is on screen, and outside the frame being drawn.
        if asks && !self.frozen && shared.for_you.idle {
            let model = ForYouModel::global(cx);
            cx.defer(move |cx| model.update(cx, |model, cx| model.ensure(cx)));
        }
        // Nothing to show is only worth saying once the library has actually answered.
        let settled = rows.is_empty() && shared.settled;
        let offline = shared.offline;
        // The queue to resume is not a library, so offline it does not hide that nothing else plays.
        let bare = settled && (offline || banner.is_none());
        v_flex()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.page)
            .p_8()
            .pb_32()
            .gap_12()
            .child(heading(cx))
            .children(banner)
            .children(rows)
            .when(bare && offline, |page| page.child(EmptyState::offline()))
            .when(bare && !offline, |page| {
                page.child(
                    EmptyState::new(t("no_albums_found"))
                        .icon(icons::DISC)
                        .detail(t("add_music_to_get_started"))
                        .action(t("add_source"), |_, _, cx| {
                            store::SetupModel::start_over(cx)
                        }),
                )
            })
    }
}

/// `text-4xl font-black` "Home"; its shelves are arranged in Settings → Interface.
fn heading(_: &App) -> AnyElement {
    PageHeader::new(t("home"))
        .size(TitleSize::Display)
        .flush()
        .into_any_element()
}

/// How many of the most-played tracks are resolved to name their artists.
const PLAYED_WINDOW: u32 = 200;

fn page(limit: u32) -> Page {
    Page { offset: 0, limit }
}

/// A per-visit seed, so the shuffled rows keep their order while the page is open.
fn seed() -> u64 {
    use std::hash::{BuildHasher as _, Hasher as _};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}
