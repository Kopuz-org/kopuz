use dioxus::prelude::*;

#[component]
pub fn StatCard(label: String, value: String, icon: &'static str) -> Element {
    if cfg!(target_os = "android") {
        return rsx! {
            div {
                class: "border border-white/5 px-1.5 py-2 rounded-lg flex flex-col items-center justify-center gap-0.5 min-w-0",
                i { class: "fa-solid {icon} text-xs text-white/50" }
                p { class: "text-base font-bold text-white leading-none", "{value}" }
                p {
                    class: "text-[9px] font-medium text-slate-500 uppercase tracking-wide truncate max-w-full",
                    "{label}"
                }
            }
        };
    }
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
