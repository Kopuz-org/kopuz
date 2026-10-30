//! Catalog shelves: every layout a catalog page lays its shelves out in, and the sideways rows Discover previews them as.

use std::collections::HashSet;

use api::{CatalogDetailRequest, CatalogItem, CatalogItemKind, CatalogShelf, ShelfLayout};
use components::format::t;
use components::{
    Arrows, Carousel, Cover, PlayState, SectionHeading, Tile, TileRow, TrackActions, TrackList,
    TrackListEvent, TrackListOptions, TrackSection, heading_link, icons, motion, style,
};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, Rgba, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div, px,
};
use store::{
    Link, Navigator, Notice, Notices, Pictured, PlayerModel, Route, SourcesModel, Target, actions,
};

use crate::states::catalog_failure;

pub(crate) const TILE: Pixels = px(176.);
pub(crate) const GAP: Pixels = px(20.);
/// A track-grid column: four rows of a song each, `w-80` as upstream lays them out.
const TRACK_COLUMN: Pixels = px(320.);
const TRACK_GRID_ROWS: usize = 4;
/// A mood button in a grid; wide enough for "Feel good" and a little air.
const MOOD_BUTTON: Pixels = px(208.);
const HERO_COVER: Pixels = px(128.);
const HERO_CARD: Pixels = px(320.);

/// A shelf's paging asked for: the page wants the shelf at this index topped up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoreOf(pub usize);

/// What a Discover row says over itself, and where its "See all" goes.
struct RowHead {
    title: SharedString,
    see_all: Option<Route>,
}

struct ShelfView {
    key: SharedString,
    shelf: CatalogShelf,
    scroll: ScrollHandle,
    list: Option<Entity<TrackList>>,
    // Set for a Discover row, which is one sideways row whatever layout its shelf came in.
    row: Option<RowHead>,
}

/// The shelves of one catalog page, with the tile that started playback remembered.
pub struct Shelves {
    views: Vec<ShelfView>,
    // The catalog id or track key whose tile owns the pause overlay.
    now_playing: Option<String>,
    pending: Option<String>,
    // A page that pages its shelves offers "Show more" under one that has more.
    offer_more: bool,
    topping_up: Option<usize>,
    _play: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MoreOf> for Shelves {}

impl Shelves {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let player = PlayerModel::global(cx);
        Self {
            views: Vec::new(),
            now_playing: None,
            pending: None,
            offer_more: false,
            topping_up: None,
            _play: None,
            _subscriptions: vec![cx.observe(&player, |_, _, cx| cx.notify())],
        }
    }

    pub fn offer_more(mut self, offer: bool) -> Self {
        self.offer_more = offer;
        self
    }

    pub fn set(&mut self, shelves: Vec<CatalogShelf>, cx: &mut Context<Self>) {
        self.views.retain(|view| view.row.is_some());
        self.topping_up = None;
        self.extend(shelves, cx);
    }

    pub fn extend(&mut self, shelves: Vec<CatalogShelf>, cx: &mut Context<Self>) {
        for shelf in shelves {
            let index = self.views.len();
            let list = self.list_for(&shelf, cx);
            self.views.push(ShelfView {
                key: SharedString::from(format!("shelf-{index}")),
                shelf,
                scroll: ScrollHandle::new(),
                list,
                row: None,
            });
        }
        cx.notify();
    }

    /// More items for the page shelf at `index`, and its next cursor.
    pub fn top_up(
        &mut self,
        index: usize,
        items: Vec<CatalogItem>,
        continuation: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.page_view_mut(index) else {
            return;
        };
        view.shelf.items.extend(items);
        view.shelf.continuation = continuation;
        if let Some(list) = view.list.clone() {
            let rows = songs(&view.shelf);
            list.update(cx, |list, cx| list.set_rows(rows, cx));
        }
        cx.notify();
    }

    /// Which page shelf is being topped up, for its "Show more" to say so.
    pub fn set_topping_up(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.topping_up = index;
        cx.notify();
    }

    /// Hold `shelf` as the Discover row `key`, replacing what that row held.
    pub fn put_row(
        &mut self,
        key: impl Into<SharedString>,
        title: SharedString,
        shelf: CatalogShelf,
        see_all: Option<Route>,
        cx: &mut Context<Self>,
    ) {
        let key = key.into();
        let head = Some(RowHead { title, see_all });
        match self.views.iter_mut().find(|view| view.key == key) {
            Some(view) => {
                view.shelf = shelf;
                view.row = head;
            }
            None => self.views.push(ShelfView {
                key,
                shelf,
                scroll: ScrollHandle::new(),
                list: None,
                row: head,
            }),
        }
        cx.notify();
    }

    /// The Discover row `key`, drawn sideways; `None` until it is held.
    pub fn row(&self, key: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let index = self.views.iter().position(|view| view.key == key)?;
        Some(self.carousel_section(index, cx))
    }

    fn page_view_mut(&mut self, index: usize) -> Option<&mut ShelfView> {
        self.views
            .iter_mut()
            .filter(|view| view.row.is_none())
            .nth(index)
    }

    fn list_for(
        &mut self,
        shelf: &CatalogShelf,
        cx: &mut Context<Self>,
    ) -> Option<Entity<TrackList>> {
        let all_songs =
            !shelf.items.is_empty() && shelf.items.iter().all(|item| item.track.is_some());
        (shelf.layout == ShelfLayout::List && all_songs).then(|| self.track_list(shelf, cx))
    }

    fn track_list(&mut self, shelf: &CatalogShelf, cx: &mut Context<Self>) -> Entity<TrackList> {
        let rows = songs(shelf);
        let list = cx.new(|cx| {
            let mut list = TrackList::new(cx)
                .with_options(TrackListOptions {
                    show_index: true,
                    show_cover: true,
                    show_album: false,
                    track_numbers: false,
                    selectable: false,
                })
                .with_actions(TrackActions::default());
            list.set_rows(rows, cx);
            list
        });
        commands::bind_track_list(&list, cx);
        self._subscriptions.push(
            cx.subscribe(&list, |this, list, event: &TrackListEvent, cx| {
                if let TrackListEvent::Play(index) = event {
                    let keys = list.read(cx).keys();
                    actions::play_tracks(keys, Some(*index as u32), cx);
                    // A song preview is nobody's tile, so no card keeps the pause overlay.
                    this.now_playing = None;
                    cx.notify();
                }
            }),
        );
        list
    }

    // TODO(prefetch): §2.3's 250 ms hover prefetch cache is not ported; every play pages fresh.
    /// Play everything behind a catalog id: the first page starts the queue, the rest append.
    pub fn play(&mut self, kind: CatalogItemKind, id: String, cx: &mut Context<Self>) {
        self.now_playing = Some(id.clone());
        self.pending = Some(id.clone());
        cx.notify();
        self._play = Some(cx.spawn(async move |this, cx| {
            let mut seen = HashSet::new();
            let mut cursor: Option<String> = None;
            let mut started = false;
            loop {
                let request = CatalogDetailRequest {
                    continuation: cursor.clone(),
                    ..CatalogDetailRequest::new(kind, id.clone())
                };
                let Ok(call) = this.update(cx, |_, cx| {
                    Link::global(cx)
                        .call(move |api| async move { api.catalog_detail(request).await })
                }) else {
                    return;
                };
                let detail = match call.await {
                    Ok(detail) => detail,
                    Err(error) => {
                        this.update(cx, |this, cx| {
                            if started {
                                Notices::push(Notice::failure(&error, catalog_failure(&error)), cx);
                            } else {
                                this.now_playing = None;
                            }
                            this.pending = None;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                };
                let fresh: Vec<String> = detail
                    .tracks
                    .into_iter()
                    .map(|track| track.key)
                    .filter(|key| seen.insert(key.clone()))
                    .collect();
                if !fresh.is_empty() {
                    let first = !started;
                    started = true;
                    if this
                        .update(cx, |this, cx| {
                            if first {
                                actions::play_tracks(fresh, None, cx);
                                this.pending = None;
                                cx.notify();
                            } else {
                                actions::append(fresh, cx);
                            }
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                match detail.continuation {
                    // A source that hands back the cursor it was given would page forever.
                    Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                    _ => break,
                }
            }
            this.update(cx, |this, cx| {
                this.pending = None;
                if !started {
                    this.now_playing = None;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn toggle_or_play(&mut self, kind: CatalogItemKind, id: String, cx: &mut Context<Self>) {
        if self.pending.as_deref() == Some(id.as_str()) {
            return;
        }
        if self.now_playing.as_deref() == Some(id.as_str()) {
            PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx));
            return;
        }
        self.play(kind, id, cx);
    }

    /// A song starts a radio where the source runs one, else plays alone; pressing it again pauses.
    fn start_song(&mut self, key: String, radio: bool, cx: &mut Context<Self>) {
        if self.now_playing.as_deref() == Some(key.as_str()) {
            PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx));
            return;
        }
        self.now_playing = Some(key.clone());
        if radio {
            actions::start_track_radio(key, cx);
        } else {
            actions::play_tracks(vec![key], None, cx);
        }
        cx.notify();
    }

    fn section(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.views[index];
        let body = match (&view.list, view.shelf.layout) {
            (Some(list), _) => return self.list_section(index, list.clone(), cx),
            (None, ShelfLayout::Carousel) => return self.carousel_section(index, cx),
            (None, ShelfLayout::TrackGrid) => return self.track_grid_section(index, cx),
            (None, ShelfLayout::Grid) => self.grid(index, cx),
            (None, ShelfLayout::Hero) => self.hero(index, cx),
            (None, ShelfLayout::List) => self.item_rows(index, &view.shelf.items, cx),
        };
        v_flex()
            .w_full()
            .gap_6()
            .child(self.heading(index))
            .child(body)
            .children(self.show_more(index, cx))
            .into_any_element()
    }

    fn heading(&self, index: usize) -> SectionHeading {
        let view = &self.views[index];
        let title = SharedString::from(view.shelf.title.clone());
        let mut heading = SectionHeading::new(title).strapline(view.shelf.strapline.clone());
        if let Some(route) = more_route(&view.shelf) {
            heading = heading.trailing(show_all(&view.key, route));
        }
        heading
    }

    /// "Show more" under a page shelf whose source has more of it.
    fn show_more(&self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = &self.views[index];
        if !self.offer_more || view.row.is_some() || view.shelf.continuation.is_none() {
            return None;
        }
        let at = self.page_index(index)?;
        if self.topping_up == Some(at) {
            return Some(
                components::Loading::new()
                    .label(t("discover_more_loading"))
                    .compact()
                    .into_any_element(),
            );
        }
        Some(
            div()
                .flex()
                .justify_center()
                .child(
                    heading_link(
                        SharedString::from(format!("{}-more", view.key)),
                        t("discover_show_more"),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(MoreOf(at)))),
                )
                .into_any_element(),
        )
    }

    fn page_index(&self, index: usize) -> Option<usize> {
        let view = self.views.get(index)?;
        view.row.as_ref().map_or(Some(()), |_| None)?;
        Some(
            self.views[..index]
                .iter()
                .filter(|view| view.row.is_none())
                .count(),
        )
    }

    fn carousel_section(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.views[index];
        let page = cx.entity();
        let (title, strapline, link) = match &view.row {
            Some(head) => (
                head.title.clone(),
                None,
                head.see_all
                    .clone()
                    .map(|route| show_all(&view.key, route).label(t("discover_show_all"))),
            ),
            None => (
                view.shelf.title.clone().into(),
                view.shelf.strapline.clone(),
                more_route(&view.shelf).map(|route| show_all(&view.key, route)),
            ),
        };
        Carousel::new(
            SharedString::from(format!("catalog-{}", view.key)),
            title,
            view.shelf.items.len(),
            TILE,
            // The carousel builds a tile once it is in view, after this render has handed the page back.
            move |at, _, cx| {
                page.update(cx, |this, cx| {
                    let view = &this.views[index];
                    this.tile(&view.key, &view.shelf.items[at], cx)
                })
            },
        )
        .gap(GAP)
        .strapline(strapline)
        .arrows(Arrows::Always)
        .grow_arrows(true)
        .handle(view.scroll.clone())
        .when_some(link, |carousel, link| carousel.action(link))
        .into_any_element()
    }

    fn list_section(
        &self,
        index: usize,
        list: Entity<TrackList>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = &self.views[index];
        let mut section =
            TrackSection::new(SharedString::from(format!("catalog-{}", view.key)), list).heading(
                SectionHeading::new(SharedString::from(view.shelf.title.clone())),
            );
        if let Some(route) = more_route(&view.shelf) {
            section = section.trailing(show_all(&view.key, route));
        }
        v_flex()
            .w_full()
            .gap_4()
            .child(section)
            .children(self.show_more(index, cx))
            .into_any_element()
    }

    /// Four songs to a column, the columns sliding sideways: "Quick picks".
    fn track_grid_section(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.views[index];
        let page = cx.entity();
        let columns = view.shelf.items.len().div_ceil(TRACK_GRID_ROWS);
        Carousel::new(
            SharedString::from(format!("catalog-{}", view.key)),
            SharedString::from(view.shelf.title.clone()),
            columns,
            TRACK_COLUMN,
            move |column, _, cx| {
                page.update(cx, |this, cx| {
                    let items = &this.views[index].shelf.items;
                    let from = column * TRACK_GRID_ROWS;
                    let to = (from + TRACK_GRID_ROWS).min(items.len());
                    v_flex()
                        .w(TRACK_COLUMN)
                        .child(this.item_rows(index, &items[from..to], cx))
                        .into_any_element()
                })
            },
        )
        .gap(px(24.))
        .strapline(view.shelf.strapline.clone())
        .arrows(Arrows::Always)
        .handle(view.scroll.clone())
        .when_some(more_route(&view.shelf), |carousel, route| {
            carousel.action(show_all(&view.key, route))
        })
        .into_any_element()
    }

    /// A shelf laid out in rows that wrap; a shelf of bare page links is a grid of buttons.
    fn grid(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.views[index];
        let buttons = view.shelf.items.iter().all(|item| item.artwork.is_none());
        let cells: Vec<AnyElement> = view
            .shelf
            .items
            .iter()
            .map(|item| match buttons {
                true => page_button(&view.key, item, MOOD_BUTTON, cx),
                false => self.tile(&view.key, item, cx),
            })
            .collect();
        match buttons {
            true => div().flex().flex_wrap().gap_3().children(cells),
            false => components::tile_grid().gap_x(GAP).children(cells),
        }
        .into_any_element()
    }

    /// The top result large, the next few as rows beside it.
    fn hero(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.views[index];
        let Some(first) = view.shelf.items.first() else {
            return div().into_any_element();
        };
        let rest = &view.shelf.items[1..];
        h_flex()
            .w_full()
            .items_start()
            .gap_6()
            .child(self.hero_card(&view.key, first, cx))
            .when(!rest.is_empty(), |row| {
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(self.item_rows(index, rest, cx)),
                )
            })
            .into_any_element()
    }

    fn hero_card(
        &self,
        shelf: &SharedString,
        item: &CatalogItem,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = SharedString::from(format!("{shelf}-hero-{}", item.id));
        let round = item.kind == CatalogItemKind::Artist;
        let radio = radio_on(item, cx);
        let song = item.track.as_ref().map(|track| track.key.clone());
        let route = open_route(item);
        let playing = song
            .as_deref()
            .is_some_and(|song| self.play_state(song, cx) == PlayState::Playing);
        let click = cx.listener(move |this, _, _, cx| match (&song, &route) {
            (Some(song), _) => this.start_song(song.clone(), radio, cx),
            (None, Some(route)) => Navigator::navigate(route.clone(), cx),
            (None, None) => {}
        });
        HeroCard {
            key,
            title: item.title.clone().into(),
            subtitle: item.subtitle.clone().map(Into::into),
            artwork: item.artwork.clone(),
            round,
            playing,
        }
        .render(click)
    }

    /// Items as list rows: a song plays, anything else opens.
    fn item_rows(&self, index: usize, items: &[CatalogItem], cx: &mut Context<Self>) -> AnyElement {
        let shelf = self.views[index].key.clone();
        v_flex()
            .w_full()
            .children(items.iter().map(|item| self.item_row(&shelf, item, cx)))
            .into_any_element()
    }

    fn item_row(
        &self,
        shelf: &SharedString,
        item: &CatalogItem,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = TileRow::new(element_id(shelf, &item.id), item.title.clone())
            .subtitle(item.subtitle.clone().unwrap_or_default())
            .artwork(item.artwork.as_ref())
            .round(item.kind == CatalogItemKind::Artist)
            .placeholder(placeholder(item.kind));
        if let Some(track) = item.track.as_ref() {
            let key = track.key.clone();
            let radio = radio_on(item, cx);
            return row
                .pictured(Pictured::Track {
                    artist: track.artist.clone().into(),
                    title: track.title.clone().into(),
                })
                .playing(self.play_state(&key, cx) == PlayState::Playing)
                .menu(Some(Target::track(key.clone(), track.title.clone())))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.start_song(key.clone(), radio, cx)),
                )
                .into_any_element();
        }
        let row = row.menu(menu_of(item));
        match open_route(item) {
            Some(route) => row.route(route).into_any_element(),
            None => row.into_any_element(),
        }
    }

    fn tile(&self, shelf: &SharedString, item: &CatalogItem, cx: &mut Context<Self>) -> AnyElement {
        let tile = Tile::new(element_id(shelf, &item.id), item.title.clone())
            .subtitle(item.subtitle.clone().unwrap_or_default())
            .artwork(item.artwork.as_ref())
            .size(TILE);
        match item.kind {
            CatalogItemKind::Track | CatalogItemKind::Video | CatalogItemKind::Episode => {
                self.song_tile(tile, item, cx)
            }
            CatalogItemKind::Album | CatalogItemKind::Playlist => {
                self.playable_tile(tile, item, cx)
            }
            // An artist tile's id is the key the daemon minted for it.
            CatalogItemKind::Artist => tile
                .placeholder(icons::MIC)
                .round(true)
                .tint(true)
                .route(Route::DiscoverArtist(item.id.clone()))
                .pictured(Pictured::Artist(item.title.clone().into()))
                .menu(menu_of(item))
                .into_any_element(),
            // A mood with no picture is a coloured button, as the source draws it.
            CatalogItemKind::Mood | CatalogItemKind::Page if item.artwork.is_none() => {
                page_button(shelf, item, TILE, cx)
            }
            CatalogItemKind::Mood | CatalogItemKind::Page => tile
                .placeholder(icons::COMPASS)
                .route(Route::CatalogPage {
                    id: item.id.clone().into(),
                })
                .into_any_element(),
            CatalogItemKind::Podcast | CatalogItemKind::Unknown => tile.into_any_element(),
        }
    }

    fn playable_tile(&self, tile: Tile, item: &CatalogItem, cx: &mut Context<Self>) -> AnyElement {
        let id = item.id.clone();
        let kind = item.kind;
        let album = kind == CatalogItemKind::Album;
        let route = match album {
            true => Route::Album(id.clone().into()),
            false => Route::DiscoverPlaylist {
                id: id.clone().into(),
                title: item.title.clone().into(),
            },
        };
        // A catalog album names its artist only inside a display subtitle, so the file takes the title.
        let of = Pictured::Album {
            artist: SharedString::default(),
            title: item.title.clone().into(),
        };
        tile.pictured(of)
            .placeholder(if album { icons::DISC } else { icons::LIST })
            .route(route)
            .menu(menu_of(item))
            .play(
                self.play_state(&id, cx),
                cx.listener(move |this, _, _, cx| this.toggle_or_play(kind, id.clone(), cx)),
            )
            .into_any_element()
    }

    fn song_tile(&self, tile: Tile, item: &CatalogItem, cx: &mut Context<Self>) -> AnyElement {
        let Some(track) = item.track.as_ref() else {
            return tile.into_any_element();
        };
        let key = track.key.clone();
        let radio = radio_on(item, cx);
        let press = key.clone();
        tile.pictured(Pictured::Track {
            artist: track.artist.clone().into(),
            title: track.title.clone().into(),
        })
        .menu(Some(Target::track(track.key.clone(), track.title.clone())))
        .on_click(cx.listener(move |this, _, _, cx| this.start_song(key.clone(), radio, cx)))
        .play(
            self.play_state(&track.key, cx),
            cx.listener(move |this, _, _, cx| this.start_song(press.clone(), radio, cx)),
        )
        .into_any_element()
    }

    fn play_state(&self, id: &str, cx: &App) -> PlayState {
        if self.pending.as_deref() == Some(id) {
            return PlayState::Loading;
        }
        match self.now_playing.as_deref() == Some(id)
            && PlayerModel::global(cx).read(cx).is_playing()
        {
            true => PlayState::Playing,
            false => PlayState::Idle,
        }
    }
}

impl Render for Shelves {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sections = Vec::with_capacity(self.views.len());
        for index in 0..self.views.len() {
            if self.views[index].row.is_none() {
                sections.push(self.section(index, cx));
            }
        }
        v_flex().w_full().gap_12().children(sections)
    }
}

/// The songs of a shelf, for a track list.
fn songs(shelf: &CatalogShelf) -> Vec<api::TrackInfo> {
    shelf
        .items
        .iter()
        .filter_map(|item| item.track.clone())
        .collect()
}

/// An episode never seeds a radio, and neither does a source that runs none.
fn radio_on(item: &CatalogItem, cx: &App) -> bool {
    item.kind != CatalogItemKind::Episode
        && SourcesModel::global(cx).read(cx).capabilities().track_radio
}

/// Where a tile that is not a song goes when clicked.
pub(crate) fn open_route(item: &CatalogItem) -> Option<Route> {
    match item.kind {
        CatalogItemKind::Album => Some(Route::Album(item.id.clone().into())),
        CatalogItemKind::Playlist => Some(Route::DiscoverPlaylist {
            id: item.id.clone().into(),
            title: item.title.clone().into(),
        }),
        CatalogItemKind::Artist => Some(Route::DiscoverArtist(item.id.clone())),
        CatalogItemKind::Mood | CatalogItemKind::Page => Some(Route::CatalogPage {
            id: item.id.clone().into(),
        }),
        CatalogItemKind::Track
        | CatalogItemKind::Video
        | CatalogItemKind::Episode
        | CatalogItemKind::Podcast
        | CatalogItemKind::Unknown => None,
    }
}

/// A catalog playlist is nobody's saved list, so only albums and artists carry a menu.
fn menu_of(item: &CatalogItem) -> Option<Target> {
    match item.kind {
        CatalogItemKind::Album => Some(Target::Album {
            id: item.id.clone().into(),
            title: item.title.clone().into(),
        }),
        CatalogItemKind::Artist => Some(Target::Artist {
            key: item.id.clone(),
            name: item.title.clone().into(),
        }),
        _ => None,
    }
}

fn placeholder(kind: CatalogItemKind) -> &'static str {
    match kind {
        CatalogItemKind::Album => icons::DISC,
        CatalogItemKind::Playlist => icons::LIST,
        CatalogItemKind::Artist => icons::MIC,
        CatalogItemKind::Mood | CatalogItemKind::Page => icons::COMPASS,
        _ => icons::MUSIC,
    }
}

/// Where a shelf's "Show all" goes, when the source gave it one.
pub(crate) fn more_route(shelf: &CatalogShelf) -> Option<Route> {
    let more = shelf.more_ref.clone()?;
    match shelf.more_kind {
        CatalogItemKind::Album => Some(Route::Album(more.into())),
        CatalogItemKind::Artist => Some(Route::DiscoverArtist(more)),
        CatalogItemKind::Mood | CatalogItemKind::Page => {
            Some(Route::CatalogPage { id: more.into() })
        }
        CatalogItemKind::Podcast => None,
        // A song list's "show all" opens as a playlist of the same songs.
        CatalogItemKind::Playlist
        | CatalogItemKind::Track
        | CatalogItemKind::Video
        | CatalogItemKind::Episode
        | CatalogItemKind::Unknown => Some(Route::DiscoverPlaylist {
            id: more.into(),
            title: shelf.title.clone().into(),
        }),
    }
}

fn show_all(shelf: &SharedString, route: Route) -> components::Link {
    heading_link(
        SharedString::from(format!("{shelf}-show-all")),
        t("discover_show_all"),
    )
    .on_click(move |_, _, cx| Navigator::navigate(route.clone(), cx))
}

/// `#rrggbb` as the source wrote it.
fn accent(item: &CatalogItem) -> Option<Hsla> {
    let hex = item.accent.as_deref()?;
    Rgba::try_from(hex).ok().map(Hsla::from)
}

/// A mood or page with no picture: its name on a raised button, edged in the source's colour.
fn page_button(
    shelf: &SharedString,
    item: &CatalogItem,
    width: Pixels,
    cx: &mut App,
) -> AnyElement {
    let key = element_id(shelf, &item.id);
    let route = Route::CatalogPage {
        id: item.id.clone().into(),
    };
    PageButton {
        key,
        title: item.title.clone().into(),
        edge: accent(item),
        width,
    }
    .render(route, cx)
}

struct PageButton {
    key: SharedString,
    title: SharedString,
    edge: Option<Hsla>,
    width: Pixels,
}

const HOVER: u64 = 1;
const WASH: u64 = 2;

impl PageButton {
    fn render(self, route: Route, cx: &mut App) -> AnyElement {
        let key = self.key;
        let edge = self.edge.unwrap_or(style::white(style::BORDER, cx));
        div()
            .id(key.clone())
            .child(PageButtonBody {
                key,
                title: self.title,
                edge,
                width: self.width,
            })
            .on_click(move |_, _, cx| Navigator::navigate(route.clone(), cx))
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct PageButtonBody {
    key: SharedString,
    title: SharedString,
    edge: Hsla,
    width: Pixels,
}

impl gpui_kit::RenderOnce for PageButtonBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (hovered, pointer) = motion::hovered(motion::channel(&self.key, HOVER), window, cx);
        let wash = motion::color(
            motion::channel(&self.key, WASH),
            style::white(if hovered { style::HOVER } else { style::RAISED }, cx),
            window,
            cx,
        );
        div()
            .id(motion::channel(&self.key, 0))
            .w(self.width)
            .h_12()
            .flex_shrink_0()
            .flex()
            .items_center()
            .px_4()
            .rounded(style::radius_lg())
            .border_l_4()
            .border_color(self.edge)
            .bg(wash)
            .cursor_pointer()
            .on_hover(motion::track(pointer))
            .child(
                div()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title),
            )
    }
}

/// The top result: a large cover over its name, the whole card one press.
struct HeroCard {
    key: SharedString,
    title: SharedString,
    subtitle: Option<SharedString>,
    artwork: Option<api::ArtworkRef>,
    round: bool,
    playing: bool,
}

impl HeroCard {
    fn render(
        self,
        click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        div()
            .id(self.key.clone())
            .flex_shrink_0()
            .on_click(click)
            .child(HeroBody(self))
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct HeroBody(HeroCard);

impl gpui_kit::RenderOnce for HeroBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let card = self.0;
        let (hovered, pointer) = motion::hovered(motion::channel(&card.key, HOVER), window, cx);
        let wash = motion::color(
            motion::channel(&card.key, WASH),
            style::white(if hovered { style::HOVER } else { style::RAISED }, cx),
            window,
            cx,
        );
        let radius = match card.round {
            true => HERO_COVER / 2.,
            false => style::to_px(style::radius_lg(), window),
        };
        v_flex()
            .id(motion::channel(&card.key, 0))
            .w(HERO_CARD)
            .gap_4()
            .p_5()
            .rounded(style::radius_xl())
            .bg(wash)
            .cursor_pointer()
            .on_hover(motion::track(pointer))
            .child(
                Cover::new(card.artwork.as_ref())
                    .size(HERO_COVER)
                    .rounded(radius)
                    .zoom(motion::channel(&card.key, 3), hovered),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .line_clamp(2)
                            .text_color(match card.playing {
                                true => gpui_kit::component::ActiveTheme::theme(cx).primary,
                                false => gpui_kit::component::ActiveTheme::theme(cx).foreground,
                            })
                            .child(card.title),
                    )
                    .when_some(card.subtitle, |lines, subtitle| {
                        lines.child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(style::white(style::TEXT_MUTED, cx))
                                .child(subtitle),
                        )
                    }),
            )
    }
}

/// A catalog id is unique only inside its shelf, and shelves only ever append.
fn element_id(shelf: &SharedString, id: &str) -> SharedString {
    SharedString::from(format!("{shelf}-{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: CatalogItemKind, id: &str) -> CatalogItem {
        CatalogItem {
            kind,
            id: id.into(),
            ..CatalogItem::default()
        }
    }

    #[test]
    fn a_mood_or_page_tile_opens_its_page() {
        for kind in [CatalogItemKind::Mood, CatalogItemKind::Page] {
            assert_eq!(
                open_route(&item(kind, "FEmusic_moods_and_genres_category?params=x")),
                Some(Route::CatalogPage {
                    id: "FEmusic_moods_and_genres_category?params=x".into()
                })
            );
        }
    }

    #[test]
    fn a_page_shelf_shows_all_on_its_page() {
        let shelf = CatalogShelf {
            more_ref: Some("FEmusic_charts".into()),
            more_kind: CatalogItemKind::Page,
            ..CatalogShelf::default()
        };
        assert_eq!(
            more_route(&shelf),
            Some(Route::CatalogPage {
                id: "FEmusic_charts".into()
            })
        );
    }

    #[test]
    fn a_shelf_without_a_page_of_its_own_has_no_show_all() {
        assert_eq!(more_route(&CatalogShelf::default()), None);
    }
}
