//! Settings → Interface: every knob that changes how the app looks, contributed by the crate that owns it.

mod apply;
mod font;
mod general;
mod skin;

use std::rc::Rc;

use components::format::t;
use components::{EmptyState, icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyView, App, Context, Global, IntoElement, ParentElement as _, Render, SharedString, Styled,
    Window,
};
use store::layout::Section as Group;

/// What builds one section's view, once, when the Interface panel is first drawn.
type Build = Rc<dyn Fn(&mut Window, &mut App) -> AnyView>;

struct Entry {
    id: &'static str,
    title: &'static str,
    order: u16,
    reset: Option<Group>,
    build: Build,
}

#[derive(Default)]
struct Registry(Vec<Entry>);

impl Global for Registry {}

/// Contribute a section. `title` is an i18n key, read at render so a locale change reaches it.
///
/// A crate registers its own knobs, so no two packages ever edit the same settings file.
pub fn register(
    id: &'static str,
    title: &'static str,
    order: u16,
    reset: Option<Group>,
    build: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    cx: &mut App,
) {
    let entry = Entry {
        id,
        title,
        order,
        reset,
        build: Rc::new(build),
    };
    let registry = cx.default_global::<Registry>();
    registry.0.retain(|held| held.id != entry.id);
    registry.0.push(entry);
    registry.0.sort_by_key(|entry| entry.order);
}

/// Register the sections this crate owns; call once at startup, before the others.
pub fn init(cx: &mut App) {
    general::register(cx);
    skin::register(cx);
    apply::watch(cx);
}

/// One drawn section, with the group its reset button puts back.
struct Panel {
    title: &'static str,
    reset: Option<Group>,
    view: AnyView,
}

/// The Interface panel: whatever has been registered, in order.
pub struct InterfaceSettings {
    panels: Vec<Panel>,
}

impl InterfaceSettings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let entries: Vec<(&'static str, Option<Group>, Build)> = cx
            .try_global::<Registry>()
            .map(|registry| {
                registry
                    .0
                    .iter()
                    .map(|entry| (entry.title, entry.reset, entry.build.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let panels = entries
            .into_iter()
            .map(|(title, reset, build)| Panel {
                title,
                reset,
                view: build(window, cx),
            })
            .collect();
        Self { panels }
    }
}

impl Render for InterfaceSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.panels.is_empty() {
            return v_flex()
                .w_full()
                .child(EmptyState::new(t("settings_interface")).icon(icons::GRID))
                .into_any_element();
        }
        // A filtered page is a list of matching rows; a "put this back" button is not one.
        let filtering = components::settings::query(cx).is_some();
        let sections: Vec<_> = self
            .panels
            .iter()
            .map(|panel| {
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(panel.view.clone())
                    .children(
                        panel
                            .reset
                            .filter(|_| !filtering)
                            .map(|group| reset_row(panel.title, group, cx)),
                    )
            })
            .collect();
        v_flex()
            .w_full()
            .gap_5()
            .children(sections)
            .when(!filtering, |panel| {
                panel.child(reset_row("settings_interface", Group::All, cx))
            })
            .into_any_element()
    }
}

/// The quiet "put this back" under a section, so nothing is only resettable by editing the file.
fn reset_row(title: &'static str, group: Group, cx: &App) -> impl IntoElement {
    h_flex().w_full().justify_end().child(
        Button::new(SharedString::from(format!("reset-{title}")))
            .ghost()
            .small()
            .label(t("reset_home"))
            .icon(icons::icon(icons::REFRESH))
            .text_color(style::white(style::TEXT_SECONDARY, cx))
            .on_click(move |_, _, cx| {
                store::LayoutModel::global(cx).update(cx, |this, cx| this.reset(group, cx));
            }),
    )
}
