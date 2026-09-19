//! Android primary navigation.
//!
//! A phone reaches Home, Search, Library and Settings constantly, and behind
//! the drawer each of those costs a tap on a top-corner target plus a scan of
//! an eleven-row overlay. These four sit in the thumb zone instead; the drawer
//! keeps the browse-by routes and the source switcher.

use dioxus::prelude::*;
use kopuz_route::Route;

struct Tab {
    key: &'static str,
    route: Route,
    icon: &'static str,
}

const TABS: &[Tab] = &[
    Tab {
        key: "home",
        route: Route::Home,
        icon: "fa-solid fa-house",
    },
    Tab {
        key: "search",
        route: Route::Search,
        icon: "fa-solid fa-magnifying-glass",
    },
    Tab {
        key: "library",
        route: Route::Library,
        icon: "fa-solid fa-book",
    },
    Tab {
        key: "settings",
        route: Route::Settings,
        icon: "fa-solid fa-gear",
    },
];

/// Whether the tab bar already offers this route. The drawer drops these on
/// Android so the two bars never list the same destination twice.
pub fn is_tab_route(route: Route) -> bool {
    TABS.iter().any(|tab| tab.route == route)
}

/// Which tab a route belongs under. The browse-by routes are reached *through*
/// Library, so they keep that tab lit instead of leaving the bar with nothing
/// selected and the user unsure where they are.
fn owning_tab(route: Route) -> Option<Route> {
    match route {
        Route::Home | Route::Discover | Route::DiscoverPlaylist => Some(Route::Home),
        Route::Search => Some(Route::Search),
        Route::Library
        | Route::Album
        | Route::Artist
        | Route::Playlists
        | Route::Favorites
        | Route::Activity
        | Route::Radio => Some(Route::Library),
        Route::Settings => Some(Route::Settings),
        _ => None,
    }
}

#[component]
pub fn TabBar(current_route: Signal<Route>, on_navigate: EventHandler<Route>) -> Element {
    let active = owning_tab(*current_route.read());

    rsx! {
        nav {
            class: "shrink-0 flex items-stretch bg-[#0a0a0a]/95 backdrop-blur-3xl border-t border-white/10 pb-[env(safe-area-inset-bottom)]",
            dir: "ltr",
            for tab in TABS {
                {
                    let is_active = active == Some(tab.route);
                    let label = i18n::t(tab.key);
                    let item_class = if is_active {
                        "flex-1 h-14 flex flex-col items-center justify-center gap-1 text-white active:scale-95 transition-transform"
                    } else {
                        "flex-1 h-14 flex flex-col items-center justify-center gap-1 text-white/45 active:scale-95 transition-transform"
                    };
                    rsx! {
                        button {
                            key: "{tab.key}",
                            class: "{item_class}",
                            "aria-label": "{label}",
                            "aria-current": if is_active { "page" } else { "false" },
                            onclick: move |_| on_navigate.call(tab.route),
                            i { class: "{tab.icon} text-[17px] leading-none", "aria-hidden": "true" }
                            span {
                                class: if is_active {
                                    "text-[10px] font-semibold leading-none tracking-tight"
                                } else {
                                    "text-[10px] font-medium leading-none tracking-tight"
                                },
                                "{label}"
                            }
                        }
                    }
                }
            }
        }
    }
}
