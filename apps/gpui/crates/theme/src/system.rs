//! System colors: Material colour roles built from the playing cover, as the Dioxus app builds them on macOS.

use material_colors::color::Argb;
use material_colors::dynamic_color::DynamicScheme;
use material_colors::hct::Hct;
use material_colors::scheme::Scheme;
use material_colors::scheme::variant::{SchemeContent, SchemeMonochrome, SchemeTonalSpot};
use store::palette::Swatches;

use crate::Colors;

/// The theme id `Ui::theme` stores for it.
pub const SYSTEM: &str = "system";

/// The default theme's copper, for when nothing is playing.
const DEFAULT_SEED: Argb = Argb::from_u32(0xffd9842f);

/// Below this HCT chroma a colour is grey, and its hue is noise.
const MIN_SEED_CHROMA: f64 = 15.0;

/// The roles for a cover's swatches, or the default seed's with none.
pub fn system_colors(swatches: Option<Swatches>, dark: bool) -> Colors {
    let scheme = match swatches {
        None => tonal_spot(DEFAULT_SEED, dark),
        Some(swatches) => {
            let vibrant = argb(swatches.vibrant);
            let dominant = argb(swatches.dominant);
            // Cover art is content colour: its own chroma is kept, and grey art invents no hue.
            match [vibrant, dominant]
                .into_iter()
                .find(|color| chroma(*color) >= MIN_SEED_CHROMA)
            {
                Some(seed) => SchemeContent::new(Hct::new(seed), dark, Some(0.0)).scheme,
                None => SchemeMonochrome::new(Hct::new(dominant), dark, Some(0.0)).scheme,
            }
        }
    };
    colors(&Scheme::from(scheme), dark)
}

fn tonal_spot(seed: Argb, dark: bool) -> DynamicScheme {
    SchemeTonalSpot::new(Hct::new(seed), dark, Some(0.0)).scheme
}

fn chroma(color: Argb) -> f64 {
    Hct::new(color).get_chroma()
}

fn argb(color: gpui_kit::Hsla) -> Argb {
    let rgba = gpui_kit::Rgba::from(color);
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    Argb::new(255, channel(rgba.r), channel(rgba.g), channel(rgba.b))
}

/// How far, in HCT tone, a primary button moves on hover and on press.
const HOVER_TONE: f64 = 5.0;
const PRESS_TONE: f64 = 10.0;

/// `color` moved `by` tones away from the background, which is also away from the button's label.
fn toned(color: Argb, by: f64, dark: bool) -> Argb {
    let mut hct = Hct::new(color);
    let tone = match dark {
        true => hct.get_tone() + by,
        false => hct.get_tone() - by,
    };
    hct.set_tone(tone.clamp(0., 100.));
    Argb::from(hct)
}

fn hex(color: Argb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue)
}

/// The Dioxus `THEME_VARS` mapping, from its `--color-*` names onto the palette tokens they define.
fn colors(roles: &Scheme, dark: bool) -> Colors {
    Colors {
        dark,
        bg: hex(roles.surface),
        raised: hex(roles.surface_container),
        surface: hex(roles.on_surface_variant),
        text: hex(roles.on_surface),
        text_muted: hex(roles.on_surface_variant),
        accent: hex(roles.primary),
        accent_soft: hex(toned(roles.primary, HOVER_TONE, dark)),
        accent_alt: hex(toned(roles.primary, PRESS_TONE, dark)),
        accent_deep: hex(roles.primary_container),
        highlight: hex(roles.tertiary),
        highlight_dark: hex(roles.tertiary_container),
        progress: hex(roles.primary),
        danger: hex(roles.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swatches(vibrant: u32, dominant: u32) -> Swatches {
        let hsla = |value: u32| gpui_kit::Hsla::from(gpui_kit::rgb(value));
        Swatches {
            vibrant: hsla(vibrant),
            dominant: hsla(dominant),
        }
    }

    fn parse(color: &str) -> gpui_kit::Hsla {
        gpui_kit::Rgba::try_from(color)
            .expect("a hex colour")
            .into()
    }

    #[test]
    fn text_stays_readable_on_every_scheme_in_both_appearances() {
        let covers = [
            None,
            Some(swatches(0xc81e28, 0x080808)),
            Some(swatches(0x5a5c5f, 0xf0f0f0)),
            Some(swatches(0x006aff, 0x101830)),
        ];
        for cover in covers {
            for dark in [false, true] {
                let colors = system_colors(cover, dark);
                for ground in [&colors.bg, &colors.raised] {
                    let ratio = crate::contrast(parse(&colors.text), parse(ground));
                    assert!(ratio >= crate::AA, "{cover:?} dark={dark}: {ratio:.2}");
                }
            }
        }
    }

    #[test]
    fn a_primary_button_moves_on_hover_and_further_on_press_keeping_its_label() {
        for dark in [false, true] {
            let colors = system_colors(Some(swatches(0xc81e28, 0x080808)), dark);
            assert_ne!(colors.accent_soft, colors.accent, "dark={dark}");
            assert_ne!(colors.accent_alt, colors.accent_soft, "dark={dark}");
            let label = parse(&colors.bg);
            let on = |fill: &str| crate::contrast(label, parse(fill));
            assert!(on(&colors.accent_soft) >= on(&colors.accent), "dark={dark}");
            assert!(
                on(&colors.accent_alt) >= on(&colors.accent_soft),
                "dark={dark}"
            );
        }
    }

    #[test]
    fn a_mix_runs_from_one_scheme_to_the_other() {
        let from = system_colors(Some(swatches(0xc81e28, 0x080808)), true);
        let to = system_colors(Some(swatches(0x006aff, 0x101830)), true);
        assert_eq!(crate::mix(&from, &to, 0.), from);
        assert_eq!(crate::mix(&from, &to, 1.), to);
        let half = crate::mix(&from, &to, 0.5);
        assert_ne!(half.accent, from.accent);
        assert_ne!(half.accent, to.accent);
    }

    #[test]
    fn grey_art_gets_no_invented_hue() {
        let colors = system_colors(Some(swatches(0x5a5c5f, 0xf0f0f0)), true);
        assert!(parse(&colors.accent).s < 0.1, "{}", colors.accent);
    }

    #[test]
    fn a_red_cover_leads_with_red() {
        let colors = system_colors(Some(swatches(0xc81e28, 0x080808)), true);
        let hue = parse(&colors.accent).h * 360.;
        assert!(!(30.0..=330.0).contains(&hue), "{hue}");
    }
}
