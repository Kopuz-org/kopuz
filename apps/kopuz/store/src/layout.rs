//! Presentation preferences: what the app looks like, kept in the daemon's per-frontend store.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use api::{ApiEvent, FrontendPref, PrefEntry};
use config::{AlbumSortField, AlbumViewMode, ArtistSortField, SortCriterion, SortDirection};
use futures::channel::oneshot;
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;
use serde::{Deserialize, Serialize};

// Dragging a width or nudging a slider settles before the daemon is written to.
const SAVE_DELAY: Duration = Duration::from_millis(600);

/// The name this app's preferences are kept under in the daemon.
pub const FRONTEND: &str = "gpui";

// Stored once `layout.toml` has been carried over, so the file is never read again.
const IMPORTED: &str = "layout_toml_imported";

/// How big the UI may be scaled, either way of 100%.
pub const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.8..=1.4;

/// Where the navigation lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavPosition {
    #[default]
    Top,
    Left,
}

/// How much of the app follows the current cover's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    On,
    SheetOnly,
    Off,
}

/// How far the playing cover's light reaches across the window behind everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ambient {
    Off,
    #[default]
    Glow,
    Full,
}

/// How much things move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    #[default]
    Full,
    Reduced,
    Off,
}

impl Motion {
    /// Whether a tween should run at all; `Reduced` keeps state changes but drops the travel.
    pub fn animates(self) -> bool {
        self == Motion::Full
    }

    /// The factor a duration is multiplied by, so one switch reaches every tween.
    pub fn scale(self) -> f32 {
        match self {
            Motion::Full => 1.,
            Motion::Reduced => 0.4,
            Motion::Off => 0.,
        }
    }
}

/// Which side of the content the now-playing sheet's panel sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    Right,
    Left,
}

/// What is behind the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    #[default]
    Artwork,
    Palette,
    Solid,
}

/// The sheet's opening tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetTab {
    #[default]
    Queue,
    Lyrics,
    Related,
}

/// How a listing is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    #[default]
    Grid,
    Table,
}

/// Row height in a table, and how much padding a tile gets; what a step measures is the skin's to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

/// How wide a tile is in a band grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl TileSize {
    pub fn width(self) -> f32 {
        match self {
            TileSize::Small => 140.,
            TileSize::Medium => 180.,
            TileSize::Large => 240.,
        }
    }
}

/// One column of a library table, in the order it is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub id: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
}

impl Column {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            visible: true,
            width: None,
        }
    }
}

/// What a listing is ordered by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sort {
    pub field: String,
    #[serde(default)]
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            field: "title".into(),
            descending: false,
        }
    }
}

/// Everything one library facet remembers about how it was left.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Facet {
    pub mode: ViewMode,
    pub density: Density,
    pub tile: TileSize,
    pub sort: Sort,
    pub group_by: Option<String>,
    pub columns: Vec<Column>,
}

/// The navigation bar: where it sits, what it holds, and where the app opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Nav {
    pub position: NavPosition,
    /// Route names in the order they are drawn; unknown names are ignored, missing ones placed by their neighbours.
    pub order: Vec<String>,
    pub hidden: Vec<String>,
    pub start: String,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            position: NavPosition::Top,
            order: DESTINATIONS.iter().map(|name| name.to_string()).collect(),
            hidden: DESTINATIONS
                .iter()
                .filter(|name| !PRIMARY.contains(*name))
                .map(|name| name.to_string())
                .collect(),
            start: "home".into(),
        }
    }
}

/// The destinations the bar shows until someone asks for more; the rest start hidden.
pub const PRIMARY: &[&str] = &["home", "library", "discover", "radio", "favorites"];

/// The destinations a nav bar can hold, in their designed order.
pub const DESTINATIONS: &[&str] = &[
    "home",
    "library",
    "discover",
    "radio",
    "playlists",
    "favorites",
    "downloader",
    "activity",
];

/// A list of names a person can reorder and hide, laid over the names it knows.
pub trait Ordered {
    /// Every name the list can hold, in its designed order.
    const KNOWN: &'static [&'static str];

    fn order(&self) -> &[String];

    fn hidden(&self) -> &[String];

    fn lists(&mut self) -> (&mut Vec<String>, &mut Vec<String>);

    /// The visible names, in order: the saved order with anything new placed by its neighbours, minus hidden.
    fn visible(&self) -> Vec<&str> {
        let mut names: Vec<String> = self
            .order()
            .iter()
            .filter(|name| Self::KNOWN.contains(&name.as_str()))
            .cloned()
            .collect();
        place_missing(&mut names, Self::KNOWN);
        names
            .iter()
            .filter(|name| self.shows(name))
            .filter_map(|name| Self::KNOWN.iter().find(|known| **known == name).copied())
            .collect()
    }

    fn shows(&self, name: &str) -> bool {
        !self.hidden().iter().any(|hidden| hidden == name)
    }

    fn show(&mut self, name: &str, shown: bool) {
        let (_, hidden) = self.lists();
        match shown {
            true => hidden.retain(|held| held != name),
            false => {
                if !hidden.iter().any(|held| held == name) {
                    hidden.push(name.to_string());
                }
            }
        }
    }

    /// Move one name a place along the full order rather than the visible one.
    fn step(&mut self, name: &str, delta: isize) {
        let (order, _) = self.lists();
        // A file written by hand or before a name existed may lack some; fill them in before moving one.
        place_missing(order, Self::KNOWN);
        let Some(from) = order.iter().position(|held| held == name) else {
            return;
        };
        let Some(to) = from.checked_add_signed(delta) else {
            return;
        };
        if to < order.len() {
            order.swap(from, to);
        }
    }
}

/// Put each known name the order lacks just before its next designed neighbour it holds, else just after its previous one.
fn place_missing(order: &mut Vec<String>, known: &[&str]) {
    let at = |order: &[String], name: &str| order.iter().position(|held| held == name);
    for (index, name) in known.iter().enumerate() {
        if at(order, name).is_some() {
            continue;
        }
        let slot = known[index + 1..]
            .iter()
            .find_map(|next| at(order, next))
            .or_else(|| {
                known[..index]
                    .iter()
                    .rev()
                    .find_map(|previous| at(order, previous).map(|found| found + 1))
            })
            .unwrap_or(order.len());
        order.insert(slot, name.to_string());
    }
}

impl Ordered for Nav {
    const KNOWN: &'static [&'static str] = DESTINATIONS;

    fn order(&self) -> &[String] {
        &self.order
    }

    fn hidden(&self) -> &[String] {
        &self.hidden
    }

    fn lists(&mut self) -> (&mut Vec<String>, &mut Vec<String>) {
        (&mut self.order, &mut self.hidden)
    }
}

/// The optional playlists column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panel {
    pub shown: bool,
    pub width: f32,
    pub pinned: Vec<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            shown: false,
            width: 260.,
            pinned: Vec::new(),
        }
    }
}

/// How large the whole player bar is drawn: its height, cover, text and controls together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerSize {
    Small,
    #[default]
    Normal,
    Large,
}

impl PlayerSize {
    /// The factor every measure of the bar is multiplied by.
    pub fn scale(self) -> f32 {
        match self {
            Self::Small => 0.875,
            Self::Normal => 1.,
            Self::Large => 1.25,
        }
    }
}

/// The player bar's size, and which of its optional controls are drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Player {
    pub size: PlayerSize,
    pub shuffle: bool,
    pub repeat: bool,
    pub volume: bool,
    pub lyrics: bool,
    pub queue: bool,
    pub favorite: bool,
    pub cover: TileSize,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            size: PlayerSize::Normal,
            shuffle: true,
            repeat: true,
            volume: true,
            lyrics: true,
            queue: true,
            favorite: true,
            cover: TileSize::Medium,
        }
    }
}

/// The expanded now-playing surface.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sheet {
    pub tab: SheetTab,
    pub side: Side,
    pub backdrop: Backdrop,
    pub cover: HeroSize,
    pub queue: QueueStyle,
}

/// How the queue lists its rows, in the sheet and the docked column alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueStyle {
    /// History folded away above what plays now and what is next; a new song takes the list back to the top.
    #[default]
    Sections,
    /// One flat list, the playing row kept in view, the way the Dioxus rightbar drew it.
    Classic,
}

/// How large the now-playing sheet draws its cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeroSize {
    Small,
    #[default]
    Medium,
    Large,
    ExtraLarge,
}

impl HeroSize {
    /// The cover's side in pixels, before the window's height caps it.
    pub fn side(self) -> f32 {
        match self {
            Self::Small => 320.,
            Self::Medium => 420.,
            Self::Large => 540.,
            Self::ExtraLarge => 680.,
        }
    }

    /// The share of the sheet's height the cover may take, so the title and transport still fit under it.
    pub fn max_height(self) -> f32 {
        match self {
            Self::Small | Self::Medium => 0.5,
            Self::Large => 0.58,
            Self::ExtraLarge => 0.66,
        }
    }
}

/// The library's per-facet state, keyed by facet name.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, transparent)]
pub struct Library(pub BTreeMap<String, Facet>);

impl Library {
    /// How `facet` was left, or how it is drawn by default.
    pub fn facet(&self, name: &str) -> Facet {
        self.0.get(name).cloned().unwrap_or_default()
    }
}

/// The ids the albums grid offers, as the daemon's album order.
pub fn album_sort_field(id: &str) -> Option<AlbumSortField> {
    Some(match id {
        "title" => AlbumSortField::Title,
        "artist" => AlbumSortField::Artist,
        "year" => AlbumSortField::Year,
        "genre" => AlbumSortField::Genre,
        "added" => AlbumSortField::RecentlyAdded,
        "tracks" => AlbumSortField::TrackCount,
        _ => return None,
    })
}

fn album_sort_id(field: AlbumSortField) -> &'static str {
    match field {
        AlbumSortField::Title => "title",
        AlbumSortField::Artist => "artist",
        AlbumSortField::Year => "year",
        AlbumSortField::Genre => "genre",
        AlbumSortField::RecentlyAdded => "added",
        AlbumSortField::TrackCount => "tracks",
    }
}

/// The ids the artists grid offers, as the daemon's artist order.
pub fn artist_sort_field(id: &str) -> Option<ArtistSortField> {
    Some(match id {
        "title" => ArtistSortField::Name,
        "tracks" => ArtistSortField::Tracks,
        "albums" => ArtistSortField::Albums,
        _ => return None,
    })
}

fn artist_sort_id(field: ArtistSortField) -> &'static str {
    match field {
        ArtistSortField::Name => "title",
        ArtistSortField::Tracks => "tracks",
        ArtistSortField::Albums => "albums",
    }
}

fn shared_mode(mode: AlbumViewMode) -> ViewMode {
    match mode {
        AlbumViewMode::Grid => ViewMode::Grid,
        AlbumViewMode::List => ViewMode::Table,
    }
}

fn shared_view(mode: ViewMode) -> AlbumViewMode {
    match mode {
        ViewMode::Grid => AlbumViewMode::Grid,
        ViewMode::Table => AlbumViewMode::List,
    }
}

fn shared_sort<F: Copy>(criteria: &[SortCriterion<F>], id: fn(F) -> &'static str) -> Option<Sort> {
    criteria.first().map(|criterion| Sort {
        field: id(criterion.field).into(),
        descending: criterion.direction == SortDirection::Desc,
    })
}

fn shared_criteria<F>(sort: &Sort, field: fn(&str) -> Option<F>) -> Option<Vec<SortCriterion<F>>> {
    let direction = match sort.descending {
        true => SortDirection::Desc,
        false => SortDirection::Asc,
    };
    field(&sort.field).map(|field| vec![SortCriterion::new(field, direction)])
}

impl Layout {
    /// How a library facet was left; albums and artists keep their order and view in the prefs every frontend shares.
    pub fn library_facet(&self, name: &str) -> Facet {
        let mut facet = self.library.facet(name);
        let (mode, sort) = match name {
            "albums" => (
                self.ui.album_view_mode,
                shared_sort(&self.ui.album_sort, album_sort_id),
            ),
            "artists" => (
                self.ui.artists_view_mode,
                shared_sort(&self.ui.artist_sort, artist_sort_id),
            ),
            _ => return facet,
        };
        facet.mode = shared_mode(mode);
        if let Some(sort) = sort {
            facet.sort = sort;
        }
        facet
    }

    /// Edit a library facet, writing a changed order or view back to the shared prefs it was read from.
    pub fn edit_library_facet(&mut self, name: &str, edit: impl FnOnce(&mut Facet)) {
        let before = self.library_facet(name);
        let mut facet = before.clone();
        edit(&mut facet);
        let (mode_changed, sort_changed) = (facet.mode != before.mode, facet.sort != before.sort);
        match name {
            "albums" => {
                if mode_changed {
                    self.ui.album_view_mode = shared_view(facet.mode);
                }
                if sort_changed && let Some(sort) = shared_criteria(&facet.sort, album_sort_field) {
                    self.ui.album_sort = sort;
                }
            }
            "artists" => {
                if mode_changed {
                    self.ui.artists_view_mode = shared_view(facet.mode);
                }
                if sort_changed && let Some(sort) = shared_criteria(&facet.sort, artist_sort_field)
                {
                    self.ui.artist_sort = sort;
                }
            }
            _ => {}
        }
        self.library.0.insert(name.to_string(), facet);
    }
}

/// Which home shelves are drawn, and in what order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Home {
    pub order: Vec<String>,
    pub hidden: Vec<String>,
}

impl Default for Home {
    fn default() -> Self {
        Self {
            order: SHELVES.iter().map(|name| name.to_string()).collect(),
            hidden: Vec::new(),
        }
    }
}

/// The home shelves, in their designed order.
pub const SHELVES: &[&str] = &[
    "for_you",
    "jump_back_in",
    "recently_added",
    "made_for_you",
    "top_artists",
    "your_playlists",
    "recently_played",
];

impl Ordered for Home {
    const KNOWN: &'static [&'static str] = SHELVES;

    fn order(&self) -> &[String] {
        &self.order
    }

    fn hidden(&self) -> &[String] {
        &self.hidden
    }

    fn lists(&mut self) -> (&mut Vec<String>, &mut Vec<String>) {
        (&mut self.order, &mut self.hidden)
    }
}

/// Home while the active source has a catalog of its own: For you leads, then its feed, and the library shelves it repeats start hidden.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogHome {
    pub order: Vec<String>,
    pub hidden: Vec<String>,
}

impl Default for CatalogHome {
    fn default() -> Self {
        Self {
            order: CATALOG_SHELVES
                .iter()
                .map(|name| name.to_string())
                .collect(),
            hidden: CATALOG_SHELVES
                .iter()
                .filter(|name| !CATALOG_SHOWN.contains(name))
                .map(|name| name.to_string())
                .collect(),
        }
    }
}

/// The shelves a catalog Home can draw, in their designed order.
pub const CATALOG_SHELVES: &[&str] = &[
    "for_you",
    "catalog",
    "your_playlists",
    "jump_back_in",
    "recently_added",
    "made_for_you",
    "top_artists",
    "recently_played",
];

/// What a catalog Home shows before anyone arranges it.
const CATALOG_SHOWN: &[&str] = &["for_you", "catalog", "your_playlists"];

impl Ordered for CatalogHome {
    const KNOWN: &'static [&'static str] = CATALOG_SHELVES;

    fn order(&self) -> &[String] {
        &self.order
    }

    fn hidden(&self) -> &[String] {
        &self.hidden
    }

    fn lists(&mut self) -> (&mut Vec<String>, &mut Vec<String>) {
        (&mut self.order, &mut self.hidden)
    }
}

/// What the command palette remembers, and how much of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Palette {
    pub history: bool,
    pub recents: usize,
    pub current_track_first: bool,
    pub recent_commands: Vec<String>,
    pub recent_searches: Vec<String>,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            history: true,
            recents: 5,
            current_track_first: true,
            recent_commands: Vec::new(),
            recent_searches: Vec::new(),
        }
    }
}

impl Palette {
    /// Put `entry` at the front of `list`, deduped, keeping at most `keep` of them.
    pub fn remember(list: &mut Vec<String>, entry: String, keep: usize) {
        if entry.is_empty() || keep == 0 {
            return;
        }
        list.retain(|held| held != &entry);
        list.insert(0, entry);
        list.truncate(keep);
    }
}

/// Type size and face, over whatever the theme asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Interface {
    pub scale: f32,
    pub font: Option<String>,
}

impl Default for Interface {
    fn default() -> Self {
        Self {
            scale: 1.,
            font: None,
        }
    }
}

/// What interrupts, and what only lands in the notification centre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub errors: bool,
    /// Per job kind; a kind with no entry toasts.
    pub jobs: BTreeMap<String, bool>,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            errors: true,
            jobs: BTreeMap::new(),
        }
    }
}

impl Notifications {
    pub fn toasts(&self, kind: &str) -> bool {
        self.jobs.get(kind).copied().unwrap_or(true)
    }
}

/// Where the window was, so it opens there again.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub maximized: bool,
}

/// Every presentation preference; each top-level field is one stored key holding its JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub nav: Nav,
    pub panel: Panel,
    pub player: Player,
    pub sheet: Sheet,
    pub library: Library,
    /// The encoded facet and filter set the library was last left on, so a restart opens it again.
    pub library_view: Option<String>,
    pub home: Home,
    pub catalog_home: CatalogHome,
    pub interface: Interface,
    pub palette: Palette,
    pub notifications: Notifications,
    pub window: WindowState,
    pub accent: Accent,
    pub ambient: Ambient,
    pub motion: Motion,
    pub skin: String,
    /// Command id to keystroke, overriding what the registry binds by default.
    pub shortcuts: BTreeMap<String, String>,
    /// The after-setup tips card was closed, so it never shows again.
    pub first_use_hint_dismissed: bool,
    /// The sorts and view modes every Kopuz frontend keeps, under the same keys the daemon seeded.
    #[serde(flatten)]
    pub ui: config::UiPrefs,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            nav: Nav::default(),
            panel: Panel::default(),
            player: Player::default(),
            sheet: Sheet::default(),
            library: Library::default(),
            library_view: None,
            home: Home::default(),
            catalog_home: CatalogHome::default(),
            interface: Interface::default(),
            palette: Palette::default(),
            notifications: Notifications::default(),
            window: WindowState::default(),
            accent: Accent::default(),
            ambient: Ambient::default(),
            motion: Motion::default(),
            skin: "normal".into(),
            shortcuts: BTreeMap::new(),
            first_use_hint_dismissed: false,
            ui: config::UiPrefs::default(),
        }
    }
}

/// A group of preferences that resets together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Nav,
    Panel,
    Player,
    Sheet,
    Library,
    Home,
    Interface,
    Palette,
    Notifications,
    Shortcuts,
    Skin,
    All,
}

impl Section {
    fn reset(self, layout: &mut Layout) {
        let fresh = Layout::default();
        match self {
            Section::Nav => layout.nav = fresh.nav,
            Section::Panel => layout.panel = fresh.panel,
            Section::Player => layout.player = fresh.player,
            Section::Sheet => layout.sheet = fresh.sheet,
            Section::Library => {
                layout.library = fresh.library;
                layout.library_view = fresh.library_view;
                layout.ui.album_view_mode = fresh.ui.album_view_mode;
                layout.ui.album_sort = fresh.ui.album_sort;
                layout.ui.artists_view_mode = fresh.ui.artists_view_mode;
                layout.ui.artist_sort = fresh.ui.artist_sort;
            }
            Section::Home => {
                layout.home = fresh.home;
                layout.catalog_home = fresh.catalog_home;
            }
            Section::Interface => {
                layout.interface = fresh.interface;
                layout.accent = fresh.accent;
                layout.motion = fresh.motion;
            }
            Section::Palette => layout.palette = fresh.palette,
            Section::Notifications => layout.notifications = fresh.notifications,
            Section::Shortcuts => layout.shortcuts = fresh.shortcuts,
            Section::Skin => layout.skin = fresh.skin,
            // The window is where the person left it, not a preference; resetting all keeps it.
            Section::All => {
                let window = layout.window;
                *layout = fresh;
                layout.window = window;
            }
        }
    }
}

fn yes() -> bool {
    true
}

/// Where the old `layout.toml` lived, read once to carry it into the daemon; `KOPUZ_GPUI_LAYOUT` moves it.
pub fn layout_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KOPUZ_GPUI_LAYOUT") {
        return Some(PathBuf::from(path));
    }
    let base = directories::BaseDirs::new()?;
    Some(base.config_dir().join("kopuz-gpui").join("layout.toml"))
}

type Fields = BTreeMap<String, String>;

/// One stored key per top-level field, each holding that field as JSON.
fn fields(layout: &Layout) -> Fields {
    match serde_json::to_value(layout) {
        Ok(serde_json::Value::Object(object)) => object
            .into_iter()
            .map(|(key, value)| (key, value.to_string()))
            .collect(),
        _ => Fields::new(),
    }
}

/// `base` with every stored field laid over it; a value that no longer reads drops only itself.
fn overlay(base: &Layout, stored: &Fields) -> Layout {
    let Ok(serde_json::Value::Object(mut object)) = serde_json::to_value(base) else {
        return base.clone();
    };
    for (key, text) in stored {
        if !object.contains_key(key) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            tracing::warn!(key, "stored preference is not JSON");
            continue;
        };
        let previous = object.insert(key.clone(), value);
        if serde_json::from_value::<Layout>(serde_json::Value::Object(object.clone())).is_err() {
            tracing::warn!(key, "stored preference ignored");
            if let Some(previous) = previous {
                object.insert(key.clone(), previous);
            }
        }
    }
    serde_json::from_value(serde_json::Value::Object(object)).unwrap_or_else(|_| base.clone())
}

/// The app's own look, mirrored from the daemon's preference store and written back debounced.
pub struct LayoutModel {
    layout: Layout,
    /// What the daemon is known to hold, per field; a field that differs is waiting to be sent.
    synced: Fields,
    generation: u64,
    import_checked: bool,
    send_imported: bool,
    ready: Option<oneshot::Sender<()>>,
    waiter: Option<oneshot::Receiver<()>>,
    _load: Option<Task<()>>,
    _save: Option<Task<()>>,
    _subscription: Subscription,
}

impl LayoutModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let connection = Link::connection(cx);
        let subscription = cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| match event {
            ApiEvent::FrontendPrefsChanged { frontend, .. } if frontend == FRONTEND => {
                this.reload(cx)
            }
            ApiEvent::Resync => this.reload(cx),
            _ => {}
        });
        let layout = Layout::default();
        let (ready, waiter) = oneshot::channel();
        Self {
            synced: fields(&layout),
            layout,
            generation: 0,
            import_checked: false,
            send_imported: false,
            ready: Some(ready),
            waiter: Some(waiter),
            _load: None,
            _save: None,
            _subscription: subscription,
        }
    }

    pub fn get(&self) -> &Layout {
        &self.layout
    }

    /// Resolves once the daemon's preferences have been read the first time; taken once.
    pub fn take_ready(&mut self) -> Option<oneshot::Receiver<()>> {
        self.waiter.take()
    }

    /// Change a preference; the daemon catches up once edits settle.
    pub fn update(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Layout)) {
        let before = self.layout.clone();
        edit(&mut self.layout);
        if self.layout == before {
            return;
        }
        cx.notify();
        self.schedule_save(cx);
    }

    /// Put one group of preferences back the way it shipped.
    pub fn reset(&mut self, section: Section, cx: &mut Context<Self>) {
        self.update(cx, |layout| section.reset(layout));
    }

    /// Change a preference from anywhere holding an `App`.
    pub fn edit(cx: &mut App, edit: impl FnOnce(&mut Layout)) {
        Self::global(cx).update(cx, |this, cx| this.update(cx, edit));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let may_import = !self.import_checked;
        let call = Link::global(cx)
            .call(|api| async move { api.frontend_prefs(FRONTEND.to_string()).await });
        self._load = Some(cx.spawn(async move |this, cx| {
            let stored = match call.await {
                Ok(stored) => stored,
                Err(error) => {
                    tracing::warn!(%error, "could not read the layout from the daemon");
                    return;
                }
            };
            let imported = match stored.is_empty() && may_import {
                true => cx.background_executor().spawn(async { import() }).await,
                false => None,
            };
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.adopt(stored, imported, cx);
                }
            })
            .ok();
        }));
    }

    /// Take what the daemon holds, keeping any local edit that has not been sent yet.
    fn adopt(
        &mut self,
        stored: Vec<FrontendPref>,
        imported: Option<Layout>,
        cx: &mut Context<Self>,
    ) {
        let current = fields(&self.layout);
        let stored: Fields = stored
            .into_iter()
            .map(|pref| (pref.key, pref.value))
            .collect();
        let remote = overlay(&Layout::default(), &stored);
        let mut merged = fields(imported.as_ref().unwrap_or(&remote));
        for (key, value) in &current {
            if self.synced.get(key) != Some(value) {
                merged.insert(key.clone(), value.clone());
            }
        }
        self.synced = fields(&remote);
        self.import_checked = true;
        self.send_imported |= imported.is_some();
        let layout = overlay(&Layout::default(), &merged);
        if layout != self.layout {
            self.layout = layout;
            cx.notify();
        }
        if let Some(ready) = self.ready.take() {
            ready.send(()).ok();
        }
        if !self.unsent().is_empty() {
            self.schedule_save(cx);
        }
    }

    /// Every field the daemon does not hold yet, as the entries that would store it.
    fn unsent(&self) -> Vec<PrefEntry> {
        let mut entries: Vec<PrefEntry> = fields(&self.layout)
            .into_iter()
            .filter(|(key, value)| self.synced.get(key) != Some(value))
            .map(|(key, value)| PrefEntry {
                key,
                value: Some(value),
            })
            .collect();
        if self.send_imported {
            entries.push(PrefEntry {
                key: IMPORTED.into(),
                value: Some("1".into()),
            });
        }
        entries
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let link = Link::global(cx).clone();
        self._save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let Ok(entries) = this.read_with(cx, |this, _| this.unsent()) else {
                return;
            };
            if entries.is_empty() {
                return;
            }
            let sent = entries.clone();
            let call = link.call(move |api| async move {
                api.set_frontend_prefs(FRONTEND.to_string(), entries).await
            });
            if let Err(error) = call.await {
                tracing::warn!(%error, "could not store the layout in the daemon");
                return;
            }
            this.update(cx, |this, _| {
                for entry in sent {
                    match (entry.key.as_str(), entry.value) {
                        (IMPORTED, _) => this.send_imported = false,
                        (_, Some(value)) => {
                            this.synced.insert(entry.key, value);
                        }
                        (_, None) => {}
                    }
                }
            })
            .ok();
        }));
    }
}

/// The old file's look, if there is one; it is left where it is.
fn import() -> Option<Layout> {
    let path = layout_path()?;
    path.exists().then(|| read(&path))
}

/// An unreadable file is the default look, not a failure worth stopping for.
fn read(path: &std::path::Path) -> Layout {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Layout::default();
    };
    match toml::from_str(&text) {
        Ok(layout) => layout,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "layout file ignored");
            Layout::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_layout_round_trips() {
        let layout = Layout::default();
        let text = toml::to_string_pretty(&layout).expect("serialise");
        let back: Layout = toml::from_str(&text).expect("deserialise");
        assert_eq!(layout, back);
    }

    #[test]
    fn the_catalog_home_opens_on_for_you_then_the_feed_and_playlists() {
        let home = CatalogHome::default();
        assert_eq!(home.visible(), &["for_you", "catalog", "your_playlists"]);
        assert_eq!(home.order.len(), CATALOG_SHELVES.len());
    }

    #[test]
    fn an_empty_file_is_the_default_look() {
        let layout: Layout = toml::from_str("").expect("deserialise");
        assert_eq!(layout, Layout::default());
    }

    #[test]
    fn a_key_from_a_later_version_is_ignored() {
        let layout: Layout =
            toml::from_str("skin = \"neon\"\nwhat_is_this = 3\n").expect("deserialise");
        assert_eq!(layout.skin, "neon");
        assert_eq!(layout.nav, Nav::default());
    }

    #[test]
    fn a_partial_section_keeps_its_other_defaults() {
        let layout: Layout = toml::from_str("[panel]\nshown = true\n").expect("deserialise");
        assert!(layout.panel.shown);
        assert_eq!(layout.panel.width, Panel::default().width);
    }

    #[test]
    fn hiding_a_destination_drops_it_from_the_bar() {
        let mut nav = Nav::default();
        nav.hidden.push("radio".into());
        assert!(!nav.visible().contains(&"radio"));
        assert!(nav.visible().contains(&"home"));
    }

    #[test]
    fn a_destination_added_later_shows_up_at_the_end() {
        let nav = Nav {
            order: vec!["home".into(), "library".into()],
            hidden: Vec::new(),
            ..Nav::default()
        };
        let visible = nav.visible();
        assert_eq!(&visible[..2], &["home", "library"]);
        assert_eq!(visible.len(), DESTINATIONS.len());
    }

    #[test]
    fn the_bar_ships_holding_the_primary_destinations() {
        assert_eq!(Nav::default().visible(), PRIMARY);
    }

    #[test]
    fn showing_a_destination_puts_it_back_in_its_designed_place() {
        let mut nav = Nav::default();
        nav.hidden.retain(|name| name != "playlists");
        assert_eq!(
            nav.visible(),
            &[
                "home",
                "library",
                "discover",
                "radio",
                "playlists",
                "favorites"
            ]
        );
    }

    #[test]
    fn reordering_the_bar_survives_a_round_trip() {
        let mut nav = Nav::default();
        nav.order.swap(0, 1);
        let text = toml::to_string_pretty(&nav).expect("serialise");
        let back: Nav = toml::from_str(&text).expect("deserialise");
        assert_eq!(
            back.visible(),
            &["library", "home", "discover", "radio", "favorites"]
        );
    }

    #[test]
    fn moving_a_destination_changes_what_the_bar_draws() {
        let mut nav = Nav::default();
        nav.step("library", -1);
        assert_eq!(
            nav.visible(),
            &["library", "home", "discover", "radio", "favorites"]
        );
    }

    #[test]
    fn the_first_destination_cannot_walk_off_the_front() {
        let mut nav = Nav::default();
        nav.step("home", -1);
        assert_eq!(nav.visible(), Nav::default().visible());
    }

    #[test]
    fn a_partial_order_still_moves_the_right_one() {
        let mut nav = Nav {
            order: vec!["home".into()],
            hidden: Vec::new(),
            ..Nav::default()
        };
        nav.step("radio", -1);
        assert_eq!(
            &nav.visible()[..4],
            &["home", "library", "radio", "discover"]
        );
    }

    #[test]
    fn showing_and_hiding_a_name_is_idempotent() {
        let mut home = Home::default();
        home.show("top_artists", false);
        home.show("top_artists", false);
        assert_eq!(home.hidden, vec!["top_artists".to_string()]);
        home.show("top_artists", true);
        assert!(home.hidden.is_empty());
    }

    #[test]
    fn moving_a_shelf_changes_what_the_page_draws() {
        let mut home = Home::default();
        home.step("jump_back_in", -1);
        assert_eq!(&home.visible()[..2], &["jump_back_in", "for_you"]);
    }

    #[test]
    fn for_you_leads_a_fresh_home() {
        assert_eq!(
            &Home::default().visible()[..2],
            &["for_you", "jump_back_in"]
        );
    }

    fn saved(names: &[&str]) -> Home {
        Home {
            order: names.iter().map(|name| name.to_string()).collect(),
            hidden: Vec::new(),
        }
    }

    // The order a person saved before for_you existed, with their own moves in it.
    const BEFORE_FOR_YOU: &[&str] = &[
        "recently_added",
        "jump_back_in",
        "made_for_you",
        "your_playlists",
        "top_artists",
        "recently_played",
    ];

    #[test]
    fn a_shelf_new_to_a_saved_order_lands_by_its_designed_neighbour() {
        let home = saved(BEFORE_FOR_YOU);
        assert_eq!(
            home.visible(),
            &[
                "recently_added",
                "for_you",
                "jump_back_in",
                "made_for_you",
                "your_playlists",
                "top_artists",
                "recently_played",
            ]
        );
    }

    #[test]
    fn moving_a_shelf_in_an_old_order_keeps_the_new_one_by_its_neighbour() {
        let mut home = saved(BEFORE_FOR_YOU);
        home.step("recently_played", -1);
        assert_eq!(
            home.order,
            [
                "recently_added",
                "for_you",
                "jump_back_in",
                "made_for_you",
                "your_playlists",
                "recently_played",
                "top_artists",
            ]
        );
    }

    #[test]
    fn a_new_shelf_never_unhides_or_moves_the_ones_placed_by_hand() {
        let mut home = saved(BEFORE_FOR_YOU);
        home.hidden.push("jump_back_in".into());
        let visible = home.visible();
        assert!(!visible.contains(&"jump_back_in"));
        assert_eq!(&visible[..2], &["recently_added", "for_you"]);
        assert_eq!(visible.last(), Some(&"recently_played"));
    }

    #[test]
    fn a_new_last_name_follows_its_previous_neighbour() {
        let mut order = vec!["library".to_string(), "home".to_string()];
        place_missing(&mut order, &["library", "home", "later"]);
        assert_eq!(order, ["library", "home", "later"]);
    }

    #[test]
    fn a_hidden_new_shelf_stays_hidden() {
        let mut home = saved(BEFORE_FOR_YOU);
        home.hidden.push("for_you".into());
        assert!(!home.visible().contains(&"for_you"));
    }

    #[test]
    fn an_unknown_destination_name_is_dropped() {
        let nav = Nav {
            order: vec!["gone".into(), "home".into()],
            ..Nav::default()
        };
        assert!(!nav.visible().contains(&"gone"));
    }

    #[test]
    fn resetting_a_section_leaves_the_others_alone() {
        let mut layout = Layout::default();
        layout.panel.shown = true;
        layout.skin = "neon".into();
        Section::Panel.reset(&mut layout);
        assert!(!layout.panel.shown);
        assert_eq!(layout.skin, "neon");
    }

    #[test]
    fn resetting_everything_keeps_where_the_window_was() {
        let mut layout = Layout::default();
        layout.window.width = Some(1200.);
        layout.skin = "neon".into();
        Section::All.reset(&mut layout);
        assert_eq!(layout.window.width, Some(1200.));
        assert_eq!(layout.skin, "normal");
    }

    #[test]
    fn a_remembered_entry_moves_to_the_front_and_the_list_stays_capped() {
        let mut list = vec!["b".to_string(), "c".to_string()];
        Palette::remember(&mut list, "b".into(), 2);
        assert_eq!(list, ["b", "c"]);
        Palette::remember(&mut list, "a".into(), 2);
        assert_eq!(list, ["a", "b"]);
    }

    #[test]
    fn a_job_kind_nobody_configured_still_toasts() {
        let notifications = Notifications::default();
        assert!(notifications.toasts("scan"));
    }

    #[test]
    fn what_is_stored_is_what_comes_back() {
        let mut layout = Layout::default();
        layout.panel.shown = true;
        layout.interface.scale = 1.25;
        layout.nav.position = NavPosition::Left;
        layout
            .shortcuts
            .insert("play.toggle".into(), "ctrl-space".into());
        assert_eq!(overlay(&Layout::default(), &fields(&layout)), layout);
    }

    #[test]
    fn a_stored_player_without_a_size_keeps_the_normal_bar() {
        let stored = Fields::from([(
            "player".to_string(),
            "{\"cover\": \"large\", \"shuffle\": false}".to_string(),
        )]);
        let layout = overlay(&Layout::default(), &stored);
        assert_eq!(layout.player.size, PlayerSize::Normal);
        assert_eq!(layout.player.cover, TileSize::Large);
        assert!(!layout.player.shuffle);
    }

    #[test]
    fn each_player_size_is_stored_and_resets_with_the_player() {
        for size in [PlayerSize::Small, PlayerSize::Normal, PlayerSize::Large] {
            let mut layout = Layout::default();
            layout.player.size = size;
            layout.interface.scale = 1.25;
            let mut stored = overlay(&Layout::default(), &fields(&layout));
            assert_eq!(stored.player.size, size);
            Section::Player.reset(&mut stored);
            assert_eq!(stored.player.size, PlayerSize::Normal);
            assert_eq!(stored.interface.scale, 1.25);
        }
    }

    #[test]
    fn every_top_level_field_is_its_own_key() {
        let keys: Vec<String> = fields(&Layout::default()).into_keys().collect();
        assert!(keys.contains(&"nav".to_string()));
        assert!(keys.contains(&"window".to_string()));
        assert!(!keys.contains(&IMPORTED.to_string()));
        assert!(keys.contains(&"album_sort".to_string()));
        assert!(!keys.contains(&"ui".to_string()));
    }

    #[test]
    fn a_sort_the_daemon_seeded_reads_back() {
        let stored = Fields::from([(
            "album_sort".to_string(),
            r#"[{"field":"Year","direction":"Desc"}]"#.to_string(),
        )]);
        let layout = overlay(&Layout::default(), &stored);
        assert_eq!(
            layout.ui.album_sort,
            vec![config::SortCriterion::new(
                config::AlbumSortField::Year,
                config::SortDirection::Desc
            )]
        );
    }

    #[test]
    fn the_albums_facet_reads_and_writes_the_seeded_prefs() {
        let stored = Fields::from([
            ("album_view_mode".to_string(), r#""List""#.to_string()),
            (
                "album_sort".to_string(),
                r#"[{"field":"Year","direction":"Desc"},{"field":"Title","direction":"Asc"}]"#
                    .to_string(),
            ),
            (
                "artist_sort".to_string(),
                r#"[{"field":"Albums","direction":"Desc"}]"#.to_string(),
            ),
        ]);
        let mut layout = overlay(&Layout::default(), &stored);
        let albums = layout.library_facet("albums");
        assert_eq!(albums.mode, ViewMode::Table);
        assert_eq!(albums.sort.field, "year");
        assert!(albums.sort.descending);
        assert_eq!(layout.library_facet("artists").sort.field, "albums");

        let stack = layout.ui.album_sort.clone();
        layout.edit_library_facet("albums", |facet| facet.tile = TileSize::Large);
        assert_eq!(layout.ui.album_sort, stack);

        layout.edit_library_facet("albums", |facet| {
            facet.mode = ViewMode::Grid;
            facet.sort = Sort {
                field: "artist".into(),
                descending: false,
            };
        });
        assert_eq!(layout.ui.album_view_mode, AlbumViewMode::Grid);
        assert_eq!(
            layout.ui.album_sort,
            vec![SortCriterion::new(
                AlbumSortField::Artist,
                SortDirection::Asc
            )]
        );
    }

    #[test]
    fn a_value_that_no_longer_reads_drops_only_itself() {
        let stored = Fields::from([
            ("skin".to_string(), "\"neon\"".to_string()),
            ("panel".to_string(), "{\"shown\": \"yes\"}".to_string()),
            ("motion".to_string(), "not json".to_string()),
            (IMPORTED.to_string(), "1".to_string()),
        ]);
        let layout = overlay(&Layout::default(), &stored);
        assert_eq!(layout.skin, "neon");
        assert_eq!(layout.panel, Panel::default());
        assert_eq!(layout.motion, Motion::default());
    }

    #[test]
    fn the_old_file_still_reads_for_its_import() {
        let dir = std::env::temp_dir().join(format!("kopuz-layout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("layout.toml");
        std::fs::write(&path, "skin = \"neon\"\n[panel]\nshown = true\n").expect("write");
        let layout = read(&path);
        assert_eq!(layout.skin, "neon");
        assert!(layout.panel.shown);
        std::fs::remove_dir_all(&dir).ok();
    }
}
