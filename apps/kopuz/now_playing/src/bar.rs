//! The bar across the bottom: the one presentation that is always there.

use components::format::t;
use components::{IconButton, TransportMetrics, icons, lit_accent, style};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, IntoElement, ParentElement as _, Pixels, Styled, Window, div, px,
    relative, rems,
};
use store::{Chrome, ConfigModel, SidePanel};

use crate::NowPlaying;

const UTILITY_GLYPH: Pixels = px(12.);
const TRANSPORT_WIDTH: Pixels = px(640.);
/// Material's surface-container: the text colour at 7% over the window.
const FLOATING_TINT: f32 = 0.07;

/// What the right end still affords; the title pays, so the least useful goes first as it narrows.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Affords {
    level: bool,
    share: bool,
    mini: bool,
    /// The dots beside the title; what it offers is on the block's own right-click either way.
    menu: bool,
    /// What the transport column takes, which is the rest of what the title has to live on.
    transport: f32,
}

const LEVEL_NEEDS: Pixels = px(1200.);
const SHARE_NEEDS: Pixels = px(1080.);
const MINI_NEEDS: Pixels = px(980.);
const WIDE_TRANSPORT: f32 = 0.4;
const TIGHT_TRANSPORT: f32 = 0.32;

/// What `width` affords a bar drawn at `scale`: a larger bar runs out of room sooner.
fn affords(width: Pixels, scale: f32) -> Affords {
    let width = width / scale;
    Affords {
        level: width >= LEVEL_NEEDS,
        share: width >= SHARE_NEEDS,
        mini: width >= MINI_NEEDS,
        menu: width >= MINI_NEEDS,
        transport: match width >= LEVEL_NEEDS {
            true => WIDE_TRANSPORT,
            false => TIGHT_TRANSPORT,
        },
    }
}
// `bg-black/40` when something is painted behind the bar, `bg-black/60` when nothing is.
const SCRIM_OVER_ART: f32 = 0.4;
const SCRIM_PLAIN: f32 = 0.6;

impl NowPlaying {
    pub(crate) fn render_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let scale = self.scale(cx);
        // Anything behind the window is worth seeing through the bar, including the cover's own light.
        let over_art = theme::ambient(cx).is_some() || {
            let config = ConfigModel::global(cx);
            let config = config.read(cx);
            let config = config.config();
            config.background.cover_art_background
                || !config.background.custom_background_path.is_empty()
        };
        let scrim = if over_art {
            SCRIM_OVER_ART
        } else {
            SCRIM_PLAIN
        };

        let skin = style::chrome();
        let floating = skin.pane_radius > px(0.);
        let bar = h_flex()
            .w_full()
            .h(skin.player_height * scale)
            .flex_shrink_0()
            // `px-4` and `gap-4`, at the bar's scale.
            .px(rems(scale))
            .gap(rems(scale))
            .justify_between()
            // A skin with rounded panes floats the bar as one more card: Material's surface container.
            .when(floating, |bar| {
                bar.rounded(skin.pane_radius)
                    .bg(style::white(FLOATING_TINT, cx))
            })
            .when(!floating, |bar| {
                bar.border_t_1()
                    .border_color(style::white(style::BORDER_SOFT, cx))
                    .bg(style::black(scrim, cx))
            })
            .child(self.track_meta(window, cx))
            .child(
                v_flex()
                    .w(relative(
                        affords(window.viewport_size().width, scale).transport,
                    ))
                    // Past this the seek bar sprawls and the title beside it starves.
                    .max_w(TRANSPORT_WIDTH * scale)
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .gap(rems(0.5 * scale))
                    .child(self.transport(TransportMetrics::COMPACT.scaled(scale), true, cx))
                    .children(self.seek.clone()),
            )
            .child(self.utilities(window, cx));
        match floating {
            true => div()
                .w_full()
                .flex_shrink_0()
                .py(skin.pane_gap)
                .px(skin.pane_gap * 1.5)
                .child(bar)
                .into_any_element(),
            false => bar.into_any_element(),
        }
    }

    /// The right end of the bar: level, the two panels, where else it can play, and the two sizes.
    fn utilities(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let sheet_bar = self.presentation == crate::Presentation::SheetBar;
        let panel = if sheet_bar {
            match self.tab {
                store::layout::SheetTab::Lyrics => Some(SidePanel::Lyrics),
                store::layout::SheetTab::Queue => Some(SidePanel::Queue),
                _ => None,
            }
        } else {
            Chrome::global(cx).read(cx).panel()
        };
        let wanted = store::layout(cx).player.clone();
        let scale = self.scale(cx);
        let room = affords(window.viewport_size().width, scale);
        let accent = lit_accent(window, cx);
        let queued = panel == Some(SidePanel::Queue);
        let lyrical = panel == Some(SidePanel::Lyrics);

        h_flex()
            // The same weight the meta block carries, so the transport between them is centred.
            .flex_1()
            .min_w_0()
            .gap(rems(scale))
            .justify_end()
            .when(wanted.volume, |row| {
                row.child(self.volume_control(room.level, cx))
            })
            .when(wanted.queue, |row| {
                row.child(
                    utility("bar-queue", icons::LIST, scale)
                        .lit(queued)
                        .accent(accent)
                        .tooltip(t("up_next"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.devices_open = false;
                            if this.presentation == crate::Presentation::SheetBar {
                                this.set_tab(store::layout::SheetTab::Queue, cx);
                                return;
                            }
                            Chrome::global(cx)
                                .update(cx, |chrome, cx| chrome.toggle_panel(SidePanel::Queue, cx));
                        })),
                )
            })
            .when(wanted.lyrics, |row| {
                row.child(
                    utility("bar-lyrics", icons::LYRICS, scale)
                        .lit(lyrical)
                        .accent(accent)
                        .tooltip(t("lyrics"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.devices_open = false;
                            if this.presentation == crate::Presentation::SheetBar {
                                this.set_tab(store::layout::SheetTab::Lyrics, cx);
                                return;
                            }
                            Chrome::global(cx).update(cx, |chrome, cx| {
                                chrome.toggle_panel(SidePanel::Lyrics, cx)
                            });
                        })),
                )
            })
            .children(self.devices_button(accent, cx))
            .when(room.share, |row| {
                row.child(
                    self.share_mark("bar-share", cx)
                        .size(style::chrome().round_button * scale, UTILITY_GLYPH * scale),
                )
            })
            .when(room.mini, |row| {
                row.child(
                    utility("bar-mini", icons::COLLAPSE, scale)
                        .tooltip(t("mini_player"))
                        .on_click(|_, _, cx| crate::open_mini(cx)),
                )
            })
            .child(
                utility("bar-sheet", icons::EXPAND, scale)
                    .tooltip(t("now_playing"))
                    .on_click(|_, _, cx| {
                        Chrome::global(cx).update(cx, |chrome, cx| chrome.toggle_sheet(cx))
                    }),
            )
    }
}

/// The shape every button at the right end of the bar takes.
pub(crate) fn utility(id: &'static str, glyph: &'static str, scale: f32) -> IconButton {
    IconButton::new(id, glyph)
        .size(style::chrome().round_button * scale)
        .glyph(UTILITY_GLYPH * scale)
}

/// Whether the title's own dots button still fits, which the meta block has to ask.
pub(crate) fn menu_fits(window: &Window, scale: f32) -> bool {
    affords(window.viewport_size().width, scale).menu
}

/// The shape and glyph size of one bar-slot button, for a package contributing another.
pub fn utility_metrics(cx: &App) -> (Pixels, Pixels) {
    let scale = store::layout(cx).player.size.scale();
    (style::chrome().round_button * scale, UTILITY_GLYPH * scale)
}

/// The bar owns its height; the shell does not have to guess it.
pub fn bar_height(cx: &App) -> Pixels {
    style::chrome().player_height * store::layout(cx).player.size.scale()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_window_keeps_the_whole_bar() {
        let room = affords(px(1400.), 1.);
        assert!(room.level && room.share && room.mini && room.menu);
        assert_eq!(room.transport, WIDE_TRANSPORT);
    }

    #[test]
    fn the_level_is_the_first_thing_the_title_takes_back() {
        let room = affords(px(1100.), 1.);
        assert!(!room.level);
        assert!(room.share && room.mini && room.menu);
        assert_eq!(room.transport, TIGHT_TRANSPORT);
    }

    #[test]
    fn a_narrow_window_keeps_only_what_the_bar_cannot_do_without() {
        let room = affords(px(924.), 1.);
        assert!(!room.level && !room.share && !room.mini && !room.menu);
    }

    #[test]
    fn the_transport_never_takes_more_of_a_narrow_window_than_a_wide_one() {
        assert!(affords(px(924.), 1.).transport < affords(px(1400.), 1.).transport);
    }

    #[test]
    fn a_larger_bar_gives_up_its_extras_before_crowding_the_title() {
        let large = store::layout::PlayerSize::Large.scale();
        assert!(affords(px(1200.), 1.).level);
        let room = affords(px(1200.), large);
        assert!(!room.level && !room.share && !room.mini && !room.menu);
        assert_eq!(room.transport, TIGHT_TRANSPORT);
    }
}
