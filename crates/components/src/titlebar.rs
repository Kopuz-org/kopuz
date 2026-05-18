#[cfg(not(target_arch = "wasm32"))]
use dioxus::desktop::use_window;
use dioxus::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use config::AppConfig;

#[component]
pub fn Titlebar() -> Element {
    #[cfg(target_arch = "wasm32")]
    {
        return rsx! {};
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let config = use_context::<Signal<AppConfig>>();
        if config.read().titlebar_mode != config::TitlebarMode::Custom {
            return rsx! {};
        }
        let minimize_text = i18n::t("minimize").to_string();
        let maximize_text = i18n::t("maximize").to_string();
        let close_text = i18n::t("close").to_string();

        rsx! {
        div {
            class: "flex items-center h-9 bg-black/50 border-b border-white/5 flex-shrink-0 select-none relative",
            onmousedown: move |_| {
                use_window().drag();
            },

            div { class: "flex-1" }

            div {
                class: "absolute inset-0 flex items-center justify-center pointer-events-none",
                span {
                    class: "text-[11px] text-white/35 tracking-[0.2em] font-mono uppercase",
                    "Kopuz"
                }
            }

            div {
                class: "flex items-center h-full",
                onmousedown: move |evt| evt.stop_propagation(),

                button {
                    class: "w-11 h-full flex items-center justify-center text-white/25 hover:text-white/70 hover:bg-white/6 transition-all duration-150",
                    title: "{minimize_text}",
                    onclick: move |_| use_window().window.set_minimized(true),
                    i { class: "fa-solid fa-minus text-[10px] leading-none" }
                }
                button {
                    class: "w-11 h-full flex items-center justify-center text-white/25 hover:text-white/70 hover:bg-white/6 transition-all duration-150",
                    title: "{maximize_text}",
                    onclick: move |_| use_window().toggle_maximized(),
                    i { class: "fa-regular fa-square text-[10px] leading-none" }
                }
                button {
                    class: "w-11 h-full flex items-center justify-center text-white/25 hover:text-white hover:bg-red-500/70 transition-all duration-150",
                    title: "{close_text}",
                    onclick: move |_| use_window().close(),
                    i { class: "fa-solid fa-xmark text-[10px] leading-none" }
                }
            }
        }
    }
    }
}
