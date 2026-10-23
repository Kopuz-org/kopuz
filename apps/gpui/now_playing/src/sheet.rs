//! The sheet: the bar grown to fill the page, with the cover travelling out of its bar slot.

#[cfg(test)]
#[path = "sheet_tests.rs"]
mod tests;

use components::format::t;
use components::{
    Cover, IconButton, IconTone, MetaLayout, NowPlayingMeta, Picture, TransportMetrics,
    TransportOverflow, WithContextMenu as _, icons, menus, motion, style,
};
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::{ActiveTheme as _, FocusTrapElement as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Styled, Window, div, point, px, relative, rems,
};
use store::layout::{Backdrop, Side};
use store::{Chrome, ConfigModel, actions};

use crate::NowPlaying;
use crate::meta::{cover_size, playing_from, track_picture, track_target};
use crate::swap::SwapFrame;

// The column the title, the clocks and the transport share, and the panel beside them.
// A cover never takes more of the stage's width than this, so a narrow window keeps its edges.
const HERO_WIDTH: f32 = 0.9;
const COLUMN: f32 = 40.;
const PANEL: f32 = 0.38;
const ACTION: Pixels = px(44.);
const ACTION_GLYPH: Pixels = px(18.);
const CLOSE: Pixels = px(40.);
const CLOSE_GLYPH: Pixels = px(22.);
// `bg-black/72` at the least: the sheet is read over, never a bright wallpaper.
const SHEET_DARKENING: u8 = 72;

pub(crate) struct ModalFocus {
    handle: FocusHandle,
    previous: Option<FocusHandle>,
    open: bool,
}

impl ModalFocus {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            handle: cx.focus_handle(),
            previous: None,
            open: false,
        }
    }

    pub(crate) fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut App) {
        if self.open == open {
            return;
        }
        self.open = open;
        if open {
            self.previous = window.focused(cx);
            self.handle.focus(window, cx);
        } else if let Some(previous) = self.previous.take()
            && self.handle.contains_focused(window, cx)
        {
            previous.focus(window, cx);
        }
    }
}

fn surface() -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id("sheet")
        .size_full()
        .relative()
        .occlude()
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
        .on_mouse_up(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
}

impl NowPlaying {
    pub(crate) fn render_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let layout = store::layout(cx).sheet.clone();
        let track = self.track(cx);
        let artwork = track.as_ref().and_then(|track| track.artwork.clone());
        let open = Chrome::global(cx).read(cx).is_sheet_open();
        // It grows in over the slower span and folds away over the quicker one.
        let shown = self.sheet_fade.sample(
            open,
            if open { motion::SLOW } else { motion::FAST },
            window,
            cx,
        );
        if !open && shown <= 0. {
            // One more frame lets the shell see the fade is over and take the sheet down.
            window.request_animation_frame();
        }
        let swap = self.swap.frame(
            components::cover::is_ready(artwork.as_ref(), false, cx),
            window,
            cx,
        );
        let owner = cx.entity().downgrade();
        let showing = self.tab;
        let config = ConfigModel::global(cx).read(cx).config();
        let use_bar = config.ui.fullscreen_use_player_bar;
        let bar_at_top = config.ui.player_bar_position == config::PlayerBarPosition::Top;

        let stage = self.stage(&swap, window, cx);
        let body = self.tab_body(showing, window, cx);
        let panel = v_flex()
            .h_full()
            .w(relative(PANEL))
            .flex_shrink_0()
            .border_l_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .bg(style::black(0.45, cx))
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .justify_center()
                    .px_4()
                    .py_3()
                    .child(self.tab_strip(showing, false, cx)),
            )
            .child(div().flex_1().min_h_0().w_full().child(body));

        let sheet = surface()
            .key_context("NowPlaying")
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "space" && this.sheet_focus.handle.is_focused(window)
                    {
                        store::PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx));
                        cx.stop_propagation();
                    }
                }),
            )
            .overflow_hidden()
            .opacity(shown)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_prepaint(move |bounds, _, cx| {
                owner
                    .update(cx, |this, cx| {
                        if this.root != Some(bounds) {
                            this.root = Some(bounds);
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .children(self.backdrop(layout.backdrop, artwork.as_ref(), window, cx))
            .child(
                v_flex()
                    .size_full()
                    .when(use_bar && bar_at_top, |body| {
                        body.children(self.sheet_bar.clone())
                    })
                    .child(
                        h_flex().flex_1().min_h_0().w_full().items_stretch().map(
                            |row| match layout.side {
                                Side::Left => row.child(panel).child(stage),
                                Side::Right => row.child(stage).child(panel),
                            },
                        ),
                    )
                    .when(use_bar && !bar_at_top, |body| {
                        body.children(self.sheet_bar.clone())
                    }),
            );
        let flying = self.flying_cover(&swap, open, window, cx);
        // The cover is outside the fade, so it travels back to the bar whole while the sheet goes.
        div()
            .size_full()
            .relative()
            .map(|layer| match open {
                true => {
                    layer.child(sheet.focus_trap("now-playing-focus", &self.sheet_focus.handle))
                }
                false => layer.child(sheet),
            })
            .children(flying)
            .into_any_element()
    }

    /// The left half: the cover's landing place, the title, the transport and the clocks.
    fn stage(&self, swap: &SwapFrame, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let use_bar = ConfigModel::global(cx)
            .read(cx)
            .config()
            .ui
            .fullscreen_use_player_bar;
        let track = self.track(cx);
        let owner = cx.entity().downgrade();
        let hero = store::layout(cx).sheet.cover;

        v_flex()
            .relative()
            .h_full()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_center()
            .gap_6()
            .p_10()
            .child(
                div().absolute().top_6().left_6().child(
                    IconButton::new("sheet-close", icons::CHEVRON_DOWN)
                        .size(CLOSE)
                        .glyph(CLOSE_GLYPH)
                        // Over a bright cover a bare glyph disappears, so the button carries a scrim of its own.
                        .tone(IconTone::Scrim)
                        .tooltip(t("close"))
                        .on_click(|_, _, cx| {
                            Chrome::global(cx).update(cx, |chrome, cx| chrome.set_sheet(false, cx))
                        }),
                ),
            )
            .child(
                // The hero cover is drawn by the flying layer; this only says where it lands.
                div()
                    .id("sheet-hero-slot")
                    .flex_shrink_0()
                    .size(px(hero.side()))
                    .max_h(relative(hero.max_height()))
                    .max_w(relative(HERO_WIDTH))
                    .on_prepaint(move |bounds, _, cx| {
                        owner
                            .update(cx, |this, cx| {
                                // A closing sheet's cover is headed for the bar, not back to this slot.
                                if this.sheet_focus.open && this.hero != Some(bounds) {
                                    this.hero = Some(bounds);
                                    cx.notify();
                                }
                            })
                            .ok();
                    }),
            )
            .children(track.map(|track| self.title_block(&track, swap, window, cx)))
            .when(!use_bar, |stage| {
                stage.child(
                    v_flex()
                        .w_full()
                        .max_w(rems(COLUMN))
                        .items_center()
                        .gap_4()
                        .children(self.seek.clone())
                        .child(
                            h_flex()
                                .items_center()
                                .gap_4()
                                .child(div().w(ACTION).flex_shrink_0())
                                .child(self.transport(TransportMetrics::HERO, true, cx))
                                .child(TransportOverflow::new(
                                    "transport-overflow",
                                    ACTION,
                                    ACTION_GLYPH,
                                )),
                        )
                        .child(self.volume_control(true, cx)),
                )
            })
            .into_any_element()
    }

    /// Title, artist, album and where the queue came from, with the track's own marks beside them.
    fn title_block(
        &self,
        track: &api::TrackInfo,
        swap: &SwapFrame,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let marks = self
            .marks("sheet", true, Some(track), cx)
            .size(ACTION, ACTION_GLYPH)
            .tone(IconTone::Soft)
            .share(true)
            .on_share({
                let key = track.key.clone();
                move |_, cx| actions::share_track(key.clone(), cx)
            });

        v_flex()
            .w_full()
            .max_w(rems(COLUMN))
            .items_center()
            .gap_1()
            .child({
                let origin = playing_from(cx);
                let meta = |id: &'static str, track: &api::TrackInfo| {
                    NowPlayingMeta::new(id, track)
                        .layout(MetaLayout::Hero)
                        .origin(origin.clone())
                        .on_before_navigate(close_sheet)
                };
                match swap.leaving.as_ref() {
                    Some(old) if swap.title < 1. => crate::meta::crossfade(
                        meta("sheet-leaving", old),
                        meta("sheet", track),
                        swap.title,
                    )
                    .w_full()
                    .into_any_element(),
                    _ => meta("sheet", track).into_any_element(),
                }
            })
            .child(div().pt_2().child(marks))
    }

    /// The cover itself, drawn once, travelling between the bar's slot and the hero's.
    fn flying_cover(
        &self,
        swap: &SwapFrame,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let root = self.root?;
        let track = self.track(cx);
        let artwork = track.as_ref().and_then(|track| track.artwork.clone());
        let picture = track.as_ref().and_then(track_picture);
        let menu = track
            .as_ref()
            .map(|track| menus::menu_for(track_target(track)));
        let bar = crate::cover_slot(cx);
        // Before the hero has been laid out there is nowhere to fly to, so it starts in the bar.
        let (origin, size) = match self.hero {
            Some(hero) => (hero.origin, hero.size.width.min(hero.size.height)),
            None => match bar {
                Some(bar) => (bar.origin, bar.size.width),
                None => return None,
            },
        };
        let local = point(origin.x - root.origin.x, origin.y - root.origin.y);
        // Back to the bar as quickly as the sheet fades, out of it as slowly as the sheet grows.
        let travel = if open { motion::SLOW } else { motion::FAST };
        let left = motion::length("sheet-cover-x", local.x, travel, window, cx);
        let top = motion::length("sheet-cover-y", local.y, travel, window, cx);
        let side = motion::length("sheet-cover-size", size, travel, window, cx);
        let radius = style::to_px(style::radius_xl(), window)
            * (f32::from(side)
                / f32::from(cover_size(store::layout(cx).player.size.scale(), cx)).max(1.))
            .clamp(0.4, 1.);

        let leaving = swap
            .leaving
            .as_ref()
            .filter(|_| swap.cover < 1.)
            .map(|old| {
                div().absolute().top_0().left_0().child(
                    Cover::new(old.artwork.as_ref())
                        .size(side)
                        .rounded(radius)
                        .hq(true),
                )
            });
        Some(
            div()
                .id("sheet-cover")
                .absolute()
                .left(left)
                .top(top)
                .size(side)
                .shadow_lg()
                .children(leaving)
                .child(
                    div()
                        .relative()
                        .opacity(swap.cover)
                        // The bar's thumbnail is already decoded, so it holds the place while the full picture fades in.
                        .child(
                            div().absolute().top_0().left_0().child(
                                Cover::new(artwork.as_ref())
                                    .size(side)
                                    .hq(false)
                                    .rounded(radius),
                            ),
                        )
                        .child(
                            Cover::new(artwork.as_ref())
                                .size(side)
                                .rounded(radius)
                                .hq(true)
                                .spot(picture.as_ref().map(Picture::spot)),
                        ),
                )
                .with_context_menu(menus::pictured(menu, picture)),
        )
    }

    /// What is behind the sheet: the cover itself, the colour taken from it, or nothing at all.
    fn backdrop(
        &self,
        backdrop: Backdrop,
        artwork: Option<&api::ArtworkRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match backdrop {
            Backdrop::Solid => None,
            Backdrop::Palette => {
                let wash = theme::wash_for(theme::Surface::Sheet, window, cx)?;
                let tinted = motion::color_over(
                    "sheet-wash",
                    wash.opacity(0.35),
                    theme::ACCENT_TWEEN,
                    window,
                    cx,
                );
                Some(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(cx.theme().background)
                        .child(div().absolute().top_0().left_0().size_full().bg(tinted))
                        .into_any_element(),
                )
            }
            Backdrop::Artwork => components::backdrop::layer(artwork, SHEET_DARKENING, window, cx),
        }
    }
}

/// A link out of the sheet folds it away first, so the page it opens is not hidden behind it.
fn close_sheet(_: &mut Window, cx: &mut App) {
    Chrome::global(cx).update(cx, |chrome, cx| chrome.set_sheet(false, cx));
}
