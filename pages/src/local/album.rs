use components::dots_menu::{DotsMenu, MenuAction};
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};

#[component]
pub fn LocalAlbum(
    library: Signal<Library>,
    album_id: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut open_album_menu: Signal<Option<String>>,
    mut show_album_playlist_modal: Signal<bool>,
    mut pending_album_id_for_playlist: Signal<Option<String>>,
) -> Element {
    let local_albums = use_memo(move || {
        let mut albums = library.read().albums.clone();
        albums.sort_by(|a, b| {
            a.title
                .trim()
                .to_lowercase()
                .cmp(&b.title.trim().to_lowercase())
        });

        let mut unique_albums = Vec::new();
        let mut seen_titles = std::collections::HashSet::new();
        for album in albums {
            let title_key = album.title.trim().to_lowercase();
            if seen_titles.insert(title_key) {
                unique_albums.push(album);
            }
        }
        unique_albums
    });

    let add_all_to_playlist_text = i18n::t("add_all_to_playlist").to_string();
    let delete_album_text = i18n::t("delete_album").to_string();

    let album_menu_actions = vec![
        MenuAction::new(add_all_to_playlist_text.as_str(), "fa-solid fa-list-music"),
        MenuAction::new(delete_album_text.as_str(), "fa-solid fa-trash").destructive(),
    ];

    rsx! {
        div {
            if local_albums().is_empty() {
                p { class: "text-slate-500", "{i18n::t(\"no_albums_found\")}" }
            } else {
                div { class: "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-6",
                    for album in local_albums() {
                        {
                            let id_for_nav    = album.id.clone();
                            let id_for_menu   = album.id.clone();
                            let id_for_action = album.id.clone();
                            let title_for_action = album.title.clone();
                            let is_open = open_album_menu.read().as_deref() == Some(&album.id);
                            let cover_url = utils::format_artwork_url(album.cover_path.as_ref());
                            rsx! {
                                div {
                                    key: "{album.id}",
                                    class: if is_open { "group relative z-50 p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors" } else { "group relative p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors" },
                                    style: if is_open { "content-visibility: visible; contain: none; contain-intrinsic-size: 0 230px;" } else { "content-visibility: auto; contain-intrinsic-size: 0 230px;" },
                                    oncontextmenu: {
                                        let id = id_for_menu.clone();
                                        move |evt| {
                                            evt.prevent_default();
                                            open_album_menu.set(Some(id.clone()));
                                        }
                                    },

                                    div {
                                        class: "cursor-pointer",
                                        onclick: {
                                            let id = id_for_nav.clone();
                                            move |_| album_id.set(id.clone())
                                        },
                                        div { class: "aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                            if let Some(url) = &cover_url {
                                                img {
                                                    src: "{url.as_ref()}",
                                                    loading: "lazy",
                                                    class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-300"
                                                }
                                            } else {
                                                div { class: "w-full h-full flex items-center justify-center",
                                                    i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                                }
                                            }
                                        }
                                        h3 { class: "text-white font-medium truncate", "{album.title}" }
                                        p { class: "text-sm text-stone-400 truncate", "{album.artist}" }
                                    }

                                    div {
                                        class: "absolute bottom-3 right-3",
                                        DotsMenu {
                                            actions: album_menu_actions.clone(),
                                            is_open,
                                            on_open: {
                                                let id = id_for_menu.clone();
                                                move |_| open_album_menu.set(Some(id.clone()))
                                            },
                                            on_close: move |_| open_album_menu.set(None),
                                            button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100 bg-black/40".to_string(),
                                            anchor: "right".to_string(),
                                            on_action: {
                                                let id = id_for_action.clone();
                                                let title = title_for_action.clone();
                                                move |idx: usize| {
                                                    open_album_menu.set(None);
                                                    match idx {
                                                        0 => {
                                                            pending_album_id_for_playlist.set(Some(id.clone()));
                                                            show_album_playlist_modal.set(true);
                                                        }
                                                        1 => {
                                                            let tracks_to_delete: Vec<_> = library
                                                                .read()
                                                                .tracks
                                                                .iter()
                                                                .filter(|t| t.album == title)
                                                                .map(|t| t.path.clone())
                                                                .collect();
                                                            for path in &tracks_to_delete {
                                                                let _ = std::fs::remove_file(path);
                                                            }
                                                            let mut lib = library.write();
                                                            lib.albums.retain(|a| a.title != title);
                                                            lib.tracks.retain(|t| t.album != title);
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
