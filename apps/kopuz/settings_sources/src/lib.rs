//! The Library and Connectivity settings: sources, radio registries, integrations.

pub mod add_source;
pub mod folders;
pub mod integrations;
mod login;
pub mod offline_cache;
mod reauth;
mod registries;
mod schedule;
mod sources;
pub mod ui;

use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled, Window,
};

use integrations::Integrations;
use offline_cache::OfflineCache;
use registries::RadioRegistries;
use sources::Sources;

/// Let a source switched from anywhere — the sidebar included — open the password form.
///
/// Which sign-in a source takes is the daemon's answer and rides on the switch; the form it may
/// need lives in this crate, so `store` is handed a way to reach it.
pub fn register(cx: &mut gpui_kit::App) {
    store::register_password_form(login::open, cx);
    store::register_session_chooser(reauth::open, cx);
}

pub struct SourceSettings {
    sources: Entity<Sources>,
    registries: Entity<RadioRegistries>,
    integrations: Entity<Integrations>,
    offline_cache: Entity<OfflineCache>,
    section: Option<SharedString>,
}

impl SourceSettings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            sources: cx.new(|cx| Sources::new(window, cx)),
            registries: cx.new(|cx| RadioRegistries::new(window, cx)),
            integrations: cx.new(|cx| Integrations::new(window, cx)),
            offline_cache: cx.new(|cx| OfflineCache::new(window, cx)),
            section: None,
        }
    }
}

impl SourceSettings {
    /// Which rail entry is showing; `None` stacks every section.
    pub fn show(&mut self, section: Option<SharedString>, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    fn shows(&self, section: &str) -> bool {
        self.section.as_deref().is_none_or(|shown| shown == section)
    }
}

impl Render for SourceSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The settings page owns the scroll; this panel is content inside it.
        v_flex()
            .id("source-settings")
            .w_full()
            .gap_4()
            .pb_4()
            .when(self.shows("library"), |panel| {
                panel.child(self.sources.clone())
            })
            .when(self.shows("library"), |panel| {
                panel.child(schedule::settings(cx))
            })
            .when(self.shows("library"), |panel| {
                panel.child(self.offline_cache.clone())
            })
            .when(self.shows("library"), |panel| {
                panel.child(self.registries.clone())
            })
            .when(self.shows("connectivity"), |panel| {
                panel.child(self.integrations.clone())
            })
    }
}
