//! WCAG contrast in GPUI terms: the check any extracted colour must pass before it carries text.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Hsla, Rgba};

/// WCAG 2.1 AA for body text.
pub const AA: f32 = 4.5;
/// WCAG 2.1 AA for large text: 24px, or 18.66px bold.
pub const AA_LARGE: f32 = 3.0;

/// The background luminance either side of which black or white is the higher-contrast text.
const PIVOT: f32 = 0.179;

/// sRGB relative luminance, the quantity both WCAG ratios are built from.
pub fn luminance(color: Hsla) -> f32 {
    let rgba: Rgba = color.into();
    let linear = |channel: f32| {
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b)
}

/// The WCAG contrast ratio between two opaque colours, from 1.0 to 21.0.
pub fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// `color` moved in lightness until it reads at `ratio` against `background`, keeping hue and saturation.
pub fn readable_at(color: Hsla, background: Hsla, ratio: f32) -> Hsla {
    if contrast(color, background) >= ratio {
        return color;
    }
    let limit = Hsla {
        l: if luminance(background) > PIVOT {
            0.
        } else {
            1.
        },
        ..color
    };
    // Neither black nor white reaches the ratio on a mid-grey background: the extreme is the best there is.
    if contrast(limit, background) < ratio {
        return limit;
    }
    let (mut near, mut far) = (color.l, limit.l);
    for _ in 0..16 {
        let middle = (near + far) / 2.;
        if contrast(Hsla { l: middle, ..color }, background) >= ratio {
            far = middle;
        } else {
            near = middle;
        }
    }
    Hsla { l: far, ..color }
}

/// The same at AA, the bar any body text has to clear.
pub fn readable_on(color: Hsla, background: Hsla) -> Hsla {
    readable_at(color, background, AA)
}

/// The same against whatever the active theme paints behind it.
pub fn readable(color: Hsla, cx: &App) -> Hsla {
    readable_on(color, cx.theme().background)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::hsla;

    fn hex(color: &str) -> Hsla {
        Rgba::try_from(color).expect("a test colour parses").into()
    }

    #[test]
    fn black_on_white_is_the_widest_ratio_there_is() {
        let ratio = contrast(hex("#000000"), hex("#ffffff"));
        assert!((ratio - 21.).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn a_colour_against_itself_has_no_contrast() {
        assert!((contrast(hex("#7f7f7f"), hex("#7f7f7f")) - 1.).abs() < 0.001);
    }

    #[test]
    fn contrast_does_not_depend_on_the_order() {
        let (a, b) = (hex("#1e66f5"), hex("#eff1f5"));
        assert!((contrast(a, b) - contrast(b, a)).abs() < 0.0001);
    }

    #[test]
    fn a_colour_that_already_reads_is_left_alone() {
        let text = hex("#ffffff");
        assert_eq!(readable_on(text, hex("#000000")), text);
    }

    #[test]
    fn a_dim_colour_on_a_dark_background_is_lightened_until_aa_holds() {
        let background = hex("#141415");
        let clamped = readable_on(hex("#333738"), background);
        assert!(clamped.l > 0.2, "{}", clamped.l);
        assert!(contrast(clamped, background) >= AA);
    }

    #[test]
    fn a_pale_colour_on_a_light_background_is_darkened_until_aa_holds() {
        let background = hex("#fafafa");
        let clamped = readable_on(hex("#d8d8d8"), background);
        assert!(clamped.l < 0.8, "{}", clamped.l);
        assert!(contrast(clamped, background) >= AA);
    }

    #[test]
    fn the_clamp_keeps_hue_and_saturation() {
        let color = hsla(0.6, 0.8, 0.45, 1.);
        let clamped = readable_on(color, hsla(0.6, 0.8, 0.5, 1.));
        assert!((clamped.h - color.h).abs() < 0.0001);
        assert!((clamped.s - color.s).abs() < 0.0001);
    }

    #[test]
    fn a_ratio_no_lightness_can_reach_gets_the_extreme_rather_than_a_loop() {
        let background = hsla(0., 0., 0.5, 1.);
        let clamped = readable_at(hsla(0., 0., 0.55, 1.), background, 7.);
        assert!(clamped.l == 0. || clamped.l == 1., "{}", clamped.l);
    }

    #[test]
    fn the_clamp_moves_no_further_than_it_has_to() {
        let background = hex("#000000");
        let clamped = readable_on(hsla(0.08, 0.7, 0.2, 1.), background);
        assert!(contrast(clamped, background) >= AA);
        assert!(contrast(clamped, background) < AA + 0.5, "overshot");
    }
}
