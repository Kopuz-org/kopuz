use super::actions::TrackActions;
use crate::NavigationController;
use dioxus::prelude::*;
use hooks::favorites::toggle_favorite;
use hooks::use_player_controller::PlayerController;

#[component]
pub(crate) fn TrackMetadata(
    mut is_fullscreen: Signal<bool>,
    current_song_cover_url: Signal<String>,
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_album: Signal<String>,
    current_song_bitrate: Signal<u16>,
) -> Element {
    let ctrl = use_context::<PlayerController>();
    let mut track_menu_open = use_signal(|| false);
    let nav_ctrl = use_context::<NavigationController>();
    let favorite_track = use_memo(move || ctrl.current_track_snapshot.read().clone());
    let is_favorite = hooks::use_db_queries::use_track_is_favorite(favorite_track)();
    let current_track_snapshot = ctrl.current_track_snapshot.read().clone();
    let actions_track = current_track_snapshot.clone();
    let favorite_label = if is_favorite {
        i18n::t("remove_from_favorites").to_string()
    } else {
        i18n::t("add_to_favorites").to_string()
    };

    rsx! {
        div {
            class: "flex-1 min-h-0 w-full flex items-center justify-center mb-6",
            {
                let cover = current_song_cover_url.read().clone();
                if cover.is_empty() {
                    rsx! {
                        div {
                            class: "rounded-xl overflow-hidden h-full flex items-center justify-center bg-black/30",
                            style: "max-width: 100%; aspect-ratio: 1/1; box-shadow: 0 25px 60px -15px rgba(0,0,0,0.55);",
                            i { class: "fa-solid fa-music text-5xl text-white/20" }
                        }
                    }
                } else {
                    let cover = crate::cover_background::high_quality_artwork_url(cover);
                    rsx! {
                        img {
                            src: "{cover}",
                            class: "rounded-xl",
                            style: "max-width: 100%; max-height: 100%; width: auto; height: auto; box-shadow: 0 25px 60px -15px rgba(0,0,0,0.55);",
                        }
                    }
                }
            }
        }

        div {
            class: "flex items-center gap-4 w-full mb-1",
            style: "max-width: 640px;",
            oncontextmenu: move |evt| {
                evt.prevent_default();
                crate::dots_menu::open_at_pointer(&evt);
                track_menu_open.set(true);
            },
            div {
                class: "flex flex-col items-start min-w-0 flex-1",
                h1 { class: "text-[28px] font-semibold tracking-tight text-white mb-1 line-clamp-2 w-full", "{current_song_title}" }
                div {
                    class: "flex flex-wrap items-center gap-x-2 gap-y-1 w-full",
                    button {
                        class: "text-xl text-white/70 font-medium line-clamp-2 max-w-full hover:text-white hover:underline text-left transition-colors",
                        onclick: move |_| {
                            let artist = current_song_artist.read().clone();
                            if artist.is_empty() {
                                return;
                            }
                            is_fullscreen.set(false);
                            nav_ctrl.navigate_to_artist(artist);
                        },
                        "{current_song_artist}"
                    }
                    span { class: "text-white/30 flex-shrink-0", "•" }
                    button {
                        class: "text-lg text-white/50 line-clamp-2 max-w-full hover:text-white/80 hover:underline text-left transition-colors",
                        onclick: move |_| {
                            let album_id = current_track_snapshot
                                .as_ref()
                                .map(|track| track.album_id.clone())
                                .unwrap_or_default();
                            if album_id.is_empty() {
                                return;
                            }
                            is_fullscreen.set(false);
                            nav_ctrl.navigate_to_album(album_id);
                        },
                        "{current_song_album}"
                    }
                }
            }
            div {
                class: "flex items-center gap-2 flex-shrink-0",
                button {
                    class: if is_favorite {
                        "w-11 h-11 rounded-full flex-shrink-0 flex items-center justify-center bg-white/10 text-red-400 hover:bg-white/15 hover:text-red-300 transition-colors active:scale-95"
                    } else {
                        "w-11 h-11 rounded-full flex-shrink-0 flex items-center justify-center bg-white/10 text-white/70 hover:bg-white/15 hover:text-red-400 transition-colors active:scale-95"
                    },
                    title: "{favorite_label}",
                    "aria-label": "{favorite_label}",
                    onclick: move |_| toggle_favorite(ctrl.current_track_snapshot.read().clone()),
                    i {
                        class: if is_favorite { "fa-solid fa-heart" } else { "fa-regular fa-heart" },
                        "aria-hidden": "true",
                    }
                }
                if let Some(track) = actions_track {
                    TrackActions { track, menu_open: track_menu_open }
                }
            }
        }

        div {
            class: "flex items-center gap-4 text-xs text-white/60 mb-3 w-full",
            style: "max-width: 640px;",
            if current_song_bitrate() > 0 {
                span { style: "font-size: 10px;", "{current_song_bitrate} kbps" }
            }
        }
    }
}
