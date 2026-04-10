use dioxus::prelude::*;

#[component]
pub fn UnsupportedServerView(service_name: String, feature: &'static str) -> Element {
    rsx! {
        div {
            class: "p-8",
            div {
                class: "max-w-2xl rounded-2xl border border-white/10 bg-white/5 p-6",
                h2 { class: "text-2xl font-bold text-white mb-2", "{feature}" }
                p { class: "text-slate-300", "{service_name} is configured, but this page is not available yet for that provider." }
                p { class: "text-slate-400 mt-2", "Provider-specific browsing for {service_name} will be added through the new server abstraction." }
            }
        }
    }
}
