use crate::NavigationController;
use dioxus::prelude::*;
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
    let nav_ctrl = use_context::<NavigationController>();
    let current_track_snapshot = ctrl.current_track_snapshot.read().clone();

    rsx! {
        div {
            class: "flex-1 min-h-0 w-full flex items-center justify-center mb-6",
            {
                let cover = current_song_cover_url.read();
                if cover.is_empty() {
                    rsx! {
                        div {
                            class: "rounded-xl overflow-hidden h-full flex items-center justify-center bg-black/30",
                            style: "max-width: 100%; aspect-ratio: 1/1; box-shadow: 0 25px 60px -15px rgba(0,0,0,0.55);",
                            i { class: "fa-solid fa-music text-5xl text-white/20" }
                        }
                    }
                } else {
                    let src = if cover.starts_with("artwork://") {
                        format!("{}&hq=1", cover)
                    } else {
                        cover.clone()
                    };
                    rsx! {
                        img {
                            src: "{src}",
                            class: "rounded-xl",
                            style: "max-width: 100%; max-height: 100%; width: auto; height: auto; box-shadow: 0 25px 60px -15px rgba(0,0,0,0.55);",
                        }
                    }
                }
            }
        }

        div {
            class: "flex flex-col items-start w-full mb-1",
            style: "max-width: 640px;",
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
            class: "flex items-center gap-4 text-xs text-white/60 mb-3 w-full",
            style: "max-width: 640px;",
            if current_song_bitrate() > 0 {
                span { style: "font-size: 10px;", "{current_song_bitrate} kbps" }
            }
        }
    }
}
