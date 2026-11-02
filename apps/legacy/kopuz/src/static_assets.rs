//! Embed styles and fonts so startup does not depend on Dioxus asset collection.

const FAVICON: &str = include_str!(concat!(env!("OUT_DIR"), "/favicon.uri"));
const MAIN_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/main.css"));
const THEME_CSS: &str = include_str!("../assets/themes.css");
const MATERIAL3_CSS: &str = include_str!("../assets/material3.css");
const TAILWIND_CSS: &str = include_str!("../assets/tailwind.css");
const REDUCED_ANIMATIONS_CSS: &str = include_str!("../assets/reduced-animations.css");
const FONT_AWESOME_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/fontawesome.css"));
const JETBRAINS_MONO_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/jetbrains-mono.css"));
const ROBOTO_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/roboto.css"));

/// Put static assets in the initial document. `document::Style` sends the CSS
/// and embedded fonts over the WebView's JavaScript bridge after the first DOM
/// edits, allowing an unstyled frame and delaying font decoding on Android.
/// An explicit UTF-8 charset preserves non-ASCII CSS content.
pub fn head() -> String {
    let mut head = String::from("<meta charset=\"utf-8\">");
    head.push_str("<link rel=\"icon\" href=\"");
    head.push_str(FAVICON);
    head.push_str("\">");
    for (name, css) in [
        ("main", MAIN_CSS),
        ("themes", THEME_CSS),
        ("tailwind", TAILWIND_CSS),
        ("material3", MATERIAL3_CSS),
        ("reduced-animations", REDUCED_ANIMATIONS_CSS),
        ("jetbrains-mono", JETBRAINS_MONO_CSS),
        ("roboto", ROBOTO_CSS),
        ("fontawesome", FONT_AWESOME_CSS),
    ] {
        head.push_str("<style id=\"kopuz-");
        head.push_str(name);
        head.push_str("\">");
        head.push_str(css);
        head.push_str("</style>");
    }
    head.push_str(&cover_fallback_script());
    head
}

// Keyed on the failing src, not a one-shot flag: a reused img (the player bar's) fails again next track.
fn cover_fallback_script() -> String {
    format!(
        r#"<script>document.addEventListener('error',function(e){{
            var t=e.target, fallback='{}';
            var src=t.getAttribute&&t.getAttribute('src');
            if(t.tagName==='IMG'&&src&&src!==fallback){{
                t.src=fallback;
            }}
        }},true);</script>"#,
        utils::DEFAULT_COVER_SVG.replace('\'', "%27"),
    )
}

#[cfg(test)]
mod tests {
    use super::THEME_CSS;
    use std::collections::HashMap;
    use utils::themes::contrast_ratio;

    /// Each top-level `.theme-*` block's custom properties, with one level of
    /// `var(--x)` resolved against the same block.
    fn themes() -> Vec<(String, HashMap<String, String>)> {
        let mut out = Vec::new();
        let mut current: Option<(String, HashMap<String, String>)> = None;
        for line in THEME_CSS.lines() {
            let line = line.trim();
            if let Some(name) = line
                .strip_prefix(".theme-")
                .and_then(|rest| rest.strip_suffix('{'))
                .map(str::trim)
                .filter(|name| !name.contains([' ', ',', '.']))
            {
                current = Some((name.to_string(), HashMap::new()));
            } else if line == "}" {
                out.extend(current.take());
            } else if let Some((_, vars)) = current.as_mut()
                && let Some((key, value)) = line.strip_suffix(';').and_then(|l| l.split_once(':'))
                && key.starts_with("--")
            {
                vars.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        for (_, vars) in &mut out {
            let snapshot = vars.clone();
            for value in vars.values_mut() {
                if let Some(target) = value
                    .strip_prefix("var(")
                    .and_then(|v| v.strip_suffix(')'))
                    .and_then(|v| snapshot.get(v))
                {
                    *value = target.clone();
                }
            }
        }
        out
    }

    #[test]
    fn every_theme_is_parsed() {
        assert!(themes().len() > 20);
    }

    /// Filled accent buttons draw `--color-on-accent` over `--color-indigo-600`;
    /// this catches an unreadable pair without checking every theme by eye.
    #[test]
    fn accent_button_labels_meet_aa_contrast() {
        let mut failures = Vec::new();
        for (name, vars) in themes() {
            let Some(fill) = vars.get("--color-indigo-600") else {
                continue;
            };
            let Some(label) = vars.get("--color-on-accent") else {
                failures.push(format!("{name}: no --color-on-accent"));
                continue;
            };
            match contrast_ratio(fill, label) {
                Some(ratio) if ratio >= 4.5 => {}
                Some(ratio) => failures.push(format!("{name}: {label} on {fill} is {ratio:.2}:1")),
                None => failures.push(format!("{name}: unparsable {label} / {fill}")),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
