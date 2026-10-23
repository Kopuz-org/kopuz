//! The accent the playing cover lends the chrome, once the *Accent from artwork* preference has had its say.

use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Hsla, Window};
use store::ambient::{Ground, Spread};
use store::layout::{Accent, Ambient};
use store::{PlayerModel, Swatches, api::ArtworkRef};

/// How long the accent takes to travel on a track change.
pub const ACCENT_TWEEN: Duration = Duration::from_millis(600);

/// What a swatch is saturated to before chrome wears it; extraction favours mid-lightness over colour.
const WORN_SATURATION: f32 = 0.62;
/// Below this the cover has no colour of its own, and saturating it would invent one.
const WORN_GREY: f32 = 0.3;

/// Where an accent is being asked for, which is what *sheet only* discriminates on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The now-playing sheet and anything that opens with it.
    Sheet,
    /// The bar, the seek fill, the playing row: everything that is always on screen.
    Chrome,
}

impl Surface {
    /// A stable name, for the motion channel a tween on this surface is keyed by.
    pub fn key(self) -> &'static str {
        match self {
            Surface::Sheet => "sheet",
            Surface::Chrome => "chrome",
        }
    }

    fn allowed(self, accent: Accent) -> bool {
        match accent {
            Accent::On => true,
            Accent::SheetOnly => self == Surface::Sheet,
            Accent::Off => false,
        }
    }
}

/// One cover's colours, with no regard for the preference; extraction starts on the first call.
pub fn swatches(
    artwork: Option<&ArtworkRef>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Swatches> {
    store::palette::of(artwork?, window, cx)
}

/// The accent a given cover lends, with no regard for the preference.
pub fn accent_from(
    artwork: Option<&ArtworkRef>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Hsla> {
    let vibrant = swatches(artwork, window, cx)?.vibrant;
    Some(worn(vibrant, cx.theme().background))
}

/// A swatch as chrome wears it: colourful enough to read at three pixels, and never below AA.
fn worn(color: Hsla, background: Hsla) -> Hsla {
    let saturated = match color.s >= WORN_GREY {
        true => Hsla {
            s: color.s.max(WORN_SATURATION),
            ..color
        },
        false => color,
    };
    crate::readable_on(saturated, background)
}

/// The accent `surface` should be wearing: the playing cover's, or `None` to stay on the theme's.
pub fn accent_for(surface: Surface, window: &mut Window, cx: &mut App) -> Option<Hsla> {
    playing(surface, cx).and_then(|artwork| accent_from(Some(&artwork), window, cx))
}

/// The wash behind `surface`: the playing cover's bulk colour rather than its liveliest.
pub fn wash_for(surface: Surface, window: &mut Window, cx: &mut App) -> Option<Hsla> {
    let artwork = playing(surface, cx)?;
    swatches(Some(&artwork), window, cx).map(|swatches| swatches.dominant)
}

/// The cover that lights the window and how far, or `None` where a preference keeps it the theme's.
pub fn ambient(cx: &App) -> Option<(ArtworkRef, Spread)> {
    let spread = match store::layout(cx).ambient {
        Ambient::Off => return None,
        Ambient::Glow => Spread::Glow,
        Ambient::Full => Spread::Full,
    };
    playing(Surface::Chrome, cx).map(|artwork| (artwork, spread))
}

/// What that light falls on: the theme's background, and the faintest ink it has to leave readable.
pub fn ground(cx: &App) -> Ground {
    let bytes = |colour: Hsla| {
        let colour: gpui_kit::Rgba = colour.into();
        [colour.r, colour.g, colour.b].map(|channel| (channel * 255.).round() as u8)
    };
    // Mid-ease the field is rendered once, for where the theme lands, not again for every frame on the way.
    let settled = crate::settled(cx);
    let or = |hex: &str, showing: Hsla| crate::parse(hex).unwrap_or(showing);
    Ground {
        background: bytes(or(&settled.bg, cx.theme().background)),
        ink: bytes(or(&settled.text_muted, cx.theme().muted_foreground)),
        dark: settled.dark,
    }
}

fn playing(surface: Surface, cx: &App) -> Option<ArtworkRef> {
    if !surface.allowed(store::layout(cx).accent) {
        return None;
    }
    PlayerModel::global(cx)
        .read(cx)
        .track()
        .and_then(|track| track.artwork.clone())
}

#[cfg(test)]
mod tests {
    use gpui_kit::hsla;

    use super::*;

    /// The steel blue a real cover extracted to, which read as dim and cleared only 4.12 on black.
    fn measured() -> Hsla {
        hsla(210. / 360., 0.386, 0.447, 1.)
    }

    #[test]
    fn a_muted_swatch_is_worn_louder_than_it_was_extracted() {
        let background = hsla(0., 0., 0., 1.);
        let accent = worn(measured(), background);
        assert!(accent.s >= WORN_SATURATION, "{} is still muted", accent.s);
        assert!(crate::contrast(accent, background) >= crate::AA);
    }

    #[test]
    fn a_grey_cover_is_not_given_colour_it_does_not_have() {
        let grey = hsla(0., 0.04, 0.5, 1.);
        assert_eq!(worn(grey, hsla(0., 0., 0., 1.)).s, grey.s);
    }

    #[test]
    fn a_light_theme_darkens_the_accent_rather_than_brightening_it() {
        let pale = hsla(210. / 360., 0.386, 0.75, 1.);
        assert!(worn(pale, hsla(0., 0., 1., 1.)).l < pale.l);
    }

    #[test]
    fn off_lends_its_accent_to_nothing() {
        assert!(!Surface::Sheet.allowed(Accent::Off));
        assert!(!Surface::Chrome.allowed(Accent::Off));
    }

    #[test]
    fn sheet_only_leaves_the_always_on_chrome_alone() {
        assert!(Surface::Sheet.allowed(Accent::SheetOnly));
        assert!(!Surface::Chrome.allowed(Accent::SheetOnly));
    }

    #[test]
    fn on_reaches_every_surface() {
        assert!(Surface::Sheet.allowed(Accent::On));
        assert!(Surface::Chrome.allowed(Accent::On));
    }

    #[test]
    fn every_surface_keys_its_own_motion_channel() {
        assert_ne!(Surface::Sheet.key(), Surface::Chrome.key());
    }
}
