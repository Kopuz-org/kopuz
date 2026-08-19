//! Reusable Dioxus UI components for the Kopuz music player.

pub mod common;
pub mod layout;
pub mod navigation;
pub mod playback;
pub mod playlist;
pub mod queue;
pub mod search;
pub mod settings;
pub mod track;

pub use common::controls::{
    dots_menu, reorder_buttons, selection_bar, sort_control, view_mode_toggle,
};
pub use common::{constants, gestures, shared, virtual_scroll};
pub use layout::{
    bottombar, download_overlay, fullscreen, header, normal, rightbar, showcase, sidebar,
    stat_card, titlebar, vaxry,
};
pub use navigation::controller::{NavSnapshot, NavigationController};
pub use navigation::{back_button, controller as navigation_controller, source_switcher};
pub use playback::compact::{CompactMode, CompactPlayer};
pub use playback::cover_background::{CoverArtBackground, high_quality_artwork_url};
pub use playback::{
    album_play_button, compact as compact_player, controls as player_controls, cover_background,
    lyrics as lyrics_view, radio_actions, spotify_devices,
};
pub use playlist::{
    detail as playlist_detail, folder_picker, modal as playlist_modal, popups as playlist_popups,
};
pub use queue::{drag as queue_drag, list_view as queue_list_view};
pub use search::quick::QuickSearch;
pub use search::{
    bar as search_bar, genre_detail as search_genre_detail, genres as search_genres,
    quick as quick_search, results as search_results,
};
pub use settings::{items as settings_items, popups as settings_popups};
pub use track::{list_view as track_list_view, metadata_modal, row as track_row};
