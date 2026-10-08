//! Material color roles from the desktop wallpaper or Android's system palette.

use std::collections::BTreeMap;

use config::AppConfig;
use dioxus::prelude::*;
use material_colors::{color::Argb, hct::Hct, scheme::Scheme, scheme::variant::SchemeTonalSpot};
use utils::color::Color;

#[cfg(any(target_os = "android", test))]
mod android;
#[cfg(not(target_os = "android"))]
mod desktop;

const SELECTOR: &str = ".theme-system[data-ui-style]";

fn tonal_roles(seed: Argb, dark: bool) -> BTreeMap<String, Argb> {
    Scheme::from(SchemeTonalSpot::new(Hct::new(seed), dark, Some(0.0)).scheme)
        .into_iter()
        .collect()
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

fn tonal_css(seed: Argb) -> String {
    format!(
        "{} @media (prefers-color-scheme: dark) {{ {} }}",
        color_css(&tonal_roles(seed, false), false),
        color_css(&tonal_roles(seed, true), true),
    )
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
        system_css.read().clone().unwrap_or_else(|| {
            let seed = artwork
                .read()
                .as_ref()
                .and_then(|colors| colors.first())
                .map_or(Argb::from_u32(0xffd9842f), argb);
            tonal_css(seed)
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
    fn dynamic_roles_keep_text_readable_in_both_appearances() {
        for seed in [0xffd9842f, 0xff000000, 0xffffffff, 0xff006aff, 0xff00ff00] {
            for dark in [false, true] {
                let roles = tonal_roles(Argb::from_u32(seed), dark);
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
