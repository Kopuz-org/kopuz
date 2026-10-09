use dioxus::prelude::*;

#[component]
pub fn QueryError(message: String, onretry: EventHandler<()>) -> Element {
    rsx! {
        div { role: "alert", class: "flex items-center gap-3 p-4 text-white/80",
            span { "{message}" }
            button { onclick: move |_| onretry.call(()), "{i18n::t(\"retry_now\")}" }
        }
    }
}
