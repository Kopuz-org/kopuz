//! The command palette: one box that reaches every verb, every row and every setting.

mod fuzzy;
mod mode;
mod prefs;
mod settings_index;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use api::{AlbumPage, ApiError, ArtistPage, KopuzApi, SearchRequest, SearchResults, Table};
use components::format::{t, t_with};
use components::{Badge, CommandRow, EmptyState, KeyHint, SearchField, icons, motion, style};
use gpui_kit::component::input::{Escape, IndentInline, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, UniformListScrollHandle, Window,
    div, px, uniform_list,
};
use mode::{Mode, Part};
use store::{Chrome, LayoutModel, Navigator, PlayerModel, PlaylistsModel, Query, Route, Target};

pub use prefs::register as register_settings;

/// How long typing settles before the daemon is asked for tracks.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// `max-w-2xl`, wide enough for a title, a subtitle and a keystroke.
const PANEL_WIDTH: Pixels = px(620.);
/// Every row is the same height, because the list is virtualized.
const ROW: Pixels = px(40.);
/// How many rows the list shows before it scrolls.
const SHOWN: usize = 11;
/// Where the panel's top edge sits, as a share of the window: fixed, so the input never moves.
const TOP: f32 = 0.15;
/// What the panel leaves under itself at its tallest, as a share of the window.
const FLOOR: f32 = 0.08;
/// The input row and the hint bar, which the list fits between.
const FRAME: Pixels = px(88.);
/// `bg-black/55` behind the panel; GPUI has no backdrop blur, so the scrim carries it alone.
const SCRIM: f32 = 0.55;

/// How many rows one block may contribute.
const FEW: usize = 5;
const MANY: usize = 8;

/// Somewhere to go, with the command that already has a keystroke for it.
struct Place {
    name: &'static str,
    icon: &'static str,
    command: Option<&'static str>,
}

const fn place(name: &'static str, icon: &'static str, command: Option<&'static str>) -> Place {
    Place {
        name,
        icon,
        command,
    }
}

static PLACES: &[Place] = &[
    place("home", icons::HOME, Some("nav.home")),
    place("library", icons::LIBRARY, Some("nav.library")),
    place("albums", icons::DISC, None),
    place("artists", icons::MIC, None),
    place("playlists", icons::LIST, None),
    place("favorites", icons::HEART, None),
    place("search", icons::SEARCH, None),
    place("discover", icons::COMPASS, Some("nav.discover")),
    place("radio", icons::RADIO, Some("nav.radio")),
    place("downloader", icons::DOWNLOAD, None),
    place("activity", icons::ACTIVITY, None),
    place("settings", icons::SETTINGS, Some("app.settings")),
];

/// What activating a row does.
#[derive(Clone)]
enum Act {
    Run {
        id: &'static str,
        target: Target,
    },
    /// Play the results themselves, from this row: a hit never costs a library read.
    Play(usize),
    Go(Route),
    Setting {
        slug: &'static str,
        title: SharedString,
    },
    /// Put a remembered query back in the box.
    Recall(SharedString),
    /// Hand the query to Discover, which shows every match rather than the best few.
    Browse(SharedString),
}

/// One drawn row, already ranked.
struct Item {
    title: SharedString,
    subtitle: Option<SharedString>,
    icon: &'static str,
    hint: Option<SharedString>,
    act: Act,
    /// What `tab` drills into, when this row stands for something commands apply to.
    target: Option<Target>,
    /// This answers the previous query; the newer one is still in flight.
    faded: bool,
}

/// The flat list the virtualizer walks: headings and rows, in drawing order.
enum Line {
    Head(SharedString),
    Row(usize),
}

type Answer = Pin<Box<dyn Future<Output = Result<SearchResults, ApiError>> + Send>>;

/// The read behind one query; a blank one answers with nothing rather than asking the daemon.
fn search_fetch(query: String) -> impl Fn(Arc<dyn KopuzApi>) -> Answer + 'static {
    move |api| {
        let query = query.clone();
        Box::pin(async move {
            if query.is_empty() {
                return Ok(SearchResults::default());
            }
            api.search(SearchRequest::new(query)).await
        })
    }
}

/// Rendered only while [`Chrome::is_quick_search_open`].
pub struct Palette {
    input: Entity<InputState>,
    tracks: Option<Entity<Query<SearchResults>>>,
    albums: Option<Entity<Query<AlbumPage>>>,
    artists: Option<Entity<Query<ArtistPage>>>,
    typed: String,
    mode: Mode,
    query: String,
    /// What the daemon was last asked for, so the same query is not asked twice.
    asked: String,
    /// `tab` narrowed the palette to one thing's own verbs.
    drill: Option<Target>,
    items: Vec<Item>,
    lines: Vec<Line>,
    /// The track keys behind the track rows, in the order they are drawn.
    keys: Vec<String>,
    selected: usize,
    open: bool,
    // Its own fade, so it can fold away after the shell stops asking for it.
    fade: motion::Fade,
    scroll: UniformListScrollHandle,
    // Closing hands focus back, or the shell's keys would have nowhere to dispatch from.
    was_focused: Option<FocusHandle>,
    _debounce: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Palette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t("palette_placeholder")));
        let chrome = Chrome::global(cx);
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => this.typed(cx),
                        InputEvent::PressEnter { .. } => this.activate(this.selected, window, cx),
                        _ => {}
                    }
                }),
                cx.observe_in(&chrome, window, |this, chrome, window, cx| {
                    let open = chrome.read(cx).is_quick_search_open();
                    if open == this.open {
                        return;
                    }
                    this.open = open;
                    if open {
                        this.reopen(window, cx);
                    } else if let Some(handle) = this.was_focused.take() {
                        window.focus(&handle, cx);
                    }
                }),
                cx.observe(&PlaylistsModel::global(cx), |this, _, cx| this.rebuild(cx)),
            ];
        let open = chrome.read(cx).is_quick_search_open();
        Self {
            input,
            tracks: None,
            albums: None,
            artists: None,
            typed: String::new(),
            mode: Mode::default(),
            query: String::new(),
            asked: String::new(),
            drill: None,
            items: Vec::new(),
            lines: Vec::new(),
            keys: Vec::new(),
            selected: 0,
            open,
            fade: motion::Fade::hidden(),
            scroll: UniformListScrollHandle::new(),
            was_focused: None,
            _debounce: None,
            _subscriptions: subscriptions,
        }
    }

    /// Every opening starts from an empty box, with the caret already in it.
    fn reopen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.typed.clear();
        self.query.clear();
        self.asked.clear();
        self.mode = Mode::default();
        self.drill = None;
        self.selected = 0;
        self._debounce = None;
        self.was_focused = window.focused(cx);
        self.load(cx);
        self.input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        if let Some(tracks) = self.tracks.clone() {
            tracks.update(cx, |query, cx| {
                query.set_fetch(search_fetch(String::new()), cx)
            });
        }
        self.rebuild(cx);
    }

    /// The library reads the client-side matching works over, asked for the first time it is needed.
    fn load(&mut self, cx: &mut Context<Self>) {
        if self.tracks.is_some() {
            return;
        }
        let tracks = cx.new(|cx| Query::new(&[Table::Tracks], search_fetch(String::new()), cx));
        let albums = cx.new(|cx| {
            Query::new(
                &[Table::Albums],
                |api| async move { api.albums(Default::default(), store::all()).await },
                cx,
            )
        });
        let artists = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Artists],
                |api| async move { api.artists(Default::default(), store::all()).await },
                cx,
            )
        });
        self._subscriptions
            .push(cx.observe(&tracks, |this, _, cx| this.rebuild(cx)));
        self._subscriptions
            .push(cx.observe(&albums, |this, _, cx| this.rebuild(cx)));
        self._subscriptions
            .push(cx.observe(&artists, |this, _, cx| this.rebuild(cx)));
        self.tracks = Some(tracks);
        self.albums = Some(albums);
        self.artists = Some(artists);
    }

    /// Open the palette with a query already in it, for a preview or a deep link.
    pub fn seed(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let query = query.to_string();
        self.input
            .update(cx, |state, cx| state.set_value(query, window, cx));
        self.typed(cx);
    }

    /// Whether it is on screen, open or still fading out.
    pub fn is_visible(&self) -> bool {
        self.open || self.fade.is_visible()
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        Chrome::global(cx).update(cx, |chrome, cx| chrome.set_quick_search(false, cx));
    }

    fn typed(&mut self, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value().to_string();
        if typed == self.typed {
            return;
        }
        self.typed = typed;
        let (mode, query) = mode::parse(&self.typed);
        self.mode = mode;
        self.query = query.to_string();
        self.selected = 0;
        self.schedule(cx);
        self.rebuild(cx);
    }

    /// Typing restarts the timer; the task it replaces is dropped, which cancels it.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        let wanted = match self.drill.is_none() && self.mode == Mode::Everything {
            true => self.query.clone(),
            false => String::new(),
        };
        if wanted == self.asked {
            return;
        }
        self.asked = wanted.clone();
        self._debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |this, cx| {
                let Some(tracks) = this.tracks.clone() else {
                    return;
                };
                tracks.update(cx, |query, cx| {
                    query.set_fetch(search_fetch(wanted.clone()), cx)
                });
            })
            .ok();
        }));
    }

    fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.selected = mode::step(self.selected, self.items.len(), forward);
        self.reveal();
        cx.notify();
    }

    /// The highlight is never allowed off-screen.
    fn reveal(&self) {
        let chosen = self.selected;
        let at = self
            .lines
            .iter()
            .position(|line| matches!(line, Line::Row(index) if *index == chosen));
        if let Some(at) = at {
            self.scroll.scroll_to_item(at, ScrollStrategy::Nearest);
        }
    }

    /// `tab`: narrow to one thing's own verbs, or step back out of them.
    fn drill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self
            .items
            .get(self.selected)
            .and_then(|item| item.target.clone())
        else {
            return;
        };
        self.drill = Some(target);
        self.selected = 0;
        self.input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.typed.clear();
        self.query.clear();
        self.mode = Mode::default();
        self.schedule(cx);
        self.rebuild(cx);
    }

    /// Escape leaves a drill-down before it leaves the palette.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drill.take().is_none() {
            self.close(cx);
            return;
        }
        self.selected = 0;
        self.input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.typed.clear();
        self.query.clear();
        self.schedule(cx);
        self.rebuild(cx);
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(act) = self.items.get(index).map(|item| item.act.clone()) else {
            return;
        };
        if let Act::Recall(query) = act {
            self.input
                .update(cx, |state, cx| state.set_value(query, window, cx));
            self.typed(cx);
            return;
        }
        self.remember_search(cx);
        match act {
            Act::Run { id, target } => {
                self.remember_command(id, cx);
                self.close(cx);
                if let Some(command) = commands::get(id) {
                    command.perform(target, window, cx);
                }
            }
            Act::Play(at) => {
                let keys = self.keys.clone();
                self.close(cx);
                if let Some(key) = keys.get(at).cloned() {
                    store::actions::play_in_library(key, keys, at as u32, cx);
                }
            }
            Act::Go(route) => {
                self.close(cx);
                Navigator::global(cx).update(cx, |navigator, cx| navigator.go(route, cx));
            }
            Act::Setting { slug, title } => {
                self.close(cx);
                components::settings::set_highlight(Some(title), cx);
                let route = Route::Settings {
                    section: Some(SharedString::from(slug)),
                };
                Navigator::global(cx).update(cx, |navigator, cx| navigator.go(route, cx));
            }
            Act::Browse(query) => {
                self.close(cx);
                Chrome::global(cx).update(cx, |chrome, cx| chrome.set_search(query, cx));
                Navigator::global(cx).update(cx, |navigator, cx| navigator.go(Route::Discover, cx));
            }
            Act::Recall(_) => {}
        }
    }

    fn remember_command(&self, id: &str, cx: &mut Context<Self>) {
        let held = store::layout(cx).palette.clone();
        if !held.history {
            return;
        }
        let id = id.to_string();
        LayoutModel::edit(cx, |layout| {
            store::layout::Palette::remember(&mut layout.palette.recent_commands, id, held.recents)
        });
    }

    fn remember_search(&self, cx: &mut Context<Self>) {
        let held = store::layout(cx).palette.clone();
        if !held.history || self.query.is_empty() || self.drill.is_some() {
            return;
        }
        let query = self.typed.trim().to_string();
        LayoutModel::edit(cx, |layout| {
            store::layout::Palette::remember(
                &mut layout.palette.recent_searches,
                query,
                held.recents,
            )
        });
    }
}

/// Take the best `limit` of what matched, keeping equal scores in the order they came.
fn ranked(mut scored: Vec<(i32, Item)>, limit: usize) -> Vec<Item> {
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, item)| item)
        .collect()
}

fn hint(id: &str, cx: &gpui_kit::App) -> Option<SharedString> {
    commands::keystroke_for(id, cx).map(SharedString::from)
}

impl Palette {
    /// Rank everything once, here, so `render` only draws what it is handed.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let mut items = Vec::new();
        let mut lines = Vec::new();
        self.keys.clear();
        if let Some(target) = self.drill.clone() {
            let found = self.command_items(&target, cx);
            section(&mut lines, &mut items, target.label(), found);
        } else {
            let playing = self.now_playing(cx);
            let leads = store::layout(cx).palette.current_track_first;
            let parts = mode::order(
                self.mode,
                !self.query.is_empty(),
                playing.is_some() && leads,
            );
            for part in parts {
                let found = self.part_items(part, playing.as_ref(), cx);
                let title = match (part, playing.as_ref()) {
                    // The block names the track once, so its rows do not each repeat it.
                    (Part::CurrentTrack, Some(target)) => t_with(
                        "palette_current_track",
                        &[("title", target.label().to_string().into())],
                    ),
                    _ => t(part.label()),
                };
                section(&mut lines, &mut items, title, found);
            }
            // Last, because enter belongs to the best hit: the way out of the best few and into all of them.
            section(&mut lines, &mut items, t("discover"), self.browse_item());
        }
        self.items = items;
        self.lines = lines;
        if self.selected >= self.items.len() {
            self.selected = 0;
        }
        self.reveal();
        cx.notify();
    }

    fn now_playing(&self, cx: &gpui_kit::App) -> Option<Target> {
        PlayerModel::global(cx)
            .read(cx)
            .track()
            .map(|now| Target::track(now.key.clone(), now.title.clone()))
    }

    fn part_items(
        &mut self,
        part: Part,
        playing: Option<&Target>,
        cx: &mut Context<Self>,
    ) -> Vec<Item> {
        match part {
            Part::CurrentTrack => match playing {
                Some(target) => self.command_items(target, cx),
                None => Vec::new(),
            },
            Part::Commands => self.command_items(&Target::None, cx),
            Part::RecentCommands => self.recent_command_items(cx),
            Part::RecentSearches => self.recent_search_items(cx),
            Part::Tracks => self.track_items(cx),
            Part::Albums => self.album_items(cx),
            Part::Artists => self.artist_items(cx),
            Part::Playlists => self.playlist_items(cx),
            Part::Navigate => self.place_items(cx),
            Part::Settings => self.setting_items(cx),
        }
    }

    fn command_items(&self, target: &Target, cx: &gpui_kit::App) -> Vec<Item> {
        let scored: Vec<(i32, Item)> = commands::for_palette(target, cx)
            .into_iter()
            .filter_map(|command| {
                let title = command.label(target, cx);
                let score = fuzzy::score(&title, &self.query)?;
                Some((score, self.command_item(command, target, title, cx)))
            })
            .collect();
        ranked(scored, MANY)
    }

    fn command_item(
        &self,
        command: &'static commands::Command,
        target: &Target,
        title: SharedString,
        cx: &gpui_kit::App,
    ) -> Item {
        Item {
            title,
            subtitle: None,
            icon: command.icon.unwrap_or(icons::MORE),
            hint: hint(command.id, cx),
            act: Act::Run {
                id: command.id,
                target: target.clone(),
            },
            target: None,
            faded: false,
        }
    }

    /// The row that hands the query on to Discover; a blank box has nothing to hand over.
    fn browse_item(&self) -> Vec<Item> {
        if self.query.is_empty() {
            return Vec::new();
        }
        vec![Item {
            title: t_with(
                "palette_all_results",
                &[("query", self.query.clone().into())],
            ),
            subtitle: None,
            icon: icons::COMPASS,
            hint: None,
            act: Act::Browse(SharedString::from(self.query.clone())),
            target: None,
            faded: false,
        }]
    }

    fn recent_command_items(&self, cx: &gpui_kit::App) -> Vec<Item> {
        let held = store::layout(cx).palette.clone();
        if !held.history {
            return Vec::new();
        }
        held.recent_commands
            .iter()
            .filter_map(|id| {
                let command = commands::get(id)?;
                command.is_available(&Target::None, cx).then(|| {
                    self.command_item(command, &Target::None, command.label(&Target::None, cx), cx)
                })
            })
            .take(held.recents)
            .collect()
    }

    fn recent_search_items(&self, cx: &gpui_kit::App) -> Vec<Item> {
        let held = store::layout(cx).palette.clone();
        if !held.history {
            return Vec::new();
        }
        held.recent_searches
            .iter()
            .take(held.recents)
            .map(|query| Item {
                title: SharedString::from(query.clone()),
                subtitle: None,
                icon: icons::SEARCH,
                hint: None,
                act: Act::Recall(SharedString::from(query.clone())),
                target: None,
                faded: false,
            })
            .collect()
    }

    fn track_items(&mut self, cx: &gpui_kit::App) -> Vec<Item> {
        let Some(tracks) = self.tracks.clone() else {
            return Vec::new();
        };
        let held = tracks.read(cx);
        let faded = held.is_stale();
        let Some(found) = held.value() else {
            return Vec::new();
        };
        let rows: Vec<Item> = found
            .tracks
            .iter()
            .take(MANY)
            .enumerate()
            .map(|(at, track)| Item {
                title: SharedString::from(track.title.clone()),
                subtitle: Some(SharedString::from(track.artist.clone())),
                icon: icons::MUSIC,
                hint: None,
                act: Act::Play(at),
                target: Some(Target::track(track.key.clone(), track.title.clone())),
                faded,
            })
            .collect();
        self.keys = found
            .tracks
            .iter()
            .take(MANY)
            .map(|track| track.key.clone())
            .collect();
        rows
    }

    fn album_items(&self, cx: &gpui_kit::App) -> Vec<Item> {
        let mut scored = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let library = match self.albums.as_ref() {
            Some(albums) => albums.read(cx).value(),
            None => None,
        };
        let searched = match self.tracks.as_ref() {
            Some(tracks) => tracks.read(cx).value(),
            None => None,
        };
        let held = library.map(|page| page.albums.as_slice()).unwrap_or(&[]);
        let found = searched.map(|page| page.albums.as_slice()).unwrap_or(&[]);
        for album in held.iter().chain(found) {
            if seen.iter().any(|id| id == &album.id) {
                continue;
            }
            let Some(score) = fuzzy::score(&album.title, &self.query) else {
                continue;
            };
            seen.push(album.id.clone());
            scored.push((
                score,
                Item {
                    title: SharedString::from(album.title.clone()),
                    subtitle: Some(SharedString::from(album.artist.clone())),
                    icon: icons::DISC,
                    hint: None,
                    act: Act::Go(Route::Album(SharedString::from(album.id.clone()))),
                    target: Some(Target::Album {
                        id: album.id.clone().into(),
                        title: album.title.clone().into(),
                    }),
                    faded: false,
                },
            ));
        }
        ranked(scored, FEW)
    }

    fn artist_items(&self, cx: &gpui_kit::App) -> Vec<Item> {
        let page = match self.artists.as_ref() {
            Some(artists) => artists.read(cx).value(),
            None => None,
        };
        let Some(page) = page else {
            return Vec::new();
        };
        let scored: Vec<(i32, Item)> = page
            .artists
            .iter()
            .filter_map(|artist| {
                let score = fuzzy::score(&artist.name, &self.query)?;
                Some((
                    score,
                    Item {
                        title: SharedString::from(artist.name.clone()),
                        subtitle: Some(t_with(
                            "track_count",
                            &[("count", artist.track_count.into())],
                        )),
                        icon: icons::MIC,
                        hint: None,
                        act: Act::Go(Route::Artist(artist.key.clone())),
                        target: Some(Target::Artist {
                            key: artist.key.clone(),
                            name: artist.name.clone().into(),
                        }),
                        faded: false,
                    },
                ))
            })
            .collect();
        ranked(scored, FEW)
    }

    fn playlist_items(&self, cx: &gpui_kit::App) -> Vec<Item> {
        let playlists = PlaylistsModel::global(cx);
        let scored: Vec<(i32, Item)> = playlists
            .read(cx)
            .playlists()
            .iter()
            .filter_map(|playlist| {
                let score = fuzzy::score(&playlist.name, &self.query)?;
                Some((
                    score,
                    Item {
                        title: SharedString::from(playlist.name.clone()),
                        subtitle: Some(t_with(
                            "track_count",
                            &[("count", playlist.track_keys.len().into())],
                        )),
                        icon: icons::LIST,
                        hint: None,
                        act: Act::Go(Route::Playlist(SharedString::from(playlist.id.clone()))),
                        target: Some(Target::Playlist {
                            id: playlist.id.clone().into(),
                            name: playlist.name.clone().into(),
                        }),
                        faded: false,
                    },
                ))
            })
            .collect();
        ranked(scored, FEW)
    }

    fn place_items(&self, cx: &gpui_kit::App) -> Vec<Item> {
        let scored: Vec<(i32, Item)> = PLACES
            .iter()
            .filter_map(|place| {
                let title = t(place.name);
                let score = fuzzy::score(&title, &self.query)?;
                let route = Route::from_name(place.name)?;
                Some((
                    score,
                    Item {
                        title,
                        subtitle: None,
                        icon: place.icon,
                        hint: place.command.and_then(|id| hint(id, cx)),
                        act: Act::Go(route),
                        target: None,
                        faded: false,
                    },
                ))
            })
            .collect();
        ranked(scored, MANY)
    }

    fn setting_items(&self, _: &gpui_kit::App) -> Vec<Item> {
        let scored: Vec<(i32, Item)> = settings_index::ROWS
            .iter()
            .chain(settings_index::CATEGORIES)
            .filter_map(|entry| {
                let title = t(entry.key);
                let score = fuzzy::score(&title, &self.query)?;
                Some((
                    score,
                    Item {
                        title: title.clone(),
                        subtitle: Some(t("settings")),
                        icon: icons::SETTINGS,
                        hint: None,
                        act: Act::Setting {
                            slug: entry.slug,
                            title,
                        },
                        target: None,
                        faded: false,
                    },
                ))
            })
            .collect();
        ranked(scored, FEW)
    }
}

/// Add one block under its heading; a block that matched nothing is not drawn at all.
fn section(lines: &mut Vec<Line>, items: &mut Vec<Item>, title: SharedString, found: Vec<Item>) {
    if found.is_empty() {
        return;
    }
    lines.push(Line::Head(title));
    for item in found {
        lines.push(Line::Row(items.len()));
        items.push(item);
    }
}

/// A `uniform_list` needs a definite height: the rows it has, capped by the panel and the window.
fn list_height(count: usize, window: &Window) -> Pixels {
    let wanted = ROW * count as f32;
    let capped = if wanted < ROW * SHOWN as f32 {
        wanted
    } else {
        ROW * SHOWN as f32
    };
    let room = window.viewport_size().height * (1. - TOP - FLOOR) - FRAME;
    let floor = ROW * 4.;
    if room < floor {
        floor
    } else if capped < room {
        capped
    } else {
        room
    }
}

impl Palette {
    fn head(&self, title: SharedString, cx: &gpui_kit::App) -> impl IntoElement {
        h_flex().h(ROW).w_full().px_5().items_end().pb_1p5().child(
            div()
                .text_size(style::text_10())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(style::white(style::TEXT_MUTED, cx))
                .child(title),
        )
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.items.get(index) else {
            return div().id(("palette-row", index)).h(ROW).into_any_element();
        };
        CommandRow::new(("palette-row", index), item.icon, item.title.clone())
            .height(ROW)
            .subtitle(item.subtitle.clone())
            .hint(item.hint.clone())
            .chosen(index == self.selected)
            .faded(item.faded)
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                if *over && this.selected != index {
                    this.selected = index;
                    cx.notify();
                }
            }))
            // Clicking a row focuses it, so the caret has to be handed back to anything still open.
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate(index, window, cx);
                if this.open {
                    this.input.update(cx, |state, cx| state.focus(window, cx));
                }
            }))
            .into_any_element()
    }

    fn footer(&self, cx: &gpui_kit::App) -> impl IntoElement {
        let hints: [(&[&str], &str); 4] = [
            (&["up", "down"], "palette_hint_move"),
            (&["enter"], "palette_hint_open"),
            (&["tab"], "palette_hint_actions"),
            (&["escape"], "palette_hint_close"),
        ];
        h_flex()
            .w_full()
            .h_9()
            .px_4()
            .gap_4()
            .items_center()
            .border_t_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .bg(style::black(style::HOVER, cx))
            .children(
                hints
                    .into_iter()
                    .map(|(keystrokes, label)| KeyHint::new(keystrokes.iter().copied(), t(label))),
            )
    }

    fn empty(&self) -> EmptyState {
        EmptyState::new(match self.query.is_empty() {
            true => t("palette_empty"),
            false => t_with("no_results_found", &[("query", self.query.clone().into())]),
        })
        .icon(icons::SEARCH)
        .compact()
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = profile::render("palette");
        let count = self.lines.len();
        let height = list_height(count, window);
        components::scroll::smooth_list("palette-list", &self.scroll, window, cx);
        let revealed = self.fade.sample(self.open, motion::FAST, window, cx);
        if !self.open && revealed <= 0. {
            // One more frame lets the shell see the fade is over and take the palette down.
            window.request_animation_frame();
        }
        // It rises into place on the way in and only fades on the way out.
        let rise = match self.open {
            true => motion::NUDGE * 1.5 * (1. - revealed),
            false => gpui_kit::px(0.),
        };
        let narrowed = match self.drill.as_ref() {
            Some(target) => Some(target.label()),
            None if self.mode != Mode::Everything => Some(t(self.mode.label())),
            None => None,
        };
        div()
            .id("palette-scrim")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            // A flex child stretches unless told not to, which ran the panel to the window's floor.
            .items_start()
            .px_4()
            .pt(window.viewport_size().height * TOP)
            .opacity(revealed)
            .bg(style::black(SCRIM, cx))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.move_selection(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.move_selection(true, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                this.drill(window, cx);
                cx.stop_propagation();
            }))
            // Escape belongs to the palette while it is open, or the shell would close it twice over.
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.back(window, cx);
                cx.stop_propagation();
            }))
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
            .child(
                v_flex()
                    .id("palette-panel")
                    .relative()
                    .top(rise)
                    .w_full()
                    .max_w(PANEL_WIDTH)
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(style::white(style::BORDER, cx))
                    .rounded(style::radius_xl())
                    .shadow_lg()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .px_4()
                            .gap_2p5()
                            .items_center()
                            .border_b_1()
                            .border_color(style::white(style::BORDER_SOFT, cx))
                            .child(
                                div().flex_1().min_w_0().child(
                                    SearchField::new(&self.input)
                                        .bare(true)
                                        .chip(narrowed.map(Badge::new))
                                        .px_0()
                                        .py_3p5()
                                        .text_base(),
                                ),
                            ),
                    )
                    .when(count == 0, |panel| panel.child(self.empty()))
                    .when(count > 0, |panel| {
                        panel.child(
                            div().w_full().h(height).py_1().child(
                                uniform_list(
                                    "palette-list",
                                    count,
                                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                        range
                                            .map(|at| match &this.lines[at] {
                                                Line::Head(title) => {
                                                    this.head(title.clone(), cx).into_any_element()
                                                }
                                                Line::Row(index) => {
                                                    this.row(*index, cx).into_any_element()
                                                }
                                            })
                                            .collect()
                                    }),
                                )
                                .track_scroll(&self.scroll)
                                .size_full(),
                            ),
                        )
                    })
                    .child(self.footer(cx)),
            )
    }
}
