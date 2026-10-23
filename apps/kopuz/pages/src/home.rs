use config::{Settings, UiStyle};
use dioxus::prelude::*;

use crate::home_body::{HomeBody, has_catalog};

#[component]
pub fn Home(
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_select_catalog: EventHandler<(api::CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
) -> Element {
    let settings = use_context::<Signal<Settings>>();
    let mut prefs = hooks::ui_prefs::use_ui_prefs();
    let is_vaxry = settings.read().ui.ui_style == UiStyle::Vaxry;
    let mut edit_mode = use_signal(|| false);
    let caps = hooks::sources::use_capabilities();
    let catalog = has_catalog(&caps.read());

    rsx! {
        div { class: "{crate::layout::page_inset_class(&settings.read().ui.ui_style)} w-full",
        div {
            class: if cfg!(target_os = "android") {
                "{crate::layout::PAGE_CONTENT} pb-6 space-y-8"
            } else if is_vaxry {
                "{crate::layout::PAGE_CONTENT} pb-24"
            } else {
                "{crate::layout::PAGE_CONTENT} space-y-12 pb-32 animate-fade-in"
            },

            div { class: "flex items-center justify-between mb-4",
                if !is_vaxry {
                    h1 { class: "text-4xl font-black text-white tracking-tight", "{i18n::t(\"home\")}" }
                } else {
                    div {}
                }
                div { class: "flex items-center gap-2",
                    if *edit_mode.read() {
                        button {
                            class: "app-button-outlined px-3 h-8 rounded-full bg-white/5 hover:bg-white/10 border border-white/10 text-white/70 hover:text-white text-xs font-semibold transition-colors",
                            onclick: move |_| {
                                *prefs.write().home_layout_mut(catalog) = if catalog {
                                    config::default_catalog_home_sections()
                                } else {
                                    config::default_home_sections()
                                };
                            },
                            i { class: "fa-solid fa-rotate-left mr-1 text-[10px]" }
                            "{i18n::t(\"reset_home\")}"
                        }
                        button {
                            class: "app-button-filled px-4 h-8 rounded-full bg-indigo-500 hover:bg-indigo-400 text-white text-xs font-bold transition-colors",
                            onclick: move |_| edit_mode.set(false),
                            i { class: "fa-solid fa-check mr-1 text-[10px]" }
                            "{i18n::t(\"done_editing\")}"
                        }
                    } else {
                        button {
                            class: "app-button-outlined px-3 h-8 rounded-full bg-white/5 hover:bg-white/10 border border-white/10 text-white/70 hover:text-white text-xs font-semibold transition-colors flex items-center gap-2",
                            onclick: move |_| edit_mode.set(true),
                            i { class: "fa-solid fa-sliders text-[10px]" }
                            "{i18n::t(\"customize_home\")}"
                        }
                    }
                }
            }

            HomeBody {
                edit_mode,
                on_select_album,
                on_play_album,
                on_select_playlist,
                on_select_catalog,
                on_open_artist,
            }
        }
        }
    }
}
