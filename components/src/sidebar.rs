use config::MusicSource;
#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
use dioxus::desktop::use_window;
use dioxus::prelude::*;
use kopuz_route::Route;

#[derive(PartialEq, Clone)]
struct SidebarItem {
    key: &'static str,
    route: Route,
    icon: &'static str,
}

const TOP_MENU: &[SidebarItem] = &[
    SidebarItem { key: "home",      route: Route::Home,      icon: "fa-solid fa-house" },
    SidebarItem { key: "search",    route: Route::Search,    icon: "fa-solid fa-magnifying-glass" },
    SidebarItem { key: "library",   route: Route::Library,   icon: "fa-solid fa-book" },
    SidebarItem { key: "albums",    route: Route::Album,     icon: "fa-solid fa-music" },
    SidebarItem { key: "artists",   route: Route::Artist,    icon: "fa-solid fa-user" },
    SidebarItem { key: "playlists", route: Route::Playlists, icon: "fa-solid fa-list" },
    SidebarItem { key: "favorites", route: Route::Favorites, icon: "fa-solid fa-heart" },
    SidebarItem { key: "Activity",  route: Route::Activity,  icon: "fa-solid fa-chart-simple" },
];

const BOTTOM_MENU: &[SidebarItem] = &[SidebarItem {
    key: "settings",
    route: Route::Settings,
    icon: "fa-solid fa-gear",
}];

#[derive(Props, Clone, PartialEq)]
pub struct SidebarProps {
    current_route: Signal<Route>,
    on_navigate: EventHandler<Route>,
}

#[component]
pub fn Sidebar(props: SidebarProps) -> Element {
    let mut config = use_context::<Signal<config::AppConfig>>();
    let mut width = use_signal(|| 240);
    let mut is_collapsed = use_signal(|| false);
    let mut is_resizing = use_signal(|| false);

    let current_width = if *is_collapsed.read() { 72 } else { *width.read() };

    let onmousemove = move |evt: MouseEvent| {
        if *is_resizing.read() {
            let new_width = evt.client_coordinates().x as i32;
            if *is_collapsed.read() {
                if new_width > 180 {
                    is_collapsed.set(false);
                    width.set(new_width);
                }
            } else if new_width < 150 {
                is_collapsed.set(true);
            } else if new_width < 450 {
                width.set(new_width);
            }
        }
    };

    let onmouseup = move |_| is_resizing.set(false);


    let extra_padding = if cfg!(target_os = "macos") { "pt-10" } else { "" };

    let is_server = config.read().active_source == MusicSource::Server;
    let local_class  = if !is_server { "text-white" } else { "text-slate-500 hover:text-slate-300" };
    let server_class = if  is_server { "text-white" } else { "text-slate-500 hover:text-slate-300" };
    let slider_style = if is_server {
        "left: calc(50% + 2px); width: calc(50% - 4px);"
    } else {
        "left: 4px; width: calc(50% - 4px);"
    };

    // Build ordered item list from saved config, appending any items not yet in the saved order
    let ordered_items: Vec<SidebarItem> = {
        let order = config.read().sidebar_order.clone();
        let mut items: Vec<SidebarItem> = order
            .iter()
            .filter_map(|key| TOP_MENU.iter().find(|item| item.key == key).cloned())
            .collect();
        for item in TOP_MENU {
            if !order.iter().any(|k| k == item.key) {
                items.push(item.clone());
            }
        }
        items
    };

    let item_count = ordered_items.len();

    rsx! {
        if *is_resizing.read() {
            div {
                class: "fixed inset-0 z-[100] cursor-col-resize",
                onmousemove: onmousemove,
                onmouseup: onmouseup,
            }
        }

        div {
            class: "h-full bg-black/40 text-slate-400 flex flex-col flex-shrink-0 select-none relative border-r border-white/5 {extra_padding}",
            style: "width: {current_width}px",

            if cfg!(all(not(target_arch = "wasm32"), target_os = "macos")) {
                div {
                    class: "absolute top-0 left-0 w-full h-10 z-50",
                    onmousedown: move |_| {
                        #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
                        use_window().drag();
                    }
                }
            }

            div {
                class: "flex-1 flex flex-col overflow-y-auto overflow-x-hidden pt-2",

                if !*is_collapsed.read() && !cfg!(target_arch = "wasm32") && config.read().show_source_toggle {
                    div {
                        class: "px-4 mb-6",
                        div {
                            class: "bg-white/5 p-1 rounded-xl flex relative h-10 items-center border border-white/5",
                            div {
                                class: "absolute h-8 bg-white/10 rounded-lg transition-all duration-300 ease-out",
                                style: "{slider_style}"
                            }
                            button {
                                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 {local_class}",
                                onclick: move |_| {
                                    let mut cfg = config.write();
                                    cfg.active_source = MusicSource::Local;
                                    cfg.source_explicitly_set = true;
                                },
                                "{rust_i18n::t!(\"local\").to_uppercase()}"
                            }
                            button {
                                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 {server_class}",
                                onclick: move |_| {
                                    let mut cfg = config.write();
                                    cfg.active_source = MusicSource::Server;
                                    cfg.source_explicitly_set = true;
                                },
                                "{rust_i18n::t!(\"server\").to_uppercase()}"
                            }
                        }
                    }
                }

                nav {
                    class: "flex-1 px-3 space-y-1",
                    for (idx, item) in ordered_items.into_iter().enumerate() {
                        SidebarLink {
                            key: "{item.key}",
                            item: item.clone(),
                            collapsed: is_collapsed,
                            active: *props.current_route.read() == item.route,
                            can_move_up: idx > 0,
                            can_move_down: idx < item_count - 1,
                            onclick: move |_| props.on_navigate.call(item.route),
                            on_move_up: move |_| {
                                let mut order = config.peek().sidebar_order.clone();
                                if idx > 0 {
                                    order.swap(idx, idx - 1);
                                    config.write().sidebar_order = order;
                                }
                            },
                            on_move_down: move |_| {
                                let mut order = config.peek().sidebar_order.clone();
                                if idx + 1 < order.len() {
                                    order.swap(idx, idx + 1);
                                    config.write().sidebar_order = order;
                                }
                            },
                        }
                    }
                    div { class: "h-px bg-white/5 my-4 mx-3" }
                    for item in BOTTOM_MENU {
                        SidebarLink {
                            item: item.clone(),
                            collapsed: is_collapsed,
                            active: *props.current_route.read() == item.route,
                            can_move_up: false,
                            can_move_down: false,
                            onclick: move |_| props.on_navigate.call(item.route),
                            on_move_up: move |_| {},
                            on_move_down: move |_| {},
                        }
                    }
                }
            }

            div {
                class: "absolute top-0 right-0 w-2 h-full cursor-col-resize group/handle z-50",
                onmousedown: move |_| is_resizing.set(true),
                div { class: "absolute inset-y-0 right-0 w-px bg-white/0 group-hover/handle:bg-white/10 transition-colors" }
            }
        }
    }
}

#[component]
fn SidebarLink(
    item: SidebarItem,
    collapsed: Signal<bool>,
    active: bool,
    can_move_up: bool,
    can_move_down: bool,
    onclick: EventHandler<MouseEvent>,
    on_move_up: EventHandler<()>,
    on_move_down: EventHandler<()>,
) -> Element {
    let is_collapsed = *collapsed.read();
    let alignment_class = if is_collapsed { "justify-center" } else { "justify-start px-3" };

    let active_class = if active {
        "bg-white/10 text-white"
    } else {
        "text-slate-400 hover:text-white/90 hover:bg-white/5"
    };

    let opacity_class = if active { "opacity-100" } else { "opacity-70 group-hover:opacity-100" };

    rsx! {
        div { class: "flex items-center group",
            a {
                class: "flex flex-1 items-center {alignment_class} relative p-3 rounded-lg transition-all duration-200 cursor-pointer {active_class}",
                title: if is_collapsed { rust_i18n::t!(item.key).to_string() } else { String::new() },
                onclick: move |evt| onclick.call(evt),

                div {
                    class: "flex items-center justify-center w-6 h-6 shrink-0 transition-transform group-active:scale-95",
                    i { class: "{item.icon} text-lg" }
                }

                if !is_collapsed {
                    span {
                        class: "ml-4 text-sm font-medium tracking-tight {opacity_class} transition-opacity",
                        "{rust_i18n::t!(item.key)}"
                    }
                }

                div {
                    class: if active {
                        "absolute left-0 w-0.5 rounded-r-full transition-all duration-300 h-6 bg-white"
                    } else {
                        "absolute left-0 w-0.5 rounded-r-full transition-all duration-300 h-0 bg-white/40 group-hover:h-4"
                    }
                }
            }

            if !is_collapsed && (can_move_up || can_move_down) {
                div { class: "flex flex-col opacity-0 group-hover:opacity-100 transition-opacity pr-1",
                    button {
                        class: if can_move_up {
                            "text-slate-500 hover:text-white transition-colors leading-none px-1"
                        } else {
                            "text-slate-700 cursor-default leading-none px-1"
                        },
                        onclick: move |evt| {
                            evt.stop_propagation();
                            if can_move_up { on_move_up.call(()); }
                        },
                        i { class: "fa-solid fa-chevron-up text-[9px]" }
                    }
                    button {
                        class: if can_move_down {
                            "text-slate-500 hover:text-white transition-colors leading-none px-1"
                        } else {
                            "text-slate-700 cursor-default leading-none px-1"
                        },
                        onclick: move |evt| {
                            evt.stop_propagation();
                            if can_move_down { on_move_down.call(()); }
                        },
                        i { class: "fa-solid fa-chevron-down text-[9px]" }
                    }
                }
            }
        }
    }
}
