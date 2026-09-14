//! Rendering what the daemon publishes: the text, the glyphs, and the field
//! lists that stand in for every form a service used to need its own code for.

pub mod schema_form;

use dioxus::prelude::*;

/// Resolve published text: a key through this client's own translations, a
/// literal as it stands.
pub fn text(text: &api::Text) -> String {
    match text {
        api::Text::Key(key) => i18n::t(key),
        api::Text::Literal(literal) => literal.clone(),
    }
}

pub fn text_or_empty(value: Option<&api::Text>) -> String {
    value.map(text).unwrap_or_default()
}

/// A published glyph: a font class, or a mark drawn from a path for the brands
/// the icon font has none for.
#[component]
pub fn Glyph(icon: api::Icon, #[props(default)] class: String) -> Element {
    match icon {
        api::Icon::Svg(path) => rsx! {
            svg {
                class: "{class}",
                view_box: "0 0 24 24",
                fill: "currentColor",
                "aria-hidden": "true",
                path { d: "{path}" }
            }
        },
        api::Icon::Class(glyph) => rsx! {
            i { class: "{glyph} {class}" }
        },
    }
}
