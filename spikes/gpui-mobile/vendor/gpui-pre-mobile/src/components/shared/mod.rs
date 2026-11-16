//! # Shared Components
//!
//! Cross-platform UI patterns that work with any design language — progress
//! bars, avatars, badges, stat cards, and skeleton loaders.
//!
//! These components use a neutral Catppuccin Mocha colour palette by default
//! and adapt to dark / light mode. They can be used alongside both the
//! [`super::glass`] and [`super::material`] design-language modules.
//!
//! ## Components
//!
//! | Component | Description |
//! |-----------|-------------|
//! | [`progress_bars`] / [`progress_row`] | Labelled progress bar tracks |
//! | [`avatars`] | Avatar circles with initials, status indicators, and stacked groups |
//! | [`avatar`] / [`avatar_status`] | Individual avatar primitives |
//! | [`badges`] | Solid, outline, and dot-indicator badge variants |
//! | [`badge_solid`] / [`badge_outline`] / [`icon_with_badge`] | Individual badge primitives |
//! | [`stat_cards`] / [`stat_card`] | Metric cards with title, value, and trend |
//! | [`skeleton_loaders`] | Placeholder loading skeletons for cards, text, and images |

pub mod avatars;
pub mod badges;
pub mod progress;
pub mod skeleton;
pub mod stat_cards;

// ── Convenience re-exports ───────────────────────────────────────────────────

// Progress
pub use progress::{progress_bars, progress_row};

// Avatars
pub use avatars::{avatar, avatar_status, avatars};

// Badges
pub use badges::{badge_outline, badge_solid, badges, icon_with_badge};

// Stat cards
pub use stat_cards::{stat_card, stat_cards};

// Skeleton loaders
pub use skeleton::skeleton_loaders;
