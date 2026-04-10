use dioxus::prelude::*;
use reader::PlaylistStore;

#[derive(PartialEq, Clone, Copy, Props)]
pub struct PlaylistModalProps {
    pub playlist_store: Signal<PlaylistStore>,
    pub is_jellyfin: bool,
    pub on_close: EventHandler,
    pub on_add_to_playlist: EventHandler<String>,
    pub on_create_playlist: EventHandler<String>,
}

#[component]
pub fn PlaylistModal(props: PlaylistModalProps) -> Element {
    let mut new_playlist_name = use_signal(String::new);
    let store = props.playlist_store.read();
    let playlists: Vec<_> = if props.is_jellyfin {
        store
            .jellyfin_playlists
            .iter()
            .map(|p| (p.id.clone(), p.name.clone(), p.tracks.len()))
            .collect()
    } else {
        store
            .playlists
            .iter()
            .map(|p| (p.id.clone(), p.name.clone(), p.tracks.len()))
            .collect()
    };

    rsx! {
        div {
            class: "fixed inset-0 bg-black/80 flex items-center justify-center z-50",
            onclick: move |_| props.on_close.call(()),
            div {
                class: "bg-neutral-900 rounded-xl border border-white/10 w-full max-w-md p-6",
                onclick: move |e| e.stop_propagation(),
                h2 { class: "text-xl font-bold text-white mb-4",
                    "Add to Playlist"
                }

                div { class: "max-h-60 overflow-y-auto mb-4 space-y-2",
                    if playlists.is_empty() {
                        p { class: "text-slate-500 text-sm italic", "No playlists found." }
                    }
                    for (id, name, track_count) in playlists {
                        button {
                            class: "w-full text-left p-3 rounded-lg bg-white/5 hover:bg-white/10 text-white transition-colors flex items-center justify-between group",
                            onclick: move |_| props.on_add_to_playlist.call(id.clone()),
                            span { "{name}" }
                            span { class: "text-xs text-slate-500 group-hover:text-slate-400", "{track_count} tracks" }
                        }
                    }
                }

                div { class: "border-t border-white/10 pt-4 mt-4",
                    h3 { class: "text-sm font-medium text-white/60 mb-2", "Create New Playlist" }
                    div { class: "flex gap-2",
                        input {
                            r#type: "text",
                            class: "flex-1 bg-white/5 border border-white/10 rounded px-3 py-2 text-white text-sm focus:outline-none focus:border-white/20",
                            placeholder: "Playlist Name",
                            value: "{new_playlist_name}",
                            oninput: move |e| new_playlist_name.set(e.value()),
                            onkeydown: move |e| e.stop_propagation()
                        }
                        button {
                            class: "bg-white text-black px-4 py-2 rounded text-sm font-medium hover:bg-slate-200 transition-colors disabled:opacity-50 disabled:cursor-not-allowed",
                            disabled: new_playlist_name.read().is_empty(),
                            onclick: move |_| {
                                let name = new_playlist_name.read().clone();
                                if !name.is_empty() {
                                    props.on_create_playlist.call(name);
                                    new_playlist_name.set(String::new());
                                }
                            },
                            "Create"
                        }
                    }
                }

                div { class: "mt-6 flex justify-end",
                    button {
                        class: "text-slate-400 hover:text-white text-sm transition-colors",
                        onclick: move |_| props.on_close.call(()),
                        "Cancel"
                    }
                }
            }
        }
    }
}
