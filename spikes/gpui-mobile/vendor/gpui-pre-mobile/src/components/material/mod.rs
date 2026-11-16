//! # Material Design 3 Components
//!
//! A comprehensive library of Material Design 3 components built with raw
//! GPUI primitives. These components follow the MD3 specification for
//! color, typography, shape, and motion tokens.
//!
//! ## Architecture
//!
//! All components use a **builder pattern** and implement `IntoElement`,
//! making them composable with GPUI's standard `.child(...)` API. Click
//! handlers use GPUI's `on_mouse_down` signature, so `cx.listener(...)`
//! works directly.
//!
//! ## Theme System
//!
//! Components consume a [`theme::MaterialTheme`] struct that provides all
//! MD3 color tokens. Use `MaterialTheme::dark()` or `MaterialTheme::light()`
//! for the baseline schemes, or `MaterialTheme::from_appearance(dark)` for
//! dynamic switching.
//!
//! ## Component Catalog
//!
//! ### Theme & Tokens
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`theme`] | MD3 color tokens, type scale, shape scale, elevation, motion |
//!
//! ### Layout & Scaffold
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`scaffold`] | Top-level visual layout: top bar, body, bottom bar, FAB, drawer |
//! | [`surface`] | Elevated card base with shadow simulation |
//!
//! ### Navigation
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`app_bar`] | Top app bars: center-aligned, small, medium, large |
//! | [`navigation_bar`] | Bottom navigation bar with icon + label items |
//! | [`navigation_rail`] | Vertical navigation rail for tablet/desktop |
//! | [`navigation_drawer`] | Standard and modal navigation drawers |
//! | [`tab_bar`] | Primary and secondary tab bars |
//!
//! ### Buttons & Actions
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`button`] | ElevatedButton, FilledButton, FilledTonalButton, OutlinedButton, TextButton, IconButton |
//! | [`fab`] | FloatingActionButton (small, regular, large) and ExtendedFab |
//! | [`list_tile`] | SegmentedButton, BottomAppBar |
//!
//! ### Form Controls
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`controls`] | Checkbox, Radio, RadioGroup, Switch, Slider, RangeSlider |
//! | [`text_fields`] | Outlined, filled, and error-state text fields |
//!
//! ### Containers & Content
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`card`] | Card (filled, elevated, outlined) and CardBuilder |
//! | [`list_tile`] | ListTile, Divider, Badge, Tooltip, Chip variants, ExpansionTile |
//! | [`hero_card`] | Gradient hero card for showcases |
//!
//! ### Feedback & Overlays
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`snackbar`] | Single-line and multi-line snackbar banners |
//! | [`bottom_sheet`] | Modal bottom sheet with drag handle |
//!
//! ### Legacy Compatibility
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`buttons`] | Legacy free-function button showcase (deprecated, use [`button`]) |
//! | [`fabs`] | Legacy free-function FAB showcase (deprecated, use [`fab`]) |
//! | [`chips`] | Legacy free-function chip showcase (deprecated, use [`list_tile::Chip`]) |
//! | [`cards`] | Legacy free-function card showcase (deprecated, use [`card`]) |
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::theme::MaterialTheme;
//! use gpui_mobile::components::material::button::FilledButton;
//! use gpui_mobile::components::material::card::CardBuilder;
//! use gpui_mobile::components::material::scaffold::Scaffold;
//!
//! let theme = MaterialTheme::dark();
//!
//! let my_button = FilledButton::new("Accept", theme)
//!     .on_click(cx.listener(|this, _, _, cx| { /* ... */ }));
//!
//! let my_card = CardBuilder::new(theme)
//!     .title("Hello")
//!     .body("World")
//!     .build();
//! ```

// ═══════════════════════════════════════════════════════════════════════════════
//  Module declarations
// ═══════════════════════════════════════════════════════════════════════════════

// ── Theme & tokens ───────────────────────────────────────────────────────────
pub mod theme;

// ── Layout & scaffold ────────────────────────────────────────────────────────
pub mod scaffold;
pub mod surface;

// ── Navigation ───────────────────────────────────────────────────────────────
pub mod app_bar;
pub mod navigation_bar;
pub mod navigation_drawer;
pub mod navigation_rail;
pub mod tab_bar;

// ── Buttons & actions ────────────────────────────────────────────────────────
pub mod button;
pub mod fab;

// ── Form controls ────────────────────────────────────────────────────────────
pub mod controls;
pub mod text_field;
pub mod text_fields;
pub mod text_input;

// ── Feedback & overlays ──────────────────────────────────────────────────────
pub mod dialog;
pub mod menu;
pub mod progress_indicator;
pub mod search_bar;

// ── Containers & content ─────────────────────────────────────────────────────
pub mod card;
pub mod hero_card;
pub mod list_tile;

pub mod bottom_sheet;
pub mod snackbar;

// ── Legacy modules (deprecated — kept for backward compatibility) ─────────────
pub mod buttons;
pub mod cards;
pub mod chips;
pub mod fabs;

// ═══════════════════════════════════════════════════════════════════════════════
//  Convenience re-exports — Theme
// ═══════════════════════════════════════════════════════════════════════════════

pub use theme::{ElevationLevel, MaterialTheme, MotionDuration, ShapeScale, StateLayer, TypeScale};

// ═══════════════════════════════════════════════════════════════════════════════
//  Convenience re-exports — Layout
// ═══════════════════════════════════════════════════════════════════════════════

pub use scaffold::Scaffold;
pub use surface::surface;

// ═══════════════════════════════════════════════════════════════════════════════
//  Convenience re-exports — Navigation
// ═══════════════════════════════════════════════════════════════════════════════

pub use app_bar::TopAppBar;
pub use navigation_bar::{NavigationBarBuilder, NavigationItem};
pub use navigation_drawer::{ModalNavigationDrawer, NavigationDrawer};
pub use navigation_rail::NavigationRail;
pub use tab_bar::TabBar;

// ═══════════════════════════════════════════════════════════════════════════════
//  Convenience re-exports — Buttons & Actions
// ═══════════════════════════════════════════════════════════════════════════════

pub use button::{
    ElevatedButton, FilledButton, FilledTonalButton, IconButton, IconButtonStyle, OutlinedButton,
    TextButton,
};
pub use fab::{ExtendedFab, FabColor, FabSize, FloatingActionButton};

// ═══════════════════════════════════════════════════════════════════════════════
//  Convenience re-exports — Form Controls
// ═══════════════════════════════════════════════════════════════════════════════

pub use controls::{Checkbox, CheckboxState, Radio, RadioGroup, RangeSlider, Slider, Switch};

// ═══════════════════════════════════════════════════════════════════════════════
//  Convenience re-exports — Containers & Content
// ═══════════════════════════════════════════════════════════════════════════════

pub use card::{Card, CardBuilder, CardVariant};
pub use hero_card::hero_card;
pub use list_tile::{
    Badge, BadgeType, BottomAppBar, Chip, ChipType, Divider, ExpansionTile, ListTile,
    ListTileDensity, SegmentedButton, SegmentedButtonMode, Tooltip, TooltipVariant,
};

// ═══════════════════════════════════════════════════════════════════════════════
//  Convenience re-exports — Feedback & Overlays
// ═══════════════════════════════════════════════════════════════════════════════

pub use bottom_sheet::{bottom_sheet, sheet_item};
pub use dialog::{BasicDialog, FullScreenDialog, SimpleDialog};
pub use menu::{Menu, MenuAnchor};
pub use progress_indicator::{CircularProgressIndicator, LinearProgressIndicator};
pub use search_bar::{SearchBar, SearchView};
pub use snackbar::snackbar;
pub use text_field::TextField;
pub use text_input::TextInput;

// ═══════════════════════════════════════════════════════════════════════════════
//  Convenience re-exports — Legacy (deprecated)
// ═══════════════════════════════════════════════════════════════════════════════

// Legacy button free functions — prefer the struct-based builders in `button`
pub use button::{button_filled, button_outlined, button_text, button_tonal, buttons};

// Legacy FAB showcase — prefer `FloatingActionButton` and `ExtendedFab`
pub use fab::fabs;

// Legacy chip helpers — prefer `Chip` with explicit `ChipType`
pub use list_tile::{chip, chips};

// Legacy card showcase — prefer `Card` / `CardBuilder`
pub use card::cards;

// Navigation bar demo (non-interactive showcase)
pub use navigation_bar::navigation_bar_demo;

// Text fields showcase
pub use text_fields::text_fields;

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo re-exports — for component showcase galleries
// ═══════════════════════════════════════════════════════════════════════════════

/// Demo / showcase functions for component galleries.
///
/// Each function renders a comprehensive, static (non-interactive) demo
/// of a component category. Pass `true` for dark mode, `false` for light.
pub mod demos {
    pub use super::app_bar::app_bar_demo;
    pub use super::button::button_demo;
    pub use super::card::card_demo;
    pub use super::controls::controls_demo;
    pub use super::dialog::dialog_demo;
    pub use super::fab::fab_demo;
    pub use super::list_tile::list_tile_demo;
    pub use super::menu::menu_demo;
    pub use super::navigation_drawer::navigation_drawer_demo;
    pub use super::navigation_rail::navigation_rail_demo;
    pub use super::progress_indicator::{
        progress_indicator_demo, progress_indicator_demo_animated,
    };
    pub use super::scaffold::scaffold_demo;
    pub use super::search_bar::search_bar_demo;
    pub use super::tab_bar::tab_bar_demo;
}
