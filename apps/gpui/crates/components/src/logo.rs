//! Kopuz's mark as outlines only: the one copy of its paths, for the loader and the step header.

/// The ten shapes of `logo-symbol.svg`, each with its measured outline length.
const PATHS: [(&str, f32); 10] = [
    (
        "M438.722 219.361C438.722 340.511 340.511 438.722 219.361 438.722C98.2113 438.722 0 340.511 0 219.361C0 98.2113 98.2113 0 219.361 0C340.511 0 438.722 98.2113 438.722 219.361Z",
        1378.5,
    ),
    (
        "M420.676 26.2555C430.194 21.1278 442.072 24.6787 447.206 34.1866L456.501 51.4022L439.266 60.6866C429.747 65.8143 417.869 62.2634 412.736 52.7555C407.602 43.2476 411.157 31.3831 420.676 26.2555Z",
        131.4,
    ),
    (
        "M516.063 129.114C507.976 133.269 498.048 130.09 493.888 122.012L486.355 107.386L500.997 99.8621C509.084 95.7066 519.012 98.886 523.172 106.964C527.333 115.041 524.15 124.958 516.063 129.114Z",
        110.5,
    ),
    (
        "M478.98 137.072C473.483 139.897 466.733 137.735 463.905 132.244C461.076 126.752 463.24 120.009 468.739 117.184L478.693 112.069L483.814 122.012C486.643 127.504 484.478 134.246 478.98 137.072Z",
        75.1,
    ),
    (
        "M256.399 181.706L462.189 76.0873L501.353 90.5641L334.746 173.316L256.399 181.706Z",
        537.9,
    ),
    (
        "M500.997 71.5357L501.28 90.6372L346.902 171.239H337.531L364.778 147.96L500.997 71.5357Z",
        394.7,
    ),
    (
        "M256.43 162.377L462.22 56.7587L501.355 71.5357L364.778 147.96L334.006 171.458L256.43 162.377Z",
        546.5,
    ),
    (
        "M371.427 151.275C372.439 160.809 364.778 166.511 361.067 173.316C354.364 189.059 352.761 191.475 350.098 209.17C337.531 292.66 308.317 334.591 240.217 349.444C153.289 358.645 38.7736 326.469 47.0212 230.817C54.2225 169.139 109.629 143.951 196.557 134.75C319.14 121.775 367.024 109.766 371.427 151.275Z",
        899.1,
    ),
    (
        "M329.48 282.909C289.766 370.665 156.859 371.901 121.576 356.996C63.5168 332.469 40.7058 293.869 47.0186 250.655C47.0186 250.655 73.0214 310.024 142.653 321.083C212.284 332.142 329.48 282.909 329.48 282.909Z",
        695.2,
    ),
    (
        "M104.662 107.405C133.466 96.1716 165.933 110.39 177.179 139.161L197.541 191.257L145.386 211.596C116.582 222.829 84.1144 208.611 72.8686 179.84C61.6228 151.068 75.857 118.638 104.662 107.405Z",
        375.7,
    ),
];

/// The mark's extent before any stroke spills past it.
const EXTENT: (f32, f32) = (525., 439.);

/// The loader's stroke, drawn at about 120px wide.
pub const THIN: f32 = 7.;
/// The step header's stroke at about 64px wide: heavy enough to read, light enough to keep the wing's lines apart.
pub const BOLD: f32 = 12.;

/// The rendered size; the loader draws it at about half this, so a HiDPI screen stays sharp.
const WIDTH: u32 = 280;
const HEIGHT: u32 = 234;

/// The whole outline at the `BOLD` stroke, for GPUI's `svg()` to tint.
pub const BOLD_SVG: &str = include_str!("../assets/kopuz-logo.svg");

/// Width over height of what `svg` draws at `stroke`, so an element can size to it.
pub fn aspect(stroke: f32) -> f32 {
    let pad = pad(stroke);
    (EXTENT.0 + 2. * pad) / (EXTENT.1 + 2. * pad)
}

fn pad(stroke: f32) -> f32 {
    stroke / 2. + 4.5
}

/// The outline at `stroke` with every path drawn to `share` of its length.
pub fn svg(share: f32, stroke: f32) -> String {
    let pad = pad(stroke);
    let (width, height) = (EXTENT.0 + 2. * pad, EXTENT.1 + 2. * pad);
    let mut out = format!(
        "<svg width=\"{WIDTH}\" height=\"{HEIGHT}\" viewBox=\"-{pad} -{pad} {width} {height}\" fill=\"none\" xmlns=\"http://www.w3.org/2000/svg\">"
    );
    let share = share.clamp(0., 1.);
    for (d, length) in PATHS {
        if share <= 0. {
            break;
        }
        let dash = match share >= 1. {
            true => String::new(),
            false => format!(
                " stroke-dasharray=\"{:.1} {:.1}\"",
                length * share,
                length + 1.
            ),
        };
        out.push_str(&format!(
            "<path d=\"{d}\" stroke=\"white\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash}/>"
        ));
    }
    out.push_str("</svg>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_is_white_outlines_only() {
        let full = svg(1., THIN);
        assert_eq!(full.matches("<path").count(), PATHS.len());
        assert!(!full.contains("fill=\"#"));
        assert!(!full.contains("stroke-dasharray"));
        assert_eq!(svg(0., THIN).matches("<path").count(), 0);
        assert!(svg(0.5, THIN).contains("stroke-dasharray"));
    }

    #[test]
    fn the_loader_keeps_its_frame() {
        assert!(svg(1., THIN).contains("viewBox=\"-8 -8 541 455\""));
        assert!(svg(1., THIN).contains("stroke-width=\"7\""));
    }

    #[test]
    fn the_committed_mark_is_the_builder_output() {
        assert_eq!(BOLD_SVG, svg(1., BOLD));
    }
}
