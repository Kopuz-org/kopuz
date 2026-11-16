//! # Apple Glass Components
//!
//! Frosted translucent panels, vibrancy, SF-style controls, thin separators,
//! and subtle depth — inspired by visionOS and iOS design language.
//!
//! ## Components
//!
//! | Component | Description |
//! |-----------|-------------|
//! | [`panel`] | Frosted-glass base container with translucent bg + border |
//! | [`separator`] / [`separator_full`] | Thin 0.5 px divider lines |
//! | [`hero_card`] | Gradient mesh background + glass overlay card |
//! | [`buttons_row`] | Tinted, plain, and capsule button rows |
//! | [`button_tinted`] / [`button_plain`] | Individual button styles |
//! | [`segmented_control`] | iOS-style multi-segment picker |
//! | [`settings_list`] | Grouped settings rows with toggles and chevrons |
//! | [`toggle`] | iOS-style on/off toggle switch |
//! | [`notification_banners`] / [`notification`] | Banner-style alerts |
//! | [`search_bar`] | Rounded search input with icon and clear button |
//! | [`sliders`] / [`slider_row`] | Labelled slider tracks with thumbs |
//! | [`tab_bar`] | Bottom tab bar with icon + label items |

pub mod buttons;
pub mod hero_card;
pub mod notification;
pub mod panel;
pub mod search_bar;
pub mod segmented_control;
pub mod settings_list;
pub mod slider;
pub mod tab_bar;

// ── Convenience re-exports ───────────────────────────────────────────────────

// Panel & separators
pub use panel::{panel, separator, separator_full};

// Hero card
pub use hero_card::hero_card;

// Buttons
pub use buttons::{button_plain, button_tinted, buttons_row};

// Segmented control
pub use segmented_control::segmented_control;

// Settings list & toggle
pub use settings_list::{settings_list, settings_row, toggle};

// Notifications
pub use notification::{notification, notification_banners};

// Search bar
pub use search_bar::search_bar;

// Sliders
pub use slider::{slider_row, sliders};

// Tab bar
pub use tab_bar::tab_bar;
