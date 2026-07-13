//! Grid ⇄ list switch for album listings, styled to sit beside
//! [`sort_control::SortControl`](crate::sort_control) in a page header.

use config::AlbumViewMode;
use dioxus::prelude::*;

#[component]
pub fn ViewModeToggle(mut mode: Signal<AlbumViewMode>) -> Element {
    let is_grid = *mode.read() == AlbumViewMode::Grid;

    let btn_active =
        "w-7 h-6 flex items-center justify-center rounded-md bg-white/10 text-white transition-all";
    let btn_inactive = "w-7 h-6 flex items-center justify-center rounded-md text-white/40 hover:text-white/80 transition-all";

    rsx! {
        div { class: "flex space-x-0.5 bg-white/5 border border-white/5 p-0.5 rounded-lg",
            button {
                class: if is_grid { btn_active } else { btn_inactive },
                title: "{i18n::t(\"view_grid\")}",
                aria_label: "{i18n::t(\"view_grid\")}",
                onclick: move |_| mode.set(AlbumViewMode::Grid),
                i { class: "fa-solid fa-grip", style: "font-size: 11px;" }
            }
            button {
                class: if !is_grid { btn_active } else { btn_inactive },
                title: "{i18n::t(\"view_list\")}",
                aria_label: "{i18n::t(\"view_list\")}",
                onclick: move |_| mode.set(AlbumViewMode::List),
                i { class: "fa-solid fa-list", style: "font-size: 11px;" }
            }
        }
    }
}
