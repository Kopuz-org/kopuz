use config::UiStyle;
use dioxus::prelude::*;

/// A listing the daemon could not answer, said rather than drawn as an empty library.
#[component]
pub fn ReadError(error: api::ApiError) -> Element {
    rsx! {
        div { class: "py-12 px-6 md:px-10 text-rose-400 text-sm",
            p { "{i18n::t(\"library_read_failed\")}" }
            p { class: "mt-1 opacity-70", "{components::forms::error_text(&error)}" }
        }
    }
}

/// Where page content starts, inline and from the top. Every page sits on this
/// edge, with its content capped by [`PAGE_CONTENT`] inside it.
pub fn page_inset_class(ui_style: &UiStyle) -> &'static str {
    if cfg!(target_os = "android") {
        return "px-3 pt-3";
    }
    match ui_style {
        UiStyle::Normal => "px-8 pt-8",
        UiStyle::Vaxry | UiStyle::Material3 => "px-6 pt-6",
    }
}

/// The inline part of [`page_inset_class`], for a full-bleed banner whose text still lines up with the page edge.
pub fn page_inline_class(ui_style: &UiStyle) -> &'static str {
    if cfg!(target_os = "android") {
        return "px-3";
    }
    match ui_style {
        UiStyle::Normal => "px-8",
        UiStyle::Vaxry | UiStyle::Material3 => "px-6",
    }
}

/// The width cap every page's content shares, centred within the page inset.
pub const PAGE_CONTENT: &str = "w-full max-w-[1600px] mx-auto";

/// A page that fills the view and scrolls inside its own lists.
pub fn page_container_class(ui_style: &UiStyle) -> &'static str {
    if cfg!(target_os = "android") {
        return "px-3 pt-3 absolute inset-0 flex flex-col overflow-x-hidden";
    }
    match ui_style {
        UiStyle::Normal => "px-8 pt-8 absolute inset-0 flex flex-col",
        UiStyle::Vaxry | UiStyle::Material3 => "px-6 pt-6 absolute inset-0 flex flex-col",
    }
}

/// The active UI style, which the page edge follows.
pub fn use_ui_style() -> UiStyle {
    use_context::<Signal<config::Settings>>().read().ui.ui_style
}
