//! The mini surface: cover, title, transport with shuffle and the heart, over the blurred cover, in ~380×148.

use components::format::t;
use components::{
    Cover, IconButton, IconTone, MetaLayout, NowPlayingMeta, TransportMetrics, icons, style,
};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Rgba, Styled, Window, div, px,
};
use store::PlayerModel;

use crate::NowPlaying;

const COVER: Pixels = px(64.);
const RESTORE: Pixels = px(28.);
const RESTORE_GLYPH: Pixels = px(12.);
// The least the cover behind it is darkened, which a light cover raises until the artist line reads.
const WASH_DARKENING: u8 = 45;
const WASH_MOST: u8 = 90;
const WASH_BLUR: u8 = 40;
// The artist line's alpha in `MetaLayout::Mini`, the faintest text over the cover.
const ARTIST_ALPHA: f32 = 0.55;
const GROUP: &str = "mini-player";

/// How far to darken `artwork` for the artist line to clear AA over it; a palette still being read keeps the least.
fn wash_darkening(artwork: &api::ArtworkRef, window: &mut Window, cx: &mut App) -> u8 {
    let Some(cover) = theme::swatches(Some(artwork), window, cx).map(|swatches| swatches.dominant)
    else {
        return WASH_DARKENING;
    };
    darkening_for(cover, cx.theme().background, style::white(ARTIST_ALPHA, cx))
}

/// The least darkening of `cover` toward `ground` that leaves `ink` at AA over the result.
fn darkening_for(cover: Hsla, ground: Hsla, ink: Hsla) -> u8 {
    (WASH_DARKENING..=WASH_MOST)
        .step_by(5)
        .find(|darkening| {
            let behind = over(
                cover.opacity(1.),
                ground.opacity(f32::from(*darkening) / 100.),
            );
            theme::contrast(over(behind, ink), behind) >= theme::AA
        })
        .unwrap_or(WASH_MOST)
}

/// `top` laid over `under` at its own alpha, as the window composites them.
fn over(under: Hsla, top: Hsla) -> Hsla {
    let (under, top) = (Rgba::from(under), Rgba::from(top));
    let at = |under: f32, top_channel: f32| under + (top_channel - under) * top.a;
    Rgba {
        r: at(under.r, top.r),
        g: at(under.g, top.g),
        b: at(under.b, top.b),
        a: 1.,
    }
    .into()
}

impl NowPlaying {
    pub(crate) fn render_mini(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let track = self.track(cx);
        let artwork = track.as_ref().and_then(|track| track.artwork.clone());
        let swap = self.swap.frame(
            components::cover::is_ready(artwork.as_ref(), false, cx),
            window,
            cx,
        );
        let cover = |artwork: Option<&api::ArtworkRef>| {
            Cover::new(artwork)
                .size(COVER)
                .rounded(style::cover_radius())
        };
        let leaving_cover = swap
            .leaving
            .as_ref()
            .filter(|_| swap.cover < 1.)
            .map(|old| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .child(cover(old.artwork.as_ref()))
            });
        // The clock is one view, so only the arriving title carries it.
        let meta = |id: &'static str, track: &api::TrackInfo, clock: bool| {
            NowPlayingMeta::new(id, track)
                .layout(MetaLayout::Mini)
                .when(clock, |meta| {
                    meta.footer(div().children(self.clock.clone()))
                })
        };
        let title = match (track.as_ref(), swap.leaving.as_ref()) {
            (Some(track), Some(old)) if swap.title < 1. => crate::meta::crossfade(
                meta("mini-leaving", old, false),
                meta("mini", track, true),
                swap.title,
            )
            .flex_1()
            .min_w_0()
            .into_any_element(),
            (Some(track), _) => meta("mini", track, true).into_any_element(),
            (None, _) => div().flex_1().into_any_element(),
        };

        let wash = artwork.as_ref().map(|artwork| {
            let darkening = wash_darkening(artwork, window, cx);
            components::backdrop::wallpaper(
                store::artwork_source(artwork, true),
                darkening,
                WASH_BLUR,
                window,
                cx,
            )
        });
        let metrics = TransportMetrics::COMPACT;
        let shuffle = PlayerModel::global(cx).read(cx).shuffle();
        // Shuffle and the heart flank the transport at the same size, so play stays centred.
        let heart = self
            .marks("mini-marks", false, track.as_ref(), cx)
            .dont_recommend(false)
            .size(metrics.side, metrics.side_glyph);

        v_flex()
            .group(GROUP)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .children(wash)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .pt_3()
                    .child(
                        div()
                            .relative()
                            .flex_shrink_0()
                            .children(leaving_cover)
                            .child(div().opacity(swap.cover).child(cover(artwork.as_ref()))),
                    )
                    .child(title),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .gap(metrics.gap)
                    .px_3()
                    .pb_1()
                    .child(
                        self.transport(metrics, true, cx)
                            .shuffle(Some(shuffle))
                            .repeat(None)
                            .balanced(false)
                            .filled(true),
                    )
                    .child(heart),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .pb_2()
                    .children(self.seek.clone()),
            )
            .child(
                // Transparent rather than hidden, so it stays a tab stop and focus reveals it as hover does.
                div()
                    .absolute()
                    .top_2()
                    .right_2()
                    .track_focus(&self.restore_focus)
                    .opacity(0.)
                    .group_hover(GROUP, |restore| restore.opacity(1.))
                    .in_focus(|restore| restore.opacity(1.))
                    .child(
                        IconButton::new("mini-restore", icons::EXPAND)
                            .size(RESTORE)
                            .glyph(RESTORE_GLYPH)
                            .tone(IconTone::Scrim)
                            .tooltip(t("restore_full_player"))
                            .on_click(|_, _, cx| crate::close_mini(cx)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::hsla;

    use super::*;

    const NIGHT: Hsla = Hsla {
        h: 0.,
        s: 0.,
        l: 0.,
        a: 1.,
    };
    const WHITE: Hsla = Hsla {
        h: 0.,
        s: 0.,
        l: 1.,
        a: 1.,
    };

    #[test]
    fn a_dark_cover_keeps_the_least_darkening() {
        let ink = WHITE.opacity(ARTIST_ALPHA);
        assert_eq!(
            darkening_for(hsla(0.7, 0.4, 0.15, 1.), NIGHT, ink),
            WASH_DARKENING
        );
    }

    #[test]
    fn a_white_cover_is_darkened_until_the_artist_line_reads() {
        let ink = WHITE.opacity(ARTIST_ALPHA);
        let darkening = darkening_for(WHITE, NIGHT, ink);
        assert!(darkening > WASH_DARKENING, "{darkening}");
        let behind = over(WHITE, NIGHT.opacity(f32::from(darkening) / 100.));
        assert!(theme::contrast(over(behind, ink), behind) >= theme::AA);
    }
}
