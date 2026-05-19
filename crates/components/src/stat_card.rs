use dioxus::prelude::*;

#[component]
pub fn StatCard(label: String, value: String, icon: &'static str) -> Element {
    rsx! {
        div {
            class: " border border-white/5 p-5 rounded-xl flex items-center space-x-4",
            div {
                class: "w-12 h-12 rounded-lg bg-white/5 flex items-center justify-center shrink-0",
                i { class: "fa-solid {icon} text-lg text-white/60" }
            }
            div {
                p { class: "text-xs font-medium text-slate-500 uppercase tracking-wider", "{label}" }
                p { class: "text-2xl font-bold text-white", "{value}" }
            }
        }
    }
}
