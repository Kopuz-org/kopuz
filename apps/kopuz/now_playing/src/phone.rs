//! The phone layout, taken only when `theme::mobile::is_compact`: a compact bar, and the full-screen player it opens.
//!
//! The player is the sheet's phone face: the shell already lays the sheet over the whole window
//! whenever `Chrome::is_sheet_open`, so opening and closing go through the same state as on desktop.

use components::format::t;
use components::phone::{Pan, Swipe, on_pan};
use components::{
    Cover, IconButton, IconTone, MetaLayout, NowPlayingMeta, Picture, PlayPauseButton,
    TransportMetrics, TransportOverflow, icons, lit_accent, style,
};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, px,
};
use store::layout::SheetTab;
use store::{Chrome, PlayerModel, actions};
use theme::mobile::{TOUCH_TARGET, safe_insets};

use crate::NowPlaying;
use crate::meta::{crossfade, playing_from, track_picture};
use crate::swap::SwapFrame;

/// The compact bar's height: a thumbnail and two touch targets with a little room around them.
pub(crate) const BAR_HEIGHT: Pixels = px(64.);
const BAR_COVER: Pixels = px(44.);
const BAR_GLYPH: Pixels = px(22.);
const GLYPH: Pixels = px(20.);
const CLOSE_GLYPH: Pixels = px(24.);
/// The player's side margin, inside the safe area.
const GUTTER: Pixels = px(20.);
/// What the player keeps besides the artwork: header, title, marks, seek, transport, volume and the tab row.
const CONTROLS_HEIGHT: Pixels = px(470.);
const MIN_ARTWORK: Pixels = px(120.);
const ROW_COVER: Pixels = px(56.);
/// The seek bar's hit area and knob, grown for a finger.
pub(crate) const SEEK_SCALE: f32 = 1.4;
/// The player's transport: every button at least a touch target.
const TRANSPORT: TransportMetrics = TransportMetrics {
    side: TOUCH_TARGET,
    step: px(56.),
    play: px(72.),
    side_glyph: px(20.),
    step_glyph: px(28.),
    play_glyph: px(30.),
    gap: px(12.),
};

fn set_player(open: bool, cx: &mut App) {
    Chrome::global(cx).update(cx, |chrome, cx| chrome.set_sheet(open, cx));
}

fn skip(swipe: Option<Swipe>, cx: &mut App) {
    match swipe {
        Some(Swipe::Left) => PlayerModel::global(cx).update(cx, |player, cx| player.next(cx)),
        Some(Swipe::Right) => PlayerModel::global(cx).update(cx, |player, cx| player.previous(cx)),
        _ => {}
    }
}

/// How far a sideways drag fades the bar's words: gone by this far.
const BAR_FADE: f32 = 160.;

/// Title over artist, each held to one line.
fn two_lines(track: &api::TrackInfo, muted: Hsla) -> gpui_kit::Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .child(
            div()
                .w_full()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(SharedString::from(track.title.clone())),
        )
        .child(
            div()
                .w_full()
                .truncate()
                .text_xs()
                .text_color(muted)
                .child(SharedString::from(track.artist.clone())),
        )
}

impl NowPlaying {
    /// The phone's bar: cover, title and artist, play-pause and next; a tap anywhere else opens the player.
    pub(crate) fn render_compact_bar(
        &mut self,
        scrim: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let track = self.track(cx);
        let swap = self.swap_frame(track.as_ref(), window, cx);
        let cover = self.cover_slot(BAR_COVER, track.as_ref(), &swap);
        let (playing, loaded) = {
            let player = PlayerModel::global(cx).read(cx);
            (player.is_playing(), player.track().is_some())
        };
        let muted = cx.theme().muted_foreground;
        let drag = self.bar_drag;
        let text = match track.as_ref() {
            Some(track) => two_lines(track, muted),
            None => div().flex_1().min_w_0(),
        }
        .relative()
        .left(drag)
        .opacity(1. - (f32::from(drag).abs() / BAR_FADE).min(0.8));
        let owner = cx.entity().downgrade();
        let pan = move |pan: Pan, _: &mut Window, cx: &mut App| {
            owner.update(cx, |this, cx| this.bar_pan(pan, cx)).ok();
        };

        let bar = h_flex()
            .id("bar-compact")
            .w_full()
            .h(BAR_HEIGHT)
            .flex_shrink_0()
            .px(px(12.))
            .gap(px(8.))
            .items_center()
            .border_t_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .bg(style::black(scrim, cx))
            .cursor_pointer()
            .on_click(|_, _, cx| set_player(true, cx))
            .child(cover)
            .child(text)
            .child(
                PlayPauseButton::new("bar-compact-play")
                    .playing(playing)
                    .size(TOUCH_TARGET, BAR_GLYPH)
                    .disabled(!loaded)
                    .stop_propagation(true)
                    .on_click(|_, _, cx| {
                        PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx))
                    }),
            )
            .child(
                IconButton::new("bar-compact-next", icons::SKIP_FORWARD)
                    .size(TOUCH_TARGET)
                    .glyph(BAR_GLYPH)
                    .disabled(!loaded)
                    .stop_propagation(true)
                    .tooltip(t("next_track"))
                    .on_click(|_, _, cx| {
                        PlayerModel::global(cx).update(cx, |player, cx| player.next(cx))
                    }),
            );
        on_pan(bar, pan, cx).into_any_element()
    }

    /// The bar follows a sideways drag; let go, a swipe left or right skips and one up opens the player.
    fn bar_pan(&mut self, pan: Pan, cx: &mut Context<Self>) {
        let sideways = pan.offset.x.abs() > pan.offset.y.abs();
        self.bar_drag = match (pan.is_over(), sideways) {
            (false, true) => pan.offset.x,
            _ => Pixels::ZERO,
        };
        match pan.swipe() {
            Some(Swipe::Up) => set_player(true, cx),
            swipe => skip(swipe, cx),
        }
        cx.notify();
    }

    /// The player follows a pull down and closes past a swipe; a swipe across the artwork skips.
    /// Closing keeps the pull, so the player folds away from where the finger left it.
    fn sheet_pan(&mut self, pan: Pan, skips: bool, cx: &mut Context<Self>) {
        let down = pan.offset.y > pan.offset.x.abs();
        let swipe = pan.swipe();
        self.sheet_pulling = !pan.is_over();
        self.sheet_drag = match (pan.is_over(), down) {
            (false, true) => pan.offset.y,
            (true, _) if swipe == Some(Swipe::Down) => self.sheet_drag,
            _ => Pixels::ZERO,
        };
        match swipe {
            Some(Swipe::Down) => set_player(false, cx),
            swipe if skips => skip(swipe, cx),
            _ => {}
        }
        cx.notify();
    }

    /// How far the player is pulled down: only while a finger pulls it, or while it folds away after one.
    pub(crate) fn sheet_drag(&mut self, open: bool) -> Pixels {
        if open && !self.sheet_pulling {
            self.sheet_drag = Pixels::ZERO;
        }
        self.sheet_drag
    }

    fn sheet_pan_listener(
        &self,
        skips: bool,
        cx: &mut Context<Self>,
    ) -> impl Fn(Pan, &mut Window, &mut App) + 'static {
        let owner = cx.entity().downgrade();
        move |pan, _, cx| {
            owner
                .update(cx, |this, cx| this.sheet_pan(pan, skips, cx))
                .ok();
        }
    }

    /// The full-screen player inside the sheet's surface: header, artwork or a tab, then the controls.
    pub(crate) fn render_phone_sheet(
        &mut self,
        swap: &SwapFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let insets = safe_insets(cx);
        let track = self.track(cx);
        let accent = lit_accent(window, cx);
        let muted = cx.theme().muted_foreground;

        let header = h_flex()
            .id("sheet-header")
            .w_full()
            .h(TOUCH_TARGET)
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .child(
                IconButton::new("sheet-close", icons::CHEVRON_DOWN)
                    .size(TOUCH_TARGET)
                    .glyph(CLOSE_GLYPH)
                    .tone(IconTone::Scrim)
                    .tooltip(t("close"))
                    .on_click(|_, _, cx| set_player(false, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_center()
                    .truncate()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(t("now_playing")),
            )
            .child(TransportOverflow::new(
                "transport-overflow",
                TOUCH_TARGET,
                GLYPH,
            ));
        let header = on_pan(header, self.sheet_pan_listener(false, cx), cx);

        let body = match self.phone_tab {
            None => {
                let stage = self.phone_stage(track.as_ref(), swap, window, cx);
                on_pan(
                    v_flex()
                        .id("sheet-stage")
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(stage),
                    self.sheet_pan_listener(true, cx),
                    cx,
                )
                .into_any_element()
            }
            Some(tab) => v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .gap_3()
                .children(track.as_ref().map(|track| {
                    h_flex()
                        .w_full()
                        .flex_shrink_0()
                        .items_center()
                        .gap_3()
                        .child(
                            Cover::new(track.artwork.as_ref())
                                .size(ROW_COVER)
                                .rounded(style::thumb_radius()),
                        )
                        .child(two_lines(track, muted))
                }))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(self.tab_body(tab, window, cx)),
                )
                .into_any_element(),
        };

        let volume_scale =
            f32::from(TOUCH_TARGET) / f32::from(style::chrome().round_button).max(1.);
        let controls = v_flex()
            .w_full()
            .flex_shrink_0()
            .items_center()
            .gap_3()
            .child(div().w_full().children(self.seek.clone()))
            .child(self.transport(TRANSPORT, true, cx))
            .child(self.volume_control(true, cx).scale(volume_scale))
            .child(self.phone_tabs(accent, cx));

        v_flex()
            .size_full()
            .pt(insets.top)
            .pb(insets.bottom + px(8.))
            .pl(insets.left + GUTTER)
            .pr(insets.right + GUTTER)
            .gap_3()
            .child(header)
            .child(body)
            .child(controls)
            .into_any_element()
    }

    /// The artwork as large as the screen allows, with the title, artist and the track's marks under it.
    fn phone_stage(
        &self,
        track: Option<&api::TrackInfo>,
        swap: &SwapFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewport = window.viewport_size();
        let insets = safe_insets(cx);
        let width = viewport.width - insets.left - insets.right - GUTTER * 2.;
        let height = viewport.height - insets.top - insets.bottom - CONTROLS_HEIGHT;
        let side = width.min(height).max(MIN_ARTWORK);
        let radius = style::to_px(style::radius_xl(), window);
        let artwork = track.and_then(|track| track.artwork.clone());
        let picture = track.and_then(track_picture);
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
        let cover = div()
            .relative()
            .size(side)
            .flex_shrink_0()
            .shadow_lg()
            .children(leaving)
            .child(
                div().opacity(swap.cover).child(
                    Cover::new(artwork.as_ref())
                        .size(side)
                        .rounded(radius)
                        .hq(true)
                        .spot(picture.as_ref().map(Picture::spot)),
                ),
            );

        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(cover)
            .children(track.map(|track| self.phone_title(track, swap, cx)))
            .into_any_element()
    }

    /// Title, artist and album as the sheet draws them, and the heart, share and menu at touch size.
    fn phone_title(
        &self,
        track: &api::TrackInfo,
        swap: &SwapFrame,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let origin = playing_from(cx);
        let meta = |id: &'static str, track: &api::TrackInfo| {
            NowPlayingMeta::new(id, track)
                .layout(MetaLayout::Hero)
                .origin(origin.clone())
                .on_before_navigate(crate::sheet::close_sheet)
        };
        let title = match swap.leaving.as_ref() {
            Some(old) if swap.title < 1. => {
                crossfade(meta("sheet-leaving", old), meta("sheet", track), swap.title)
                    .w_full()
                    .into_any_element()
            }
            _ => meta("sheet", track).into_any_element(),
        };
        let marks = self
            .marks("sheet", true, Some(track), cx)
            .size(TOUCH_TARGET, GLYPH)
            .tone(IconTone::Soft)
            .share(true)
            .on_share({
                let key = track.key.clone();
                move |_, cx| actions::share_track(key.clone(), cx)
            });

        v_flex()
            .w_full()
            .items_center()
            .gap_1()
            .child(title)
            .child(marks)
    }

    /// Where else it can play, and the queue and lyrics, each shown in place of the artwork until tapped again.
    fn phone_tabs(&self, accent: Hsla, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let devices = self.devices_button(accent, cx);
        let showing = self.phone_tab;
        let tab = |id: &'static str, glyph: &'static str, tip: &'static str, tab: SheetTab| {
            IconButton::new(id, glyph)
                .size(TOUCH_TARGET)
                .glyph(GLYPH)
                .lit(showing == Some(tab))
                .accent(accent)
                .tooltip(t(tip))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.phone_tab = (this.phone_tab != Some(tab)).then_some(tab);
                    cx.notify();
                }))
        };

        h_flex()
            .w_full()
            .items_center()
            .justify_around()
            .children(devices)
            .child(tab("sheet-queue", icons::LIST, "up_next", SheetTab::Queue))
            .child(tab(
                "sheet-lyrics",
                icons::LYRICS,
                "lyrics",
                SheetTab::Lyrics,
            ))
    }
}
