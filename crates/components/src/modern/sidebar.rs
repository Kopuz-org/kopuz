use config::MusicSource;
use dioxus::prelude::*;
use kopuz_route::Route;

use crate::sidebar::SidebarProps;

#[derive(PartialEq, Clone)]
struct NavItem {
    key: &'static str,
    route: Route,
    icon: &'static str,
}

const SECTIONS: &[(&str, &[NavItem])] = &[
    (
        "discover",
        &[
            NavItem {
                key: "home",
                route: Route::Home,
                icon: "fa-solid fa-house",
            },
            NavItem {
                key: "search",
                route: Route::Search,
                icon: "fa-solid fa-magnifying-glass",
            },
            NavItem {
                key: "discover",
                route: Route::Discover,
                icon: "fa-solid fa-compass",
            },
            NavItem {
                key: "radio",
                route: Route::Radio,
                icon: "fa-solid fa-radio",
            },
        ],
    ),
    (
        "library",
        &[
            NavItem {
                key: "library",
                route: Route::Library,
                icon: "fa-solid fa-music",
            },
            NavItem {
                key: "albums",
                route: Route::Album,
                icon: "fa-solid fa-record-vinyl",
            },
            NavItem {
                key: "artists",
                route: Route::Artist,
                icon: "fa-solid fa-user",
            },
            NavItem {
                key: "favorites",
                route: Route::Favorites,
                icon: "fa-solid fa-heart",
            },
            NavItem {
                key: "activity",
                route: Route::Activity,
                icon: "fa-solid fa-chart-simple",
            },
            NavItem {
                key: "playlists",
                route: Route::Playlists,
                icon: "fa-solid fa-list",
            },
        ],
    ),
];

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
const TOOL_ITEMS: &[NavItem] = &[
    NavItem {
        key: "ytdlp",
        route: Route::Ytdlp,
        icon: "fa-solid fa-download",
    },
    NavItem {
        key: "settings",
        route: Route::Settings,
        icon: "fa-solid fa-gear",
    },
];

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
const TOOL_ITEMS: &[NavItem] = &[NavItem {
    key: "settings",
    route: Route::Settings,
    icon: "fa-solid fa-gear",
}];

#[component]
pub fn SidebarModern(props: SidebarProps) -> Element {
    let mut config = use_context::<Signal<config::AppConfig>>();
    let mut width = use_signal(|| 200i32);
    let mut is_collapsed = use_signal(|| false);
    let mut is_resizing = use_signal(|| false);

    let is_android = cfg!(target_os = "android");
    let fallback_collapse = use_signal(|| true);
    let mut mobile_collapsed = try_consume_context::<crate::sidebar::SidebarCollapsed>()
        .map(|c| c.0)
        .unwrap_or(fallback_collapse);

    let current_width = if *is_collapsed.read() {
        56
    } else {
        *width.read()
    };

    let onmousemove = move |evt: MouseEvent| {
        if *is_resizing.read() {
            let new_width = evt.client_coordinates().x as i32;
            if *is_collapsed.read() {
                if new_width > 160 {
                    is_collapsed.set(false);
                    width.set(new_width);
                }
            } else if new_width < 130 {
                is_collapsed.set(true);
            } else if new_width < 400 {
                width.set(new_width);
            }
        }
    };
    let onmouseup = move |_| is_resizing.set(false);

    let is_server = config.read().active_source == MusicSource::Server;
    let collapsed = if is_android {
        false
    } else {
        *is_collapsed.read()
    };
    let current_route = *props.current_route.read();

    let root_class = if is_android {
        "h-full flex flex-col shrink-0 select-none relative border-r border-white/10 overflow-hidden transition-all duration-300 ease-out"
    } else {
        "h-full flex flex-col shrink-0 select-none relative border-r border-white/5"
    };
    let root_style = if is_android {
        if *mobile_collapsed.read() {
            "position: fixed; left: 0; top: 0; z-index: 100; height: 100%; width: 0px; background: rgba(10,10,10,0.97);".to_string()
        } else {
            "position: fixed; left: 0; top: 0; z-index: 100; height: 100%; width: 280px; background: rgba(10,10,10,0.97);".to_string()
        }
    } else {
        format!("width: {current_width}px; background: rgba(0,0,0,0.5);")
    };

    rsx! {
        if *is_resizing.read() {
            div {
                class: "fixed inset-0 z-[100] cursor-col-resize",
                onmousemove,
                onmouseup,
            }
        }
        if is_android && !*mobile_collapsed.read() {
            div {
                class: "fixed inset-0 bg-black/80 backdrop-blur-[2px] z-[90]",
                onclick: move |_| mobile_collapsed.set(true),
            }
        }

        div {
            class: "{root_class}",
            style: "{root_style}",

            if is_android {
                div {
                    class: "flex items-center justify-between px-5 border-b border-white/5 bg-white/5 shrink-0",
                    style: "padding-top: max(env(safe-area-inset-top), 16px); padding-bottom: 16px;",
                    h2 {
                        class: "text-base font-bold tracking-widest text-white/90 uppercase",
                        style: "font-family: 'JetBrains Mono', monospace;",
                        "KOPUZ"
                    }
                    button {
                        class: "p-2 rounded-xl bg-white/10 text-white active:scale-95 transition-all flex items-center justify-center border border-white/10 w-9 h-9",
                        onclick: move |_| mobile_collapsed.set(true),
                        i { class: "fa-solid fa-xmark text-base" }
                    }
                }
            }

            if cfg!(all(not(target_arch = "wasm32"), target_os = "macos")) {
                div {
                    class: "h-10 flex-shrink-0",
                    onmousedown: move |_| crate::window_chrome::drag(),
                }
            }

            if !cfg!(target_arch = "wasm32") && config.read().show_source_toggle {
                if collapsed {
                    div { class: "flex flex-col items-center gap-1 py-3 border-b border-white/5",
                        button {
                            class: if !is_server { "text-[10px] font-bold py-1" } else { "text-[10px] font-bold py-1 opacity-30" },
                            style: if !is_server { "color: var(--color-indigo-500);" } else { "" },
                            onclick: move |_| { config.write().active_source = MusicSource::Local; config.write().source_explicitly_set = true; },
                            i { class: "fa-solid fa-hard-drive text-xs" }
                        }
                        button {
                            class: if is_server { "text-[10px] font-bold py-1" } else { "text-[10px] font-bold py-1 opacity-30" },
                            style: if is_server { "color: var(--color-indigo-500);" } else { "" },
                            onclick: move |_| { config.write().active_source = MusicSource::Server; config.write().source_explicitly_set = true; },
                            i { class: "fa-solid fa-server text-xs" }
                        }
                    }
                } else {
                    div { class: "px-3 pt-3 pb-2 border-b border-white/5",
                        div { class: "flex rounded-lg overflow-hidden border border-white/10 text-[11px] font-bold",
                            button {
                                class: "flex-1 py-1.5 transition-colors",
                                style: if !is_server { "background: color-mix(in oklab, var(--color-indigo-500) 20%, transparent); color: var(--color-indigo-500);" } else { "color: rgba(255,255,255,0.3);" },
                                onclick: move |_| { config.write().active_source = MusicSource::Local; config.write().source_explicitly_set = true; },
                                "{i18n::t(\"local\").to_uppercase()}"
                            }
                            button {
                                class: "flex-1 py-1.5 transition-colors",
                                style: if is_server { "background: color-mix(in oklab, var(--color-indigo-500) 20%, transparent); color: var(--color-indigo-500);" } else { "color: rgba(255,255,255,0.3);" },
                                onclick: move |_| { config.write().active_source = MusicSource::Server; config.write().source_explicitly_set = true; },
                                "{i18n::t(\"server\").to_uppercase()}"
                            }
                        }
                    }
                }
            }

            div { class: "flex-1 overflow-y-auto overflow-x-hidden py-2",
                for (section_key, items) in SECTIONS {
                    div { class: "mb-2",
                        if !collapsed {
                            div { class: "px-4 pt-3 pb-1",
                                span {
                                    class: "text-[10px] font-bold tracking-widest uppercase",
                                    style: "color: rgba(255,255,255,0.25);",
                                    "{i18n::t(section_key)}"
                                }
                            }
                        }
                        for item in *items {
                            ModernNavItem {
                                key: "{item.key}",
                                item: item.clone(),
                                active: current_route == item.route,
                                collapsed,
                                onclick: move |_| {
                                    props.on_navigate.call(item.route);
                                    if is_android { mobile_collapsed.set(true); }
                                },
                            }
                        }
                    }
                }

                div { class: "mx-3 my-2 h-px", style: "background: rgba(255,255,255,0.06);" }
                for item in TOOL_ITEMS {
                    ModernNavItem {
                        key: "{item.key}",
                        item: item.clone(),
                        active: current_route == item.route,
                        collapsed,
                        onclick: move |_| {
                            props.on_navigate.call(item.route);
                            if is_android { mobile_collapsed.set(true); }
                        },
                    }
                }
            }

            div {
                class: "absolute top-0 right-0 w-2 h-full cursor-col-resize z-50",
                onmousedown: move |_| is_resizing.set(true),
            }
        }
    }
}

#[component]
fn ModernNavItem(
    item: NavItem,
    active: bool,
    collapsed: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        a {
            class: "flex items-center gap-3 cursor-pointer transition-colors relative mx-1 rounded-lg",
            style: if active {
                "padding: 6px 10px; background: color-mix(in oklab, var(--color-indigo-500) 15%, transparent);"
            } else {
                "padding: 6px 10px;"
            },
            title: if collapsed { i18n::t(item.key) } else { String::new() },
            onclick: move |evt| onclick.call(evt),

            div {
                class: "w-5 h-5 flex items-center justify-center shrink-0 text-sm",
                style: if active {
                    "color: var(--color-indigo-500);"
                } else {
                    "color: rgba(255,255,255,0.4);"
                },
                i { class: "{item.icon}" }
            }

            if !collapsed {
                span {
                    class: "text-sm font-medium truncate",
                    style: if active {
                        "color: var(--color-indigo-500); font-weight: 600;"
                    } else {
                        "color: rgba(255,255,255,0.7);"
                    },
                    "{i18n::t(item.key)}"
                }
            }
        }
    }
}
