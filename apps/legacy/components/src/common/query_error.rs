use dioxus::prelude::*;

#[component]
pub fn QueryError(message: String, onretry: EventHandler<()>) -> Element {
    rsx! {
        div { role: "alert", class: "flex items-center gap-3 p-4 text-white/80",
            span { "{message}" }
            button {
                r#type: "button",
                class: "app-button-tonal px-3 py-1.5 rounded-lg bg-white/10 hover:bg-white/20 active:bg-white/25 text-sm text-white transition-colors",
                onclick: move |_| onretry.call(()),
                "{i18n::t(\"retry_now\")}"
            }
        }
    }
}
