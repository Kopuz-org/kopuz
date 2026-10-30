use std::collections::HashMap;

const VAR_MAP: &[(&str, &str)] = &[
    ("bg", "--color-black"),
    ("text", "--color-white"),
    ("text-muted", "--color-slate-400"),
    ("surface", "--color-slate-500"),
    ("progress", "--color-green-500"),
    ("accent-soft", "--color-indigo-400"),
    ("accent", "--color-indigo-500"),
    ("accent-alt", "--color-indigo-600"),
    ("accent-deep", "--color-indigo-900"),
    ("highlight", "--color-purple-600"),
    ("highlight-dark", "--color-purple-700"),
    ("danger", "--color-red-400"),
    ("raised", "--color-neutral-900"),
];

#[derive(Debug, Clone, PartialEq)]
pub enum ThemeKind {
    Dark,
    Light,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub kind: ThemeKind,
    pub vars: HashMap<String, String>,
}

impl Theme {
    pub fn var(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn to_css(&self) -> String {
        self.to_css_for(&format!(".theme-{}", self.id))
    }

    /// The same body under a caller-chosen selector, for a theme that has to
    /// outrank an existing `.theme-*` rule instead of sitting beside one.
    pub fn to_css_for(&self, selector: &str) -> String {
        let mut out = format!("{selector} {{\n");
        for (purpose, css_var) in VAR_MAP {
            if let Some(val) = self.var(purpose) {
                out.push_str(&format!("    {}: {};\n", css_var, val));
            }
        }
        if let Some(label) = self.var("accent-alt").and_then(label_on) {
            out.push_str(&format!("    --color-on-accent: {label};\n"));
        }
        out.push('}');
        out
    }
}

/// Generate a CSS block for a single custom theme given its id and var map.
pub fn custom_theme_to_css(id: &str, vars: &std::collections::HashMap<String, String>) -> String {
    let theme = Theme {
        id: id.to_string(),
        name: String::new(),
        kind: ThemeKind::Dark,
        vars: vars.clone(),
    };
    theme.to_css()
}

/// WCAG 2 relative luminance of a `#rrggbb` colour.
fn relative_luminance(hex: &str) -> Option<f64> {
    let hex = hex.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| -> Option<f64> {
        let value = f64::from(u8::from_str_radix(hex.get(at..at + 2)?, 16).ok()?) / 255.0;
        Some(if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        })
    };
    Some(0.2126 * channel(0)? + 0.7152 * channel(2)? + 0.0722 * channel(4)?)
}

/// WCAG 2 contrast ratio between two `#rrggbb` colours, from 1 to 21.
pub fn contrast_ratio(a: &str, b: &str) -> Option<f64> {
    let (a, b) = (relative_luminance(a)?, relative_luminance(b)?);
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    Some((light + 0.05) / (dark + 0.05))
}

/// Black or white, whichever reads better on `background`. One of the two
/// always clears 4.5:1, so a label drawn this way passes AA on any fill.
pub fn label_on(background: &str) -> Option<&'static str> {
    let on_black = contrast_ratio(background, "#000000")?;
    let on_white = contrast_ratio(background, "#ffffff")?;
    Some(if on_black >= on_white {
        "#000000"
    } else {
        "#ffffff"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_spans_one_to_twenty_one() {
        assert_eq!(contrast_ratio("#000000", "#000000"), Some(1.0));
        let max = contrast_ratio("#000000", "#ffffff").unwrap_or_default();
        assert!((max - 21.0).abs() < 1e-9);
        assert_eq!(contrast_ratio("#zzzzzz", "#000000"), None);
    }

    #[test]
    fn a_label_on_any_fill_passes_aa() {
        for fill in [
            "#fabd2f", "#b86a22", "#458588", "#7f7f7f", "#1d2021", "#ffffff",
        ] {
            let label = label_on(fill).unwrap_or_default();
            let ratio = contrast_ratio(fill, label).unwrap_or_default();
            assert!(ratio >= 4.5, "{label} on {fill} is only {ratio:.2}:1");
        }
    }

    #[test]
    fn a_custom_theme_carries_its_accent_label() {
        let vars = HashMap::from([("accent-alt".to_string(), "#fabd2f".to_string())]);
        let css = custom_theme_to_css("mine", &vars);
        assert!(css.contains("--color-on-accent: #000000;"));
    }
}
