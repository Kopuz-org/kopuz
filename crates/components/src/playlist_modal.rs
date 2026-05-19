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
    let add_to_playlist_text = i18n::t("add_to_playlist").to_string();
    let no_playlists_found_text = i18n::t("no_playlists_found").to_string();
    let create_new_playlist_text = i18n::t("create_new_playlist").to_string();
    let create_text = i18n::t("create").to_string();
    let cancel_text = i18n::t("cancel").to_string();
    let playlist_name_input = i18n::t("playlist_name_input").to_string();
    
    let playlists: Vec<(String, String, String)> = if props.is_jellyfin {
        store
            .jellyfin_playlists
            .iter()
            .map(|p| {
                let track_text = if p.tracks.len() == 1 {
                    i18n::t("track_count_singular").to_string()
                } else {
                    i18n::t_with("track_count", &[("count", p.tracks.len().to_string())])
                };
                (p.id.clone(), p.name.clone(), track_text)
            })
            .collect()
    } else {
        store
            .playlists
            .iter()
            .map(|p| {
                let track_text = if p.tracks.len() == 1 {
                    i18n::t("track_count_singular").to_string()
                } else {
                    i18n::t_with("track_count", &[("count", p.tracks.len().to_string())])
                };
                (p.id.clone(), p.name.clone(), track_text)
            })
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
                    "{add_to_playlist_text}"
                }

                div { class: "max-h-60 overflow-y-auto mb-4 space-y-2",
                    if playlists.is_empty() {
                        p { class: "text-slate-500 text-sm italic", "{no_playlists_found_text}" }
                    }
                    for (id, name, track_count) in playlists {
                        button {
                            class: "w-full text-left p-3 rounded-lg bg-white/5 hover:bg-white/10 text-white transition-colors flex items-center justify-between group",
                            onclick: move |_| props.on_add_to_playlist.call(id.clone()),
                            span { "{name}" }
                            span { class: "text-xs text-slate-500 group-hover:text-slate-400", "{track_count}" }
                        }
                    }
                }

                div { class: "border-t border-white/10 pt-4 mt-4",
                    h3 { class: "text-sm font-medium text-white/60 mb-2", "{create_new_playlist_text}" }
                    div { class: "flex gap-2",
                        input {
                            r#type: "text",
                            class: "flex-1 bg-white/5 border border-white/10 rounded px-3 py-2 text-white text-sm focus:outline-none focus:border-white/20",
                            placeholder: "{playlist_name_input}",
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
                            "{create_text}"
                        }
                    }
                }

                div { class: "mt-6 flex justify-end",
                    button {
                        class: "text-slate-400 hover:text-white text-sm transition-colors",
                        onclick: move |_| props.on_close.call(()),
                        "{cancel_text}"
                    }
                }
            }
        }
    }
}
