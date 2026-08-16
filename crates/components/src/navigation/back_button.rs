use dioxus::prelude::*;

#[component]
pub fn BackButton(on_click: EventHandler<()>, #[props(default)] class: Option<String>) -> Element {
    let layout = class.unwrap_or_else(|| "mb-6 shrink-0 self-start".to_string());
    rsx! {
        button {
            class: "flex items-center justify-center text-slate-400 hover:text-white transition-colors group {layout}",
            title: "Back",
            "aria-label": "Back",
            onclick: move |_| on_click.call(()),
            i { class: "fa-solid fa-arrow-left text-base group-hover:-translate-x-0.5 transition-transform" }
        }
    }
}
