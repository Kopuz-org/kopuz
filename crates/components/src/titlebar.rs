#[cfg(not(target_arch = "wasm32"))]
use config::AppConfig;
use dioxus::prelude::*;

/// The desktop window handle, or `None` under the blitz/native renderer
/// (no webview, no DesktopContext) — window chrome no-ops there.
#[cfg(not(target_arch = "wasm32"))]
fn desktop_window() -> Option<dioxus::desktop::DesktopContext> {
    try_consume_context::<dioxus::desktop::DesktopContext>()
}

#[component]
pub fn Titlebar() -> Element {
    #[cfg(target_arch = "wasm32")]
    {
        return rsx! {};
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let config = use_context::<Signal<AppConfig>>();
        if config.read().titlebar_mode != config::TitlebarMode::Custom || desktop_window().is_none()
        {
            return rsx! {};
        }
        let minimize_text = i18n::t("minimize").to_string();
        let maximize_text = i18n::t("maximize").to_string();
        let close_text = i18n::t("close").to_string();

        rsx! {
            div {
                class: "flex items-center h-9 bg-black/50 border-b border-white/5 flex-shrink-0 select-none relative",
                onmousedown: move |_| {
                    if let Some(w) = desktop_window() {
                        w.drag();
                    }
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
                        onclick: move |_| if let Some(w) = desktop_window() { w.window.set_minimized(true) },
                        i { class: "fa-solid fa-minus text-[10px] leading-none" }
                    }
                    button {
                        class: "w-11 h-full flex items-center justify-center text-white/25 hover:text-white/70 hover:bg-white/6 transition-all duration-150",
                        title: "{maximize_text}",
                        onclick: move |_| if let Some(w) = desktop_window() { w.toggle_maximized() },
                        i { class: "fa-regular fa-square text-[10px] leading-none" }
                    }
                    button {
                        class: "w-11 h-full flex items-center justify-center text-white/25 hover:text-white hover:bg-red-500/70 transition-all duration-150",
                        title: "{close_text}",
                        onclick: move |_| if let Some(w) = desktop_window() { w.close() },
                        i { class: "fa-solid fa-xmark text-[10px] leading-none" }
                    }
                }
            }
        }
    }
}
