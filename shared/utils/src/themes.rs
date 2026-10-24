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

    // Maps values back to the css custom properties Kopuz uses.
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
