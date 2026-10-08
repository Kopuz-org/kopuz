//! Material color roles from the desktop wallpaper or Android's system palette.

use std::collections::BTreeMap;

use config::AppConfig;
use dioxus::prelude::*;
use material_colors::{
    color::Argb,
    dynamic_color::DynamicScheme,
    hct::Hct,
    scheme::Scheme,
    scheme::variant::{SchemeContent, SchemeMonochrome, SchemeTonalSpot},
};
use utils::color::Color;

#[cfg(any(target_os = "android", test))]
mod android;
#[cfg(not(target_os = "android"))]
mod desktop;

const SELECTOR: &str = ".theme-system[data-ui-style]";

/// The default theme's copper, for when there is no wallpaper or artwork.
const DEFAULT_SEED: Argb = Argb::from_u32(0xffd9842f);

type Variant = fn(Hct, bool) -> DynamicScheme;

fn tonal_spot(seed: Hct, dark: bool) -> DynamicScheme {
    SchemeTonalSpot::new(seed, dark, Some(0.0)).scheme
}

fn content(seed: Hct, dark: bool) -> DynamicScheme {
    SchemeContent::new(seed, dark, Some(0.0)).scheme
}

fn monochrome(seed: Hct, dark: bool) -> DynamicScheme {
    SchemeMonochrome::new(seed, dark, Some(0.0)).scheme
}

fn roles(seed: Argb, variant: Variant, dark: bool) -> BTreeMap<String, Argb> {
    Scheme::from(variant(Hct::new(seed), dark))
        .into_iter()
        .collect()
}

fn tonal_roles(seed: Argb, dark: bool) -> BTreeMap<String, Argb> {
    roles(seed, tonal_spot, dark)
}

/// The `--color-*` vars every theme sets, and the Material role each takes.
const THEME_VARS: &[(&str, &str)] = &[
    ("black", "surface"),
    ("white", "on_surface"),
    ("slate-400", "on_surface_variant"),
    ("slate-500", "on_surface_variant"),
    ("green-500", "primary"),
    ("indigo-400", "primary"),
    ("indigo-500", "primary"),
    ("indigo-600", "primary"),
    ("indigo-900", "primary_container"),
    ("purple-600", "tertiary"),
    ("purple-700", "tertiary_container"),
    ("red-400", "error"),
    ("neutral-900", "surface_container"),
];

fn color_css(roles: &BTreeMap<String, Argb>, dark: bool) -> String {
    let mut css = format!(
        "{SELECTOR} {{ color-scheme: {};",
        if dark { "dark" } else { "light" }
    );
    for (name, color) in roles {
        css.push_str(&format!(
            "--md-sys-color-{}: {color};",
            name.replace('_', "-")
        ));
    }
    for (variable, role) in THEME_VARS {
        if let Some(color) = roles.get(*role) {
            css.push_str(&format!("--color-{variable}: {color};"));
        }
    }
    css.push('}');
    css
}

fn scheme_css(seed: Argb, variant: Variant) -> String {
    format!(
        "{} @media (prefers-color-scheme: dark) {{ {} }}",
        color_css(&roles(seed, variant, false), false),
        color_css(&roles(seed, variant, true), true),
    )
}

fn tonal_css(seed: Argb) -> String {
    scheme_css(seed, tonal_spot)
}

/// Below this HCT chroma a palette colour is grey, and its hue is noise.
const MIN_SEED_CHROMA: f64 = 15.0;

/// Album art is content colour, so it takes Material's content scheme, which
/// keeps the cover's own chroma instead of TonalSpot's fixed one. The seed is
/// the most common colour that has a hue at all; a cover with none (black and
/// white art) gets a neutral scheme rather than a hue invented from grey.
fn artwork_css(colors: &[Color]) -> String {
    match artwork_seed(colors) {
        Some(seed) => scheme_css(seed, content),
        None => scheme_css(colors.first().map_or(DEFAULT_SEED, argb), monochrome),
    }
}

fn artwork_seed(colors: &[Color]) -> Option<Argb> {
    colors
        .iter()
        .map(argb)
        .find(|color| Hct::new(*color).get_chroma() >= MIN_SEED_CHROMA)
}

fn argb(color: &Color) -> Argb {
    Argb::from_u32(
        0xff000000 | u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b),
    )
}

#[component]
pub fn SystemColors(config: Signal<AppConfig>, artwork: Signal<Option<Vec<Color>>>) -> Element {
    let enabled = use_memo(move || config.read().theme == "system");
    let mut system_css = use_signal(|| None::<String>);
    use_resource(move || {
        let enabled = enabled();
        async move {
            #[cfg(target_os = "android")]
            android::set_enabled(enabled);
            // macOS gives no reliable read of the wallpaper on screen, so there
            // System colors follows the album art (the fallback below).
            if !enabled || cfg!(target_os = "macos") {
                system_css.set(None);
                return;
            }
            #[cfg(not(target_os = "android"))]
            let mut wallpaper = desktop::Wallpaper::default();
            loop {
                #[cfg(target_os = "android")]
                let next = android::read().await;
                #[cfg(not(target_os = "android"))]
                let next = {
                    let live_path = config.peek().live_theme_path.clone();
                    let (cache, seed) = tokio::task::spawn_blocking(move || {
                        let seed = wallpaper.seed(&live_path);
                        (wallpaper, seed)
                    })
                    .await
                    .unwrap_or_default();
                    wallpaper = cache;
                    desktop::css(seed).await
                };
                if *system_css.peek() != next {
                    system_css.set(next);
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    });

    let css = use_memo(move || {
        if !enabled() {
            return String::new();
        }
        system_css
            .read()
            .clone()
            .unwrap_or_else(|| match artwork.read().as_deref() {
                Some(colors) if !colors.is_empty() => artwork_css(colors),
                _ => tonal_css(DEFAULT_SEED),
            })
    });
    rsx! { style { id: "system-colors", "{css}" } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Argb) -> f64 {
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
    }

    fn contrast(a: Argb, b: Argb) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// `--surface-*: color-mix(in srgb, var(--color-A, …) P%, var(--color-B, …))`
    /// from themes.css, as (name, A, P, B).
    fn surface_tokens() -> Vec<(String, String, f64, String)> {
        include_str!("../../assets/themes.css")
            .lines()
            .filter_map(|line| {
                let (name, mix) = line.trim().strip_prefix("--surface-")?.split_once(':')?;
                let (_, rest) = mix.split_once("var(--color-")?;
                let (first, rest) = rest.split_once(',')?;
                let (_, rest) = rest.split_once(") ")?;
                let (percent, rest) = rest.split_once('%')?;
                let (_, rest) = rest.split_once("var(--color-")?;
                let (second, _) = rest.split_once(',')?;
                Some((
                    name.to_string(),
                    first.to_string(),
                    percent.parse().ok()?,
                    second.to_string(),
                ))
            })
            .collect()
    }

    #[test]
    fn chrome_surfaces_keep_their_text_readable_in_both_appearances() {
        let tokens = surface_tokens();
        assert_eq!(tokens.len(), 3, "{tokens:?}");
        let role_of = |var: &str| {
            THEME_VARS
                .iter()
                .find(|(name, _)| *name == var)
                .map(|(_, role)| *role)
                .unwrap_or_else(|| panic!("--color-{var} is not themed"))
        };
        for seed in [0xffd9842f, 0xff000000, 0xffffffff, 0xff006aff, 0xff00ff00] {
            for dark in [false, true] {
                let roles = tonal_roles(Argb::from_u32(seed), dark);
                for (name, first, percent, second) in &tokens {
                    let (a, b) = (roles[role_of(first)], roles[role_of(second)]);
                    let mix = |x: u8, y: u8| {
                        (f64::from(x) * percent / 100.0 + f64::from(y) * (1.0 - percent / 100.0))
                            .round() as u8
                    };
                    let surface = Argb::new(
                        255,
                        mix(a.red, b.red),
                        mix(a.green, b.green),
                        mix(a.blue, b.blue),
                    );
                    for text in ["white", "slate-400"] {
                        let ratio = contrast(surface, roles[role_of(text)]);
                        assert!(
                            ratio >= 4.5,
                            "{seed:x} dark={dark} {name} {text}: {ratio:.2}"
                        );
                    }
                }
            }
        }
    }

    /// A literal hex surface ignores the theme: under a light one, or System
    /// colors in a light appearance, its `text-white` copy turns dark on dark.
    #[test]
    fn frontend_surfaces_take_their_colour_from_the_theme() {
        fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, hits);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let source = std::fs::read_to_string(&path).unwrap();
                    for (index, line) in source.lines().enumerate() {
                        // Split so this test does not find itself.
                        if line.contains(&["bg-[", "#"].concat()) {
                            hits.push(format!("{}:{}", path.display(), index + 1));
                        }
                    }
                }
            }
        }
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut hits = Vec::new();
        for frontend in ["components", "pages", "kopuz"] {
            walk(&crates.join(frontend).join("src"), &mut hits);
        }
        assert!(hits.is_empty(), "hardcoded surface colours: {hits:#?}");
    }

    #[test]
    fn grey_artwork_gets_no_invented_hue() {
        let grey = [
            Color::new(4, 4, 4),
            Color::new(240, 240, 240),
            Color::new(90, 92, 95),
        ];
        assert_eq!(artwork_seed(&grey), None);

        let red = Color::new(200, 30, 40);
        let dark_with_red = [Color::new(8, 8, 8), Color::new(250, 250, 250), red.clone()];
        assert_eq!(artwork_seed(&dark_with_red), Some(argb(&red)));
    }

    #[test]
    fn dynamic_roles_keep_text_readable_in_both_appearances() {
        let variants: [Variant; 3] = [tonal_spot, content, monochrome];
        for (seed, variant) in [0xffd9842f, 0xff000000, 0xffffffff, 0xff006aff, 0xff00ff00]
            .into_iter()
            .flat_map(|seed| variants.map(|variant| (seed, variant)))
        {
            for dark in [false, true] {
                let roles = roles(Argb::from_u32(seed), variant, dark);
                for (background, foreground) in [
                    ("surface", "on_surface"),
                    ("surface_container", "on_surface"),
                    ("primary", "on_primary"),
                    ("secondary_container", "on_secondary_container"),
                ] {
                    let a = luminance(roles[background]);
                    let b = luminance(roles[foreground]);
                    assert!(
                        (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5,
                        "{seed:x} {dark} {background}"
                    );
                }
            }
        }
    }
}
