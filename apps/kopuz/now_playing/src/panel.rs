//! The docked column beside the content: the same queue and lyrics the sheet draws, in a narrower slot.

use components::format::t;
use components::{DockPanel, Edge, Segment, Segmented, icons, motion};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Pixels, Styled as _, Window, div, px,
};
use store::layout::SheetTab;
use store::{Chrome, SidePanel};

use crate::NowPlaying;

// The Dioxus rightbar's drag bounds, and the width the shell's slot starts at.
const MIN_WIDTH: Pixels = px(280.);
const MAX_WIDTH: Pixels = px(600.);
const DEFAULT_WIDTH: Pixels = px(320.);

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
            // A closing column keeps what it showed while the shell fades it out.
            _ => match self.tab_drawn {
                Some(tab) => tab,
                None => return div().into_any_element(),
            },
        };
        let panel = cx.entity().downgrade();
        let header = self.tab_strip(showing, false, cx);
        let body = self.tab_body(showing, window, cx);

        DockPanel::new("panel", Edge::Left)
            .width(self.panel_width())
            .header(header)
            .body(body)
            .on_close(|_, cx| {
                Chrome::global(cx).update(cx, |chrome, cx| chrome.set_panel(None, cx))
            })
            // The left edge drags the column between 280 and 600 px.
            .resizable(
                self.panel_width(),
                MIN_WIDTH,
                MAX_WIDTH,
                move |width, _, cx| {
                    panel
                        .update(cx, |this, cx| {
                            if this.panel_width != Some(width) {
                                this.panel_width = Some(width);
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
        let mut tabs = vec![(SheetTab::Queue, "up_next"), (SheetTab::Lyrics, "lyrics")];
        if with_related {
            tabs.push((SheetTab::Related, "related"));
        }
        let page = cx.entity().downgrade();
        Segmented::new("now-playing-tabs")
            .segments(tabs.iter().map(|(_, key)| Segment::new(*key).label(t(key))))
            .selected(tabs.iter().position(|(tab, _)| *tab == showing))
            .on_select(move |index, _, cx| {
                let Some((tab, _)) = tabs.get(*index).copied() else {
                    return;
                };
                page.update(cx, |this, cx| this.set_tab(tab, cx)).ok();
            })
    }

    /// Which tab is showing, remembered so the app opens on the one it was left on.
    pub(crate) fn set_tab(&mut self, tab: SheetTab, cx: &mut Context<Self>) {
        self.tab = tab;
        if self.presentation == crate::Presentation::Panel {
            let panel = match tab {
                SheetTab::Lyrics => SidePanel::Lyrics,
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
            SheetTab::Queue => self
                .queue
                .clone()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(|| div().into_any_element()),
        }
    }
}
