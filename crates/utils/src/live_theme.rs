//! Wallpaper-driven themes from [matugen](https://github.com/InioX/matugen) and
//! [pywal](https://github.com/dylanaraps/pywal).
//!
//! Both tools rewrite their output every time the wallpaper changes, so Kopuz
//! polls that file while the theme is selected and recolours in place. No
//! restart, and nothing to wire up on the generator's side.
//!
//! Two file shapes are accepted. Pywal's own `colors.json` is read directly, so
//! pywal users only have to point Kopuz at it. Matugen has no fixed output, so
//! it renders a template: a flat object keyed the way `assets/themes.json` keys
//! a theme.
//!
//! ```json
//! {
//!   "bg":   "{{colors.surface.default.hex}}",
//!   "text": "{{colors.on_surface.default.hex}}"
//! }
//! ```
//!
//! Keys Kopuz doesn't know are ignored and missing ones keep the built-in
//! default, so a partial palette is fine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Theme id the palette is injected under, and the value `AppConfig::theme`
/// carries while a generator drives the colours.
pub const THEME_ID: &str = "live";

/// Default matugen target, under the config dir so a template can point at it
/// without knowing the install layout.
const MATUGEN_FILE: &str = "matugen.json";

/// Pywal's colour groups mapped onto Kopuz's theme vars. Pywal always emits all
/// sixteen, so this only has to pick sensible roles for them.
const PYWAL_MAP: &[(&str, &str, &str)] = &[
    ("bg", "special", "background"),
    ("text", "special", "foreground"),
    ("text-muted", "colors", "color8"),
    ("surface", "colors", "color7"),
    ("progress", "colors", "color2"),
    ("accent-soft", "colors", "color12"),
    ("accent", "colors", "color4"),
    ("accent-alt", "colors", "color6"),
    ("accent-deep", "colors", "color0"),
    ("highlight", "colors", "color13"),
    ("highlight-dark", "colors", "color5"),
    ("danger", "colors", "color1"),
    ("raised", "colors", "color0"),
];

/// Where the palette is read from: the configured path, otherwise whichever
/// default already exists. Falls back to the matugen one so the settings screen
/// always has a concrete path to show.
pub fn resolve_path(configured: &str) -> PathBuf {
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    let matugen = db::config_dir().join(MATUGEN_FILE);
    if matugen.exists() {
        return matugen;
    }
    pywal_path().filter(|p| p.exists()).unwrap_or(matugen)
}

/// Where pywal writes its palette. It follows XDG on every platform it runs on,
/// so this doesn't go through `directories`.
fn pywal_path() -> Option<PathBuf> {
    let cache = match std::env::var_os("XDG_CACHE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    Some(cache.join("wal").join("colors.json"))
}

/// Raw palette text, `None` while the file doesn't exist. A generator that
/// hasn't run yet is the normal case, so that stays quiet.
///
/// Polling compares this verbatim rather than a modification time and length
/// stamp. Every palette is the same length, because the values are all
/// fixed-width hex, so a stamp would rest entirely on the filesystem's mtime
/// resolution and miss two wallpapers picked inside the same tick.
pub fn read(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => Some(raw),
        Err(e) => {
            tracing::debug!("live palette {} unreadable: {e}", path.display());
            None
        }
    }
}

/// Theme vars from palette text. Malformed JSON means the user's template is
/// wrong, so it warns and leaves the current colours up.
pub fn parse(raw: &str, source: &Path) -> Option<HashMap<String, String>> {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => Some(pywal_vars(&value).unwrap_or_else(|| flat_vars(&value))),
        Err(e) => {
            tracing::warn!("live palette {} is malformed: {e}", source.display());
            None
        }
    }
}

/// Pywal's shape. Both groups together are what tells it apart from a rendered
/// matugen template, which is flat.
fn pywal_vars(value: &serde_json::Value) -> Option<HashMap<String, String>> {
    let has_group = |key| value.get(key).is_some_and(serde_json::Value::is_object);
    if !(has_group("special") && has_group("colors")) {
        return None;
    }
    Some(
        PYWAL_MAP
            .iter()
            .filter_map(|(purpose, group, key)| {
                let hex = value.get(group)?.get(key)?.as_str()?;
                Some(((*purpose).to_string(), hex.to_string()))
            })
            .collect(),
    )
}

/// A rendered template. Non-string values are dropped instead of failing the
/// whole file, so a template can carry extras Kopuz has no use for.
fn flat_vars(value: &serde_json::Value) -> HashMap<String, String> {
    value
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// CSS for the palette, empty when there is nothing usable to inject.
///
/// The `:root` prefix is what lets the default theme sit underneath as a
/// fallback for missing vars: both are plain `.theme-*` rules otherwise, so
/// which one won would come down to the order things land in `<head>`.
pub fn to_css(vars: &HashMap<String, String>) -> String {
    if vars.is_empty() {
        return String::new();
    }
    crate::themes::Theme {
        id: THEME_ID.to_string(),
        name: String::new(),
        kind: crate::themes::ThemeKind::Dark,
        vars: vars.clone(),
    }
    .to_css_for(&format!(":root .theme-{THEME_ID}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_str(json: &str) -> HashMap<String, String> {
        parse(json, Path::new("palette.json")).expect("test json parses")
    }

    #[test]
    fn malformed_json_leaves_the_current_colours_alone() {
        assert!(parse("{ not json", Path::new("palette.json")).is_none());
    }

    #[test]
    fn a_rendered_template_is_read_flat() {
        let vars = read_str(r##"{"bg": "#101010", "text": "#eeeeee"}"##);
        assert_eq!(vars.get("bg").map(String::as_str), Some("#101010"));
        assert_eq!(vars.get("text").map(String::as_str), Some("#eeeeee"));
    }

    #[test]
    fn non_string_values_are_dropped_rather_than_failing_the_file() {
        let vars = read_str(r##"{"bg": "#101010", "alpha": 100}"##);
        assert_eq!(vars.get("bg").map(String::as_str), Some("#101010"));
        assert!(!vars.contains_key("alpha"));
    }

    #[test]
    fn pywal_colors_json_is_mapped_without_a_template() {
        let vars = read_str(
            r##"{
                "wallpaper": "/home/me/wall.png",
                "special": {"background": "#1d2021", "foreground": "#d4be98"},
                "colors": {"color0": "#1b1b1b", "color1": "#ea6962", "color2": "#a9b665",
                           "color4": "#7daea3", "color5": "#d3869b", "color6": "#89b482",
                           "color7": "#d4be98", "color8": "#a89984", "color12": "#7daea3",
                           "color13": "#d3869b"}
            }"##,
        );
        assert_eq!(vars.get("bg").map(String::as_str), Some("#1d2021"));
        assert_eq!(vars.get("text").map(String::as_str), Some("#d4be98"));
        assert_eq!(vars.get("danger").map(String::as_str), Some("#ea6962"));
        assert!(!vars.contains_key("wallpaper"));
    }

    #[test]
    fn known_vars_reach_the_css_and_unknown_ones_do_not() {
        let vars = HashMap::from([
            ("bg".to_string(), "#101010".to_string()),
            ("name".to_string(), "Whatever".to_string()),
        ]);
        let css = to_css(&vars);
        assert!(css.starts_with(":root .theme-live {"));
        assert!(css.contains("--color-black: #101010;"));
        assert!(!css.contains("Whatever"));
    }

    #[test]
    fn an_empty_palette_injects_nothing() {
        assert!(to_css(&HashMap::new()).is_empty());
    }

    #[test]
    fn a_configured_path_wins_over_the_defaults() {
        assert_eq!(
            resolve_path("/tmp/palette.json"),
            PathBuf::from("/tmp/palette.json")
        );
    }
}
