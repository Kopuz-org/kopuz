//! The live theme: a matugen or pywal palette file drawn over the default, recoloured whenever the file changes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{App, Global, Task};
use utils::live_theme;

use crate::{Colors, apply, palette, parse};

/// The theme id `Ui::theme` stores while a generator drives the colours.
pub const LIVE: &str = live_theme::THEME_ID;

/// How often the file is reread; the generators rewrite it in place and announce nothing.
const POLL: Duration = Duration::from_millis(400);

/// The poller, held only while the live theme is the one applied.
struct Follower {
    _poll: Task<()>,
}

impl Global for Follower {}

/// The default with whichever of the palette's colours parse laid over it; a partial palette is fine.
pub fn live_colors(vars: &HashMap<String, String>) -> Colors {
    let mut colors: Colors = palette("default").expect("default theme").into();
    let slots: [(&str, &mut String); 13] = [
        ("bg", &mut colors.bg),
        ("raised", &mut colors.raised),
        ("surface", &mut colors.surface),
        ("text", &mut colors.text),
        ("text-muted", &mut colors.text_muted),
        ("accent", &mut colors.accent),
        ("accent-soft", &mut colors.accent_soft),
        ("accent-alt", &mut colors.accent_alt),
        ("accent-deep", &mut colors.accent_deep),
        ("highlight", &mut colors.highlight),
        ("highlight-dark", &mut colors.highlight_dark),
        ("progress", &mut colors.progress),
        ("danger", &mut colors.danger),
    ];
    for (key, slot) in slots {
        if let Some(value) = vars.get(key).filter(|value| parse(value).is_some()) {
            *slot = value.clone();
        }
    }
    colors.dark = crate::is_dark(&colors.bg);
    colors
}

pub(crate) fn following(cx: &App) -> bool {
    cx.has_global::<Follower>()
}

/// Start following the palette file.
pub(crate) fn follow(cx: &mut App) {
    let task = cx.spawn(async move |cx| {
        let mut last: Option<(PathBuf, String)> = None;
        loop {
            // Read every turn, so a new path in the settings is followed without a restart.
            let Some(path) = cx.update(configured_path) else {
                return;
            };
            let probe = path.clone();
            let raw = cx
                .background_executor()
                .spawn(async move { live_theme::read(&probe) })
                .await;
            let current = raw.map(|raw| (path, raw));
            if current != last {
                let colors = match &current {
                    // A generator mid-write leaves broken JSON for a moment; the colours up stay up.
                    Some((path, raw)) => {
                        live_theme::parse(raw, path).map(|vars| live_colors(&vars))
                    }
                    None => Some(palette("default").expect("default theme").into()),
                };
                last = current;
                if let Some(colors) = colors {
                    cx.update(|cx| {
                        apply(LIVE, &colors, None, cx);
                        cx.refresh_windows();
                    });
                }
            }
            cx.background_executor().timer(POLL).await;
        }
    });
    cx.set_global(Follower { _poll: task });
}

/// Stop following the palette file.
pub(crate) fn stop(cx: &mut App) {
    if cx.has_global::<Follower>() {
        cx.remove_global::<Follower>();
    }
}

/// The palette file to read, or `None` once another theme has been picked.
fn configured_path(cx: &mut App) -> Option<PathBuf> {
    let ui = &store::ConfigModel::global(cx).read(cx).config().ui;
    (ui.theme == LIVE).then(|| live_theme::resolve_path(&ui.live_theme_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_palette_keeps_the_default_for_what_it_leaves_out() {
        let vars = HashMap::from([("accent".to_string(), "#ff0000".to_string())]);
        let colors = live_colors(&vars);
        let default: Colors = palette("default").expect("default theme").into();
        assert_eq!(colors.accent, "#ff0000");
        assert_eq!(colors.bg, default.bg);
    }

    #[test]
    fn a_colour_that_does_not_parse_is_ignored() {
        let vars = HashMap::from([("bg".to_string(), "{{colors.surface}}".to_string())]);
        let default: Colors = palette("default").expect("default theme").into();
        assert_eq!(live_colors(&vars).bg, default.bg);
    }

    #[test]
    fn a_light_background_makes_a_light_theme() {
        let vars = HashMap::from([("bg".to_string(), "#f5f5f5".to_string())]);
        assert!(!live_colors(&vars).dark);
    }
}
