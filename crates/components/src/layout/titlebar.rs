use config::AppConfig;
use dioxus::desktop::window;
use dioxus::prelude::*;

#[cfg(target_os = "linux")]
const RESIZE_HANDLES: [(&str, dioxus::desktop::tao::window::ResizeDirection); 8] = {
    use dioxus::desktop::tao::window::ResizeDirection as D;
    [
        ("top:0;left:0;right:0;height:5px;cursor:n-resize;", D::North),
        (
            "bottom:0;left:0;right:0;height:5px;cursor:s-resize;",
            D::South,
        ),
        ("top:0;bottom:0;left:0;width:5px;cursor:w-resize;", D::West),
        ("top:0;bottom:0;right:0;width:5px;cursor:e-resize;", D::East),
        (
            "top:0;left:0;width:12px;height:12px;cursor:nw-resize;",
            D::NorthWest,
        ),
        (
            "top:0;right:0;width:12px;height:12px;cursor:ne-resize;",
            D::NorthEast,
        ),
        (
            "bottom:0;left:0;width:12px;height:12px;cursor:sw-resize;",
            D::SouthWest,
        ),
        (
            "bottom:0;right:0;width:12px;height:12px;cursor:se-resize;",
            D::SouthEast,
        ),
    ]
};

#[component]
pub fn ResizeHandles() -> Element {
    #[cfg(target_os = "linux")]
    {
        let config = use_context::<Signal<AppConfig>>();
        if config.read().titlebar_mode == config::TitlebarMode::System {
            return rsx! {};
        }

        return rsx! {
            for (placement , direction) in RESIZE_HANDLES {
                div {
                    key: "{placement}",
                    style: "position:fixed;z-index:999;{placement}",
                    onmousedown: move |evt| {
                        if evt.trigger_button() != Some(dioxus::html::input_data::MouseButton::Primary) {
                            return;
                        }
                        evt.stop_propagation();
                        let win = window();
                        if win.window.is_maximized() || win.window.fullscreen().is_some() {
                            return;
                        }
                        if let Err(err) = win.window.drag_resize_window(direction) {
                            tracing::warn!(?direction, %err, "window resize drag failed");
                        }
                    },
                }
            }
        };
    }

    #[cfg(not(target_os = "linux"))]
    rsx! {}
}

#[component]
pub fn Titlebar() -> Element {
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
                    window().drag();
                },

                div { class: "flex-1" }

                div {
                    class: "absolute inset-0 flex items-center justify-center pointer-events-none",
                    span {
                        class: "text-[11px] text-white/35 font-mono",
                        "Kopuz"
                    }
                }

                div {
                    class: "flex items-center h-full",
                    onmousedown: move |evt| evt.stop_propagation(),

                    button {
                        class: "w-11 h-full flex items-center justify-center text-white/25 hover:text-white/70 hover:bg-white/6 transition-all duration-150",
                        title: "{minimize_text}",
                        onclick: move |_| window().window.set_minimized(true),
                        i { class: "fa-solid fa-minus text-[10px] leading-none" }
                    }
                    button {
                        class: "w-11 h-full flex items-center justify-center text-white/25 hover:text-white/70 hover:bg-white/6 transition-all duration-150",
                        title: "{maximize_text}",
                        onclick: move |_| window().toggle_maximized(),
                        i { class: "fa-regular fa-square text-[10px] leading-none" }
                    }
                    button {
                        class: "w-11 h-full flex items-center justify-center text-white/25 hover:text-white hover:bg-red-500/70 transition-all duration-150",
                        title: "{close_text}",
                        onclick: move |_| window().close(),
                        i { class: "fa-solid fa-xmark text-[10px] leading-none" }
                    }
                }
            }
        }
    }
}
