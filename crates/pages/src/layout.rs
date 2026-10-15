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

pub fn page_container_class(ui_style: &UiStyle) -> &'static str {
    if cfg!(target_os = "android") {
        "px-3 pt-3 absolute inset-0 flex flex-col overflow-x-hidden"
    } else {
        match ui_style {
            UiStyle::Vaxry => "px-6 pt-6 absolute inset-0 flex flex-col",
            UiStyle::Normal => "px-8 pt-8 absolute inset-0 flex flex-col",
            UiStyle::Material3 => "px-6 pt-6 absolute inset-0 flex flex-col",
        }
    }
}
