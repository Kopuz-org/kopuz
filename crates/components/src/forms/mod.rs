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

/// A daemon failure in this client's language: by its reason, else by its code with the message as detail.
pub fn error_text(error: &api::ApiError) -> String {
    match error.reason {
        Some(reason) => i18n::t(reason.message_key()),
        None => i18n::t_with(
            error.code.message_key(),
            &[("detail", error.message.clone().into())],
        ),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_said_in_the_client_language_and_anything_else_by_its_code() {
        let keyless = api::ApiError::invalid_input("set the API key and secret first")
            .because(api::ErrorReason::ApiKeyMissing);
        assert_eq!(error_text(&keyless), "Set the API key and secret first.");
        let plain = api::ApiError::internal("database error: locked");
        assert_eq!(
            error_text(&plain).replace(['\u{2068}', '\u{2069}'], ""),
            "Something went wrong: database error: locked"
        );
        let offline = api::ApiError::new(api::ErrorCode::SourceUnreachable, "timed out");
        assert!(!error_text(&offline).contains("timed out"));
    }

    #[test]
    fn every_reason_and_code_has_a_message_in_the_baseline_locale() {
        let baseline = include_str!("../../../i18n/locales/en.ftl");
        let keys = api::ErrorReason::ALL
            .iter()
            .map(|reason| reason.message_key())
            .chain(api::ErrorCode::ALL.iter().map(|code| code.message_key()));
        for key in keys {
            assert!(
                baseline
                    .lines()
                    .any(|line| line.starts_with(&format!("{key} ="))),
                "{key} is missing from en.ftl"
            );
        }
    }
}
