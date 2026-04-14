use serde::Deserialize;
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

    // Maps values back to the css custom properties Rusic uses.
    pub fn to_css(&self) -> String {
        let mut out = format!(".theme-{} {{\n", self.id);
        for (purpose, css_var) in VAR_MAP {
            if let Some(val) = self.var(purpose) {
                out.push_str(&format!("    {}: {};\n", css_var, val));
            }
        }
        out.push('}');
        out
    }
}

#[derive(Deserialize)]
struct RawTheme {
    name: String,
    #[serde(flatten)]
    vars: HashMap<String, String>,
}

#[derive(Deserialize)]
struct ThemeFile {
    dark: HashMap<String, RawTheme>,
    light: HashMap<String, RawTheme>,
}

pub fn load_themes() -> Vec<Theme> {
    let path = std::env::var("RUSIC_THEMES_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("assets/themes.json")))
                .unwrap_or_else(|| std::path::PathBuf::from("assets/themes.json"))
        });
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read themes.json from {}: {e}", path.display()));
    let file: ThemeFile = serde_json::from_str(&json).expect("themes.json is malformed");

    let mut themes = Vec::new();

    for (id, raw) in file.dark {
        themes.push(Theme {
            id,
            name: raw.name,
            kind: ThemeKind::Dark,
            vars: raw.vars,
        });
    }
    for (id, raw) in file.light {
        themes.push(Theme {
            id,
            name: raw.name,
            kind: ThemeKind::Light,
            vars: raw.vars,
        });
    }

    themes
}

pub fn theme_map() -> HashMap<String, Theme> {
    load_themes()
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect()
}

pub fn all_themes_css() -> String {
    load_themes()
        .iter()
        .map(|t| t.to_css())
        .collect::<Vec<_>>()
        .join("\n\n")
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
