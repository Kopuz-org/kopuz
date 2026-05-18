use dioxus::prelude::*;

#[component]
pub fn UnsupportedServerView(service_name: String, feature: &'static str) -> Element {
    let unsupported_msg = i18n::t_with("unsupported_provider", &[("service", service_name.clone())]);
    let unsupported_desc = i18n::t_with("unsupported_provider_desc", &[("service", service_name.clone())]);

    rsx! {
        div {
            class: "p-8",
            div {
                class: "max-w-2xl rounded-2xl border border-white/10 bg-white/5 p-6",
                h2 { class: "text-2xl font-bold text-white mb-2", "{feature}" }
                p { class: "text-slate-300", "{unsupported_msg}" }
                p { class: "text-slate-400 mt-2", "{unsupported_desc}" }
            }
        }
    }
}
