//! The docked column beside the content: the same queue and lyrics the sheet draws, in a narrower slot.

use api::PluginPanel;
use components::format::t;
use components::{DockPanel, Edge, Segment, Segmented, icons, motion, plugin_ui};
use gpui_kit::{
    AnyElement, AppContext as _, Context, IntoElement, ParentElement as _, Pixels, SharedString,
    Styled as _, Window, div, px,
};
use store::layout::SheetTab;
use store::{Chrome, SidePanel};

use crate::NowPlaying;

// The Dioxus rightbar's drag bounds, and the width the shell's slot starts at.
const MIN_WIDTH: Pixels = px(280.);
const MAX_WIDTH: Pixels = px(600.);
const DEFAULT_WIDTH: Pixels = px(320.);

/// The longest a plugin's tab title is drawn, so a few tabs still fit the narrowest column
/// beside its close button.
const TAB_TITLE: usize = 14;

/// A plugin's title, cut to fit its tab.
fn tab_label(title: &str) -> SharedString {
    match title.chars().count() > TAB_TITLE {
        true => {
            let cut: String = title.chars().take(TAB_TITLE - 1).collect();
            SharedString::from(format!("{}…", cut.trim_end()))
        }
        false => SharedString::from(title.to_string()),
    }
}

/// A tab in the strip: one of the player's own, or one a plugin adds.
#[derive(Clone)]
enum Tab {
    Own(SheetTab, &'static str),
    Plugin(PluginPanel),
}

impl NowPlaying {
    /// What the docked column has been dragged to; the shell sizes the slot it puts it in.
    pub fn panel_width(&self) -> Pixels {
        self.panel_width.unwrap_or(DEFAULT_WIDTH)
    }

    pub(crate) fn render_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Devices is the bar's popover; this column only ever shows the queue or the lyrics.
        let showing = match Chrome::global(cx).read(cx).panel() {
            Some(SidePanel::Lyrics) => SheetTab::Lyrics,
            Some(SidePanel::Queue) => SheetTab::Queue,
            Some(SidePanel::Plugin) => SheetTab::Plugin,
            // A closing column keeps what it showed while the shell fades it out.
            _ => match self.tab_drawn {
                Some(tab) => tab,
                None => return div().into_any_element(),
            },
        };
        let panel = cx.entity().downgrade();
        // Resolved first, so the strip lights the tab the body then draws.
        let showing = self.resolve_tab(showing, cx);
        let header = self.tab_strip(showing, false, cx);
        let body = self.tab_body(showing, window, cx);

        let edge = match (store::layout(cx).interface.queue_side == store::layout::Side::Right)
            != i18n::is_rtl()
        {
            true => Edge::Left,
            false => Edge::Right,
        };
        DockPanel::new("panel", edge)
            .width(self.panel_width())
            .header(header)
            .body(body)
            .on_close(|_, cx| {
                Chrome::global(cx).update(cx, |chrome, cx| chrome.set_panel(None, cx))
            })
            .resizable(
                self.panel_width(),
                MIN_WIDTH,
                MAX_WIDTH,
                move |width, _, cx| {
                    panel
                        .update(cx, |this, cx| {
                            if this.panel_width != Some(width) {
                                this.panel_width = Some(width);
                                store::LayoutModel::edit(cx, move |layout| {
                                    layout.interface.queue_width = Some(width.as_f32());
                                });
                                cx.notify();
                            }
                        })
                        .ok();
                },
            )
            .into_any_element()
    }

    /// The queue / lyrics / related picker, shared by the column and the sheet.
    pub(crate) fn tab_strip(
        &self,
        showing: SheetTab,
        with_related: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut tabs: Vec<Tab> = vec![
            Tab::Own(SheetTab::Queue, "up_next"),
            Tab::Own(SheetTab::Lyrics, "lyrics"),
        ];
        if with_related {
            tabs.push(Tab::Own(SheetTab::Related, "related"));
        }
        tabs.extend(plugin_ui::now_playing_tabs(cx).into_iter().map(Tab::Plugin));
        let picked = self.plugin_tab.as_ref().map(|tab| tab.read(cx));
        let selected = tabs.iter().position(|tab| match tab {
            Tab::Own(tab, _) => *tab == showing,
            Tab::Plugin(panel) => {
                showing == SheetTab::Plugin && picked.is_some_and(|picked| picked.is(panel))
            }
        });
        let page = cx.entity().downgrade();
        Segmented::new("now-playing-tabs")
            .surface(theme::Surface::Sheet)
            .segments(tabs.iter().enumerate().map(|(at, tab)| {
                match tab {
                    Tab::Own(_, key) => Segment::new(*key).label(t(key)),
                    Tab::Plugin(panel) => {
                        Segment::new(SharedString::from(format!("plugin-tab-{at}")))
                            .label(tab_label(&panel.title))
                    }
                }
            }))
            .selected(selected)
            .on_select(move |index, _, cx| {
                let Some(tab) = tabs.get(*index).cloned() else {
                    return;
                };
                page.update(cx, |this, cx| match tab {
                    Tab::Own(tab, _) => this.set_tab(tab, cx),
                    Tab::Plugin(panel) => this.show_plugin(&panel, cx),
                })
                .ok();
            })
    }

    /// Show a plugin's tab, keeping the view when it is the one already picked.
    pub(crate) fn show_plugin(&mut self, panel: &PluginPanel, cx: &mut Context<Self>) {
        let picked = self
            .plugin_tab
            .as_ref()
            .is_some_and(|tab| tab.read(cx).is(panel));
        if !picked {
            self.plugin_tab = Some(cx.new(|cx| plugin_ui::PluginTab::new(panel, cx)));
        }
        self.set_tab(SheetTab::Plugin, cx);
    }

    /// The plugin tab to draw, picking the first one when the app opened on a
    /// plugin's tab; none once no plugin offers one.
    fn plugin_tab_to_draw(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let offered = plugin_ui::now_playing_tabs(cx);
        let kept = self
            .plugin_tab
            .as_ref()
            .is_some_and(|tab| offered.iter().any(|panel| tab.read(cx).is(panel)));
        if !kept {
            self.plugin_tab = offered
                .first()
                .map(|panel| cx.new(|cx| plugin_ui::PluginTab::new(panel, cx)));
        }
        self.plugin_tab.clone().map(IntoElement::into_any_element)
    }

    /// The tab to draw for `showing`: a plugin's whose plugin is gone is the queue instead.
    pub(crate) fn resolve_tab(&mut self, showing: SheetTab, cx: &mut Context<Self>) -> SheetTab {
        match showing {
            SheetTab::Plugin if self.plugin_tab_to_draw(cx).is_none() => SheetTab::Queue,
            showing => showing,
        }
    }

    /// Which tab is showing, remembered so the app opens on the one it was left on.
    pub(crate) fn set_tab(&mut self, tab: SheetTab, cx: &mut Context<Self>) {
        self.tab = tab;
        if self.presentation == crate::Presentation::Panel {
            let panel = match tab {
                SheetTab::Lyrics => SidePanel::Lyrics,
                SheetTab::Plugin => SidePanel::Plugin,
                _ => SidePanel::Queue,
            };
            Chrome::global(cx).update(cx, |chrome, cx| chrome.set_panel(Some(panel), cx));
        }
        store::LayoutModel::edit(cx, move |layout| layout.sheet.tab = tab);
        cx.notify();
    }

    /// The tab's body, crossfading from the last one when the tab changes.
    pub(crate) fn tab_body(
        &mut self,
        showing: SheetTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let showing = self.resolve_tab(showing, cx);
        if self.tab_drawn != Some(showing) {
            self.tab_leaving = self.tab_drawn.replace(showing);
            self.tab_fade = motion::Fade::hidden();
        }
        let shown = self.tab_fade.sample(true, motion::FAST, window, cx);
        let leaving = self
            .tab_leaving
            .filter(|leaving| shown < 1. && *leaving != showing);
        if leaving.is_none() {
            self.tab_leaving = None;
            return self.tab_view(showing);
        }
        div()
            .size_full()
            .relative()
            .children(leaving.map(|leaving| {
                div()
                    .absolute()
                    .inset_0()
                    .opacity(1. - shown)
                    .child(self.tab_view(leaving))
            }))
            .child(
                div()
                    .size_full()
                    .opacity(shown)
                    .child(self.tab_view(showing)),
            )
            .into_any_element()
    }

    fn tab_view(&self, showing: SheetTab) -> AnyElement {
        match showing {
            SheetTab::Lyrics => self
                .lyrics
                .clone()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(|| div().into_any_element()),
            SheetTab::Related => components::EmptyState::new(t("related_soon"))
                .icon(icons::COMPASS)
                .into_any_element(),
            SheetTab::Plugin => self
                .plugin_tab
                .clone()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(|| div().into_any_element()),
            SheetTab::Queue => self
                .queue
                .clone()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(|| div().into_any_element()),
        }
    }
}
