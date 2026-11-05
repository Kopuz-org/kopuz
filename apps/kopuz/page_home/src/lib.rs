//! Home: the queue to pick back up, and the shelves the layout puts under it.

mod cards;
mod data;
mod for_you;
mod hero;
mod ranked;
mod resume;
mod settings;
mod shelf;

pub use settings::register;

use std::collections::HashMap;
use std::rc::Rc;

use api::{
    AlbumPage, AlbumQuery, ArtistPage, Page, StatsView, Table, TrackFilter, TrackPage, TrackSort,
};
use components::format::t;
use components::{EmptyState, PageHeader, TitleSize, icons, style};
use config::{AlbumSortField, ListenNowStyle, SortCriterion, SortDirection};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, FontWeight, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div,
};
use store::layout::Ordered;
use store::{
    DownloadsModel, FavoritesModel, ForYouModel, LayoutModel, PlaylistsModel, Query, SourcesModel,
};

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
#[derive(Clone, Copy)]
enum Drawn {
    Catalog,
    Shelf(usize),
}

/// A row and where it sits in the arrangement, for edit mode to draw its controls over it.
struct Arranged {
    name: &'static str,
    drawn: Option<Drawn>,
    shown: bool,
    first: bool,
    last: bool,
}

pub struct Home {
    albums: Entity<Query<AlbumPage>>,
    recently_added: Entity<Query<AlbumPage>>,
    new_releases: Entity<Query<AlbumPage>>,
    // The hero album's track keys, which its heart reads, and the album they belong to.
    hero_tracks: Entity<Query<(Option<String>, Vec<String>)>>,
    hero_album: Option<String>,
    // Customize home: every shelf in its place, hidden ones too, with its controls over it.
    editing: bool,
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
        let new_releases = cx.new(|cx| {
            Query::new(
                &[Table::Albums],
                |client| async move {
                    let query = AlbumQuery {
                        sort: vec![SortCriterion::new(
                            AlbumSortField::Year,
                            SortDirection::Desc,
                        )],
                        ..Default::default()
                    };
                    client.albums(query, page(RECENTLY_ADDED_WINDOW)).await
                },
                cx,
            )
        });
        let hero_tracks = cx.new(|cx| {
            Query::new(
                &[Table::Tracks],
                |_| async move { Ok((None, Vec::new())) },
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
            cx.observe(&new_releases, |this, _, cx| this.rebuild(cx)),
            cx.observe(&hero_tracks, |_, _, cx| cx.notify()),
            cx.observe(&FavoritesModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&LayoutModel::global(cx), |_, _, cx| cx.notify()),
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
            new_releases,
            hero_tracks,
            hero_album: None,
            editing: false,
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
            new_releases: self.new_releases.read(cx).value(),
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
        let hero_album = self
            .shelves
            .iter()
            .find(|shelf| shelf.form == data::Form::Featured)
            .and_then(|shelf| shelf.tiles.first())
            .and_then(|tile| match &tile.play {
                data::Play::Album(id) => Some(id.clone()),
                _ => None,
            });
        if hero_album != self.hero_album {
            self.hero_album = hero_album.clone();
            point_hero(&self.hero_tracks, hero_album, cx);
        }
        cx.notify();
    }

    /// Every row the layout knows, hidden ones too, and what each would draw; a shelf with nothing in it draws nothing.
    fn arranged(&self, cx: &Context<Self>) -> Vec<Arranged> {
        // The mix is radio from the source, which offline mode never reaches.
        let for_you = self.shared.for_you.shown && !self.shared.offline;
        let catalog = self.shared.catalog;
        let layout = store::layout(cx);
        let names = match catalog {
            true => with_shown(&layout.catalog_home),
            false => with_shown(&layout.home),
        };
        let count = names.len();
        names
            .into_iter()
            .enumerate()
            .map(|(at, (name, shown))| {
                let drawn = match name {
                    "catalog" => catalog.then_some(Drawn::Catalog),
                    _ => self
                        .shelves
                        .iter()
                        .position(|shelf| shelf.name == name)
                        .map(Drawn::Shelf),
                }
                .filter(|drawn| match drawn {
                    Drawn::Catalog => true,
                    // Waiting and failing are both said in the row, so it stands before it has tiles.
                    Drawn::Shelf(index) if self.shelves[*index].name == "for_you" => for_you,
                    // Edit mode keeps an empty hero as the block its height is dragged on.
                    Drawn::Shelf(index) if self.shelves[*index].form == data::Form::Featured => {
                        self.editing || !self.shelves[*index].tiles.is_empty()
                    }
                    Drawn::Shelf(index) => !self.shelves[*index].tiles.is_empty(),
                });
                Arranged {
                    name,
                    drawn,
                    shown,
                    first: at == 0,
                    last: at + 1 == count,
                }
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
        let editing = self.editing;
        // Edit mode draws the hidden rows too, faded, so they can be shown again where they sit.
        let arranged: Vec<Arranged> = self
            .arranged(cx)
            .into_iter()
            .filter(|row| row.shown || editing)
            .collect();
        // The hero already offers the last session, so the resume banner would say it twice.
        let featured = arranged.iter().any(|row| {
            row.shown
                && matches!(row.drawn, Some(Drawn::Shelf(index))
                    if self.shelves[index].form == data::Form::Featured
                        && !self.shelves[index].tiles.is_empty())
        });
        let banner = match featured {
            true => None,
            false => shared.resume.as_ref().map(resume::banner),
        };
        let asks = arranged.iter().any(|row| {
            matches!(row.drawn, Some(Drawn::Shelf(index)) if self.shelves[index].name == "for_you")
        });
        let catalog = match arranged
            .iter()
            .any(|row| matches!(row.drawn, Some(Drawn::Catalog)))
        {
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
        let layout = store::layout(cx);
        let (hero_height, listed) = (
            layout.ui.hero_height,
            layout.ui.listen_now_style == ListenNowStyle::List,
        );
        // Keys fetched for another album leave the heart off until this one's arrive.
        let hero_keys = match self.hero_tracks.read(cx).value() {
            Some((album, keys)) if *album == self.hero_album => keys.clone(),
            _ => Vec::new(),
        };
        let mut rows: Vec<AnyElement> = Vec::with_capacity(arranged.len());
        // The chips head For you, or the feed where For you is not shown, whichever comes first.
        let mut chips = catalog.as_ref().map(|views| views.chips.clone());
        let headed = |chips: &mut Option<AnyView>, row: AnyElement| match chips.take() {
            Some(chips) => v_flex().child(chips).child(row).into_any_element(),
            None => row,
        };
        for row in &arranged {
            let drawn: Option<AnyElement> = match row.drawn {
                None => None,
                Some(Drawn::Catalog) => catalog
                    .as_ref()
                    .map(|views| headed(&mut chips, views.feed.clone().into_any_element())),
                Some(Drawn::Shelf(index)) => {
                    let shelf = &self.shelves[index];
                    let handle = self.scroll.get(shelf.name).cloned().unwrap_or_default();
                    // A ranking is a list and a shelf is a row; `Form` is what says which a name gets.
                    Some(match shelf.form {
                        data::Form::Featured => hero::hero(
                            hero::Hero {
                                tile: shelf.tiles.first(),
                                height: hero_height,
                                keys: &hero_keys,
                                editing,
                            },
                            window,
                            cx,
                        ),
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
                        _ if shelf.name == "listen_now" && listed => {
                            shelf::rows(shelf, &shared.now, cx)
                        }
                        _ => shelf::shelf(shelf, &handle, &shared.now),
                    })
                }
            };
            match editing {
                true => rows.push(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(self.strip(row, cx))
                        .children(drawn.map(|drawn| {
                            div()
                                .w_full()
                                .when(!row.shown, |hidden| hidden.opacity(HIDDEN))
                                .child(drawn)
                        }))
                        .into_any_element(),
                ),
                false => rows.extend(drawn),
            }
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
        let heading = self.heading(cx);
        v_flex()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.page)
            .p_8()
            .pb_32()
            .gap_12()
            .child(heading)
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

/// `opacity-40`, a hidden row in edit mode.
const HIDDEN: f32 = 0.4;

impl Home {
    /// `text-4xl font-black` "Home", and Customize home, which turns into Reset and Done while arranging.
    fn heading(&self, cx: &mut Context<Self>) -> AnyElement {
        let catalog = self.shared.catalog;
        let actions = match self.editing {
            false => h_flex().child(
                pill("home-customize", icons::SETTINGS, "customize_home", cx).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.editing = true;
                        cx.notify();
                    }),
                ),
            ),
            true => h_flex()
                .gap_2()
                .child(
                    pill("home-reset", icons::REFRESH, "reset_home", cx).on_click(
                        move |_, _, cx| {
                            LayoutModel::edit(cx, |layout| match catalog {
                                true => layout.catalog_home = Default::default(),
                                false => layout.home = Default::default(),
                            })
                        },
                    ),
                )
                .child(
                    Button::new("home-done")
                        .small()
                        .primary()
                        .rounded_full()
                        .icon(icons::icon(icons::CHECK))
                        .label(t("done_editing"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.editing = false;
                            cx.notify();
                        })),
                ),
        };
        PageHeader::new(t("home"))
            .size(TitleSize::Display)
            .flush()
            .action(actions)
            .into_any_element()
    }

    /// One row's controls in edit mode: its name, a step either way, show or hide, and Listen Now's layout.
    fn strip(&self, row: &Arranged, cx: &mut Context<Self>) -> AnyElement {
        let (name, shown, catalog) = (row.name, row.shown, self.shared.catalog);
        let step = |delta: isize, glyph: &'static str, tip: &'static str, enabled: bool| {
            Button::new(SharedString::from(format!("home-edit-{name}-{delta}")))
                .small()
                .custom(style::quiet_button(style::slate_400(cx), cx))
                .icon(icons::icon(glyph))
                .tooltip(t(tip))
                .disabled(!enabled)
                .on_click(move |_, _, cx| arrange(catalog, cx, |list| list.step(name, delta)))
        };
        let visibility = Button::new(SharedString::from(format!("home-edit-{name}-shown")))
            .small()
            .custom(match shown {
                true => style::tint_button(cx.theme().primary, cx),
                false => style::soft_button(cx.theme().foreground, cx),
            })
            .label(t(match shown {
                true => "hide_section",
                false => "show_section",
            }))
            .on_click(move |_, _, cx| arrange(catalog, cx, |list| list.show(name, !shown)));
        let listen_now = (name == "listen_now").then(|| {
            let cards = store::layout(cx).ui.listen_now_style == ListenNowStyle::Cards;
            Button::new("home-edit-listen-now-style")
                .small()
                .custom(style::soft_button(cx.theme().foreground, cx))
                .icon(icons::icon(match cards {
                    true => icons::GRID,
                    false => icons::ROWS,
                }))
                .label(t(match cards {
                    true => "layout_cards",
                    false => "layout_list",
                }))
                .tooltip(t("listen_now_layout"))
                .on_click(move |_, _, cx| {
                    LayoutModel::edit(cx, |layout| {
                        layout.ui.listen_now_style = match cards {
                            true => ListenNowStyle::List,
                            false => ListenNowStyle::Cards,
                        }
                    })
                })
        });
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(style::radius_lg())
            .border_1()
            .border_color(style::white(style::BORDER, cx))
            .bg(style::white(style::RAISED, cx))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(style::white(style::TEXT_STRONG, cx))
                    .child(t(shelf::title(name))),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .children(listen_now)
                    .child(step(-1, icons::CHEVRON_UP, "move_up", !row.first))
                    .child(step(1, icons::CHEVRON_DOWN, "move_down", !row.last))
                    .child(visibility),
            )
            .into_any_element()
    }
}

fn with_shown<L: Ordered>(list: &L) -> Vec<(&'static str, bool)> {
    list.arranged()
        .into_iter()
        .map(|name| (name, list.shows(name)))
        .collect()
}

/// Change whichever Home arrangement the active source draws.
fn arrange(catalog: bool, cx: &mut App, edit: impl FnOnce(&mut dyn Arrangement)) {
    LayoutModel::edit(cx, |layout| match catalog {
        true => edit(&mut layout.catalog_home),
        false => edit(&mut layout.home),
    });
}

/// The two edits a strip makes, over either Home arrangement.
trait Arrangement {
    fn step(&mut self, name: &str, delta: isize);
    fn show(&mut self, name: &str, shown: bool);
}

impl<L: Ordered> Arrangement for L {
    fn step(&mut self, name: &str, delta: isize) {
        Ordered::step(self, name, delta);
    }

    fn show(&mut self, name: &str, shown: bool) {
        Ordered::show(self, name, shown);
    }
}

fn pill(
    id: &'static str,
    glyph: &'static str,
    label: &'static str,
    cx: &App,
) -> gpui_kit::component::button::Button {
    Button::new(id)
        .small()
        .rounded_full()
        .custom(style::raised_button(cx.theme().foreground, cx))
        .icon(icons::icon(glyph))
        .label(t(label))
}

fn point_hero(
    query: &Entity<Query<(Option<String>, Vec<String>)>>,
    album: Option<String>,
    cx: &mut App,
) {
    query.update(cx, |query, cx| {
        query.set_fetch(
            move |api| {
                let album = album.clone();
                async move {
                    let Some(id) = album.clone() else {
                        return Ok((None, Vec::new()));
                    };
                    let page = api.album_tracks(id, store::all()).await?;
                    Ok((
                        album,
                        page.items.into_iter().map(|track| track.key).collect(),
                    ))
                }
            },
            cx,
        )
    });
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
