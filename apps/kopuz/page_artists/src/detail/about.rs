//! The About view: what the source says of the artist, its sound, and artists like it.

use api::{ArtistInfoView, SimilarArtist};
use components::format::t;
use components::{
    Badge, BadgeTone, Chip, ChipSize, ChipTone, Heading, SectionHeading, sound_profile, style,
};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, div,
};
use store::{Navigator, Route};

use super::{ArtistDetail, capabilities};

/// Lines of biography shown before "Show more".
const CLAMPED_LINES: usize = 4;

/// A biography this short fits its clamp, so it offers no "Show more".
const CLAMP_FITS_CHARS: usize = 320;

impl ArtistDetail {
    /// What the source says of the artist, what AudioMuse-AI hears, and one row of similar
    /// artists; `None` when there is nothing to show.
    pub(super) fn about(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sound = self.sound_like(cx);
        let info = match capabilities(cx).artist_info {
            true => self.info.read(cx).value().cloned().flatten(),
            false => None,
        };
        // ListenBrainz, when its switch is on and it found any, else the source's own list.
        let similar = self
            .similar
            .as_deref()
            .filter(|found| !found.is_empty())
            .or(info.as_ref().map(|info| info.similar.as_slice()))
            .and_then(similar_artists);
        let biography = info.map(|info| self.biography(&info, cx));
        if biography.is_none() && sound.is_none() && similar.is_none() {
            return None;
        }
        Some(
            v_flex()
                .w_full()
                .gap_4()
                .when_some(biography, |about, biography| {
                    about
                        .child(SectionHeading::new(t("artist_about")).heading(Heading::Group))
                        .child(biography)
                })
                .children(sound)
                .children(similar)
                .into_any_element(),
        )
    }

    /// The artist's tags and moods over the library artists that sound like them; nothing while loading or empty.
    fn sound_like(&self, cx: &App) -> Option<AnyElement> {
        if !self.sound_on {
            return None;
        }
        let sound = self.sound.read(cx).value()?;
        let profile = sound_profile(sound);
        let chips: Vec<Chip> = sound
            .similar_artists
            .iter()
            .filter_map(|artist| {
                let key = artist.key.clone()?;
                Some(
                    Chip::new(SharedString::from(format!("sound-{key}")))
                        .label(SharedString::from(artist.name.clone()))
                        .size(ChipSize::Medium)
                        .on_click(move |_, _, cx| {
                            Navigator::navigate(Route::Artist(key.clone()), cx)
                        }),
                )
            })
            .collect();
        if profile.is_none() && chips.is_empty() {
            return None;
        }
        let artists = (!chips.is_empty()).then(|| {
            v_flex()
                .w_full()
                .gap_2()
                .child(SectionHeading::new(t("sounds_like")).heading(Heading::Group))
                .child(h_flex().w_full().flex_wrap().gap_2().children(chips))
        });
        Some(
            v_flex()
                .w_full()
                .gap_4()
                .children(profile)
                .children(artists)
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

        // No picture beside the text: the header above already shows the artist.
        v_flex()
            .w_full()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .text_sm()
                    .text_color(ink)
                    .when(!open, |text| text.line_clamp(CLAMPED_LINES))
                    .child(SharedString::from(text)),
            )
            .child(h_flex().gap_2().children(toggle).children(link))
            .into_any_element()
    }
}

/// A chip to open each similar artist the library holds, plain text for the others.
fn similar_artists(similar: &[SimilarArtist]) -> Option<AnyElement> {
    if similar.is_empty() {
        return None;
    }
    let chips = similar.iter().map(|artist| {
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
            .child(SectionHeading::new(t("similar_artists")).heading(Heading::Group))
            .child(h_flex().w_full().flex_wrap().gap_2().children(chips))
            .into_any_element(),
    )
}
