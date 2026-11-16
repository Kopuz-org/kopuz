//! # GPUI Mobile Components
//!
//! A library of ready-to-use UI components built with raw GPUI primitives,
//! organised into three design-language modules:
//!
//! - **[`glass`]** — Apple-style frosted translucent panels, vibrancy,
//!   SF-style controls, thin separators, and subtle depth.
//! - **[`material`]** — Material Design 3 elevated cards, FABs,
//!   filled/outlined buttons, chips, text fields, snackbars, and bottom sheets.
//! - **[`shared`]** — Cross-platform patterns that work with any design
//!   language: progress bars, avatars, badges, stat cards, and skeleton loaders.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use gpui_mobile::components::glass;
//! use gpui_mobile::components::material;
//! use gpui_mobile::components::shared;
//!
//! // Use individual components
//! let panel = glass::panel(dark);
//! let card  = material::hero_card(dark);
//! let bars  = shared::progress_bars(dark);
//! ```
//!
//! Every public function in the sub-modules returns `impl IntoElement` (or
//! `gpui::Div` for base containers) so they compose naturally with GPUI's
//! builder API.

pub mod common;
pub mod glass;
pub mod material;
pub mod platform_view_element;
pub mod shared;

// ── Convenience re-exports ───────────────────────────────────────────────────

pub use common::{design_language_header, section_label};
