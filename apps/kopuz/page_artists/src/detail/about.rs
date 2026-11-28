//! The About section: what the source says of the artist, and the artists it calls similar.

use api::ArtistInfoView;
use components::format::t;
use components::{
    Badge, BadgeTone, Chip, ChipSize, ChipTone, Cover, Heading, SectionHeading, style,
};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};
use store::{Navigator, Route};

use super::{ArtistDetail, capabilities};

/// Lines of biography shown before "Show more".
const CLAMPED_LINES: usize = 4;

/// A biography this short fits its clamp, so it offers no "Show more".
const CLAMP_FITS_CHARS: usize = 320;

const PICTURE: gpui_kit::Pixels = px(96.);

impl ArtistDetail {
    /// `None` while the source has nothing to say, or has no artist info at all.
    pub(super) fn about(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !capabilities(cx).artist_info {
            return None;
        }
        let info = self.info.read(cx).value()?.clone()?;
        let body = self.biography(&info, cx);
        Some(
            v_flex()
                .w_full()
                .gap_4()
                .child(SectionHeading::new(t("artist_about")).heading(Heading::Group))
                .child(body)
                .children(similar_artists(&info))
                .into_any_element(),
        )
    }

    fn biography(&mut self, info: &ArtistInfoView, cx: &mut Context<Self>) -> AnyElement {
        let text = info.biography.clone().unwrap_or_default();
        let long = text.chars().count() > CLAMP_FITS_CHARS;
        let open = self.about_open;
        let ink = style::white(style::TEXT_SECONDARY, cx);

        let toggle = long.then(|| {
            let label = match open {
                true => t("discover_show_less"),
                false => t("discover_show_more"),
            };
            Chip::new("artist-about-toggle")
                .label(label)
                .tone(ChipTone::Filter)
                .size(ChipSize::Medium)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.about_open = !this.about_open;
                    cx.notify();
                }))
        });
        let link = info.link.clone().map(|url| {
            Chip::new("artist-about-link")
                .label(t("artist_info_link"))
                .tone(ChipTone::Filter)
                .size(ChipSize::Medium)
                .on_click(move |_, _, cx| cx.open_url(&url))
        });

        h_flex()
            .w_full()
            .gap_4()
            .items_start()
            .when(info.image.is_some(), |row| {
                row.child(
                    Cover::new(info.image.as_ref())
                        .size(PICTURE)
                        .rounded(px(8.))
                        .placeholder(components::icons::MIC),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_3()
                    .child(
                        div()
                            .w_full()
                            .text_sm()
                            .text_color(ink)
                            .when(!open, |text| text.line_clamp(CLAMPED_LINES))
                            .child(SharedString::from(text)),
                    )
                    .child(h_flex().gap_2().children(toggle).children(link)),
            )
            .into_any_element()
    }
}

/// A chip to open each similar artist the library holds, plain text for the others.
fn similar_artists(info: &ArtistInfoView) -> Option<AnyElement> {
    if info.similar.is_empty() {
        return None;
    }
    let chips = info.similar.iter().map(|artist| {
        let name = SharedString::from(artist.name.clone());
        match artist.key.clone() {
            Some(key) => Chip::new(SharedString::from(format!("similar-{key}")))
                .label(name)
                .size(ChipSize::Medium)
                .on_click(move |_, _, cx| Navigator::navigate(Route::Artist(key.clone()), cx))
                .into_any_element(),
            None => Badge::new(name)
                .tone(BadgeTone::Muted)
                .size(ChipSize::Medium)
                .into_any_element(),
        }
    });
    Some(
        v_flex()
            .w_full()
            .gap_2()
            .child(SectionHeading::new(t("artist_similar")).heading(Heading::Group))
            .child(h_flex().w_full().flex_wrap().gap_2().children(chips))
            .into_any_element(),
    )
}
