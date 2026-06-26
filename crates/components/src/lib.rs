//! Reusable Dioxus UI components for the Kopuz music player: modern/normal list views,
//! navigation controller, titlebar, sidebar, bottombar, and shared UI primitives.

/// `KOPUZ_BLITZ=1` selects the native (Blitz/wgpu) renderer instead of the
/// webview. Process-constant (read once), so renderer-conditional hooks keep a
/// stable order across renders.
pub fn blitz_active() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KOPUZ_BLITZ").is_some_and(|v| v == "1"))
}

pub mod modern;
pub mod navigation_controller;
pub mod normal;
pub mod shared;
pub use navigation_controller::NavigationController;
pub mod album_play_button;
pub mod bottombar;
pub mod compact_player;
pub use compact_player::{CompactMode, CompactPlayer};
pub mod constants;
pub mod dots_menu;
pub mod download_overlay;
pub mod folder_picker;
pub mod fullscreen;
pub mod header;
pub mod lyrics_view;
pub mod metadata_modal;
pub mod playlist_detail;
pub mod playlist_modal;
pub mod playlist_popups;
pub mod queue_drag;
pub mod queue_list_view;
pub mod reorder_buttons;
pub mod rightbar;
pub mod search_bar;
pub mod search_genre_detail;
pub mod search_genres;
pub mod search_results;
pub mod selection_bar;
pub mod settings_items;
pub mod settings_popups;
pub mod showcase;
pub mod sidebar;
pub mod source_switcher;
pub mod stat_card;
pub mod titlebar;
pub mod track_list_view;
pub mod track_row;
pub mod virtual_scroll;
