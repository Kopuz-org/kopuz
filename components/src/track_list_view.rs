use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use reader::{Library, PlaylistStore, Track};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Props, Clone, PartialEq)]
pub struct TrackListViewProps {
    pub name: String,
    pub description: String,
    pub cover_url: Option<utils::CoverUrl>,
    pub back_label: String,
    pub tracks: Vec<Track>,
    pub library: Signal<Library>,
    pub playlist_store: Signal<PlaylistStore>,
    pub on_close: EventHandler<()>,
    pub on_cover_click: Option<EventHandler<()>>,
    pub on_delete_track: Option<EventHandler<usize>>,
    pub on_selection_delete: Option<EventHandler<Vec<PathBuf>>>,
    pub on_remove_from_playlist: Option<EventHandler<usize>>,
    pub on_download_all: Option<EventHandler<()>>,
    pub on_delete_all: Option<EventHandler<()>>,
    pub on_download_track: Option<EventHandler<usize>>,
    #[props(default = false)]
    pub is_downloading_all: bool,
    #[props(default = false)]
    pub is_reorderable: bool,
    #[props(default)]
    pub on_move_up: EventHandler<usize>,
    #[props(default)]
    pub on_move_down: EventHandler<usize>,
    #[props(default = true)]
    pub show_delete_in_selection: bool,
    pub actions: Option<Element>,
}

#[component]
pub fn TrackListView(mut props: TrackListViewProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());

    let tracks_select_all = props.tracks.clone();
    let tracks_long_press = props.tracks.clone();
    let tracks_select = props.tracks.clone();
    let tracks_play_all = props.tracks.clone();
    let tracks_play = props.tracks.clone();
    let tracks_add = props.tracks.clone();
    let tracks_queue = props.tracks.clone();
    let tracks_menu = props.tracks.clone();
    let tracks_sel_delete = props.tracks.clone();
    let tracks_sel_queue = props.tracks.clone();

    rsx! {
        div { class: "w-full max-w-[1600px] mx-auto select-none",
            div { class: "flex items-center mb-8",
                button {
                    class: "flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                    onclick: move |_| props.on_close.call(()),
                    i { class: "fa-solid fa-arrow-left" }
                    "{props.back_label}"
                }
            }

            crate::showcase::Showcase {
                name: props.name.clone(),
                description: props.description.clone(),
                cover_url: props.cover_url.clone(),
                tracks: props.tracks.clone(),
                library: props.library,
                is_selection_mode: is_selection_mode(),
                selected_tracks: selected_tracks.read().clone(),
                all_selected: !props.tracks.is_empty()
                    && props.tracks.iter().all(|t| selected_tracks.read().contains(&t.path)),
                on_cover_click: props.on_cover_click,
                actions: props.actions,
                on_select_all: move |selected: bool| {
                    if selected {
                        selected_tracks.set(tracks_select_all.iter().map(|t| t.path.clone()).collect());
                        is_selection_mode.set(true);
                    } else {
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    }
                },
                on_long_press: move |idx: usize| {
                    if let Some(t) = tracks_long_press.get(idx) {
                        is_selection_mode.set(true);
                        selected_tracks.write().insert(t.path.clone());
                    }
                },
                on_select: move |(idx, sel): (usize, bool)| {
                    if let Some(t) = tracks_select.get(idx) {
                        if sel {
                            is_selection_mode.set(true);
                            selected_tracks.write().insert(t.path.clone());
                        } else {
                            selected_tracks.write().remove(&t.path);
                            if selected_tracks.read().is_empty() {
                                is_selection_mode.set(false);
                            }
                        }
                    }
                },
                on_play_all: move |_| {
                    let is_shuffle = *ctrl.shuffle.peek();
                    if is_shuffle {
                        ctrl.play_queue_shuffled(tracks_play_all.clone());
                    } else {
                        ctrl.play_queue_linear(tracks_play_all.clone());
                    }
                },
                on_play: move |idx: usize| {
                    ctrl.queue.set(tracks_play.clone());
                    ctrl.play_track(idx);
                },
                on_add_to_playlist: move |idx: usize| {
                    if let Some(t) = tracks_add.get(idx) {
                        selected_track_for_playlist.set(Some(t.path.clone()));
                        show_playlist_modal.set(true);
                        active_menu_track.set(None);
                    }
                },
                on_queue: move |idx: usize| {
                    if let Some(t) = tracks_queue.get(idx) {
                        ctrl.add_to_queue(vec![t.clone()]);
                        active_menu_track.set(None);
                    }
                },
                active_track: active_menu_track.read().clone(),
                on_click_menu: move |idx: usize| {
                    if let Some(t) = tracks_menu.get(idx) {
                        if active_menu_track.read().as_ref() == Some(&t.path) {
                            active_menu_track.set(None);
                        } else {
                            active_menu_track.set(Some(t.path.clone()));
                        }
                    }
                },
                on_close_menu: move |_| active_menu_track.set(None),
                on_delete_track: props.on_delete_track,
                on_remove_from_playlist: props.on_remove_from_playlist,
                on_download_all: props.on_download_all,
                on_delete_all: props.on_delete_all,
                on_download_track: props.on_download_track,
                is_downloading_all: props.is_downloading_all,
                is_reorderable: props.is_reorderable,
                on_move_up: props.on_move_up,
                on_move_down: props.on_move_down,
            }

            if is_selection_mode() {
                crate::selection_bar::SelectionBar {
                    count: selected_tracks.read().len(),
                    show_delete: props.show_delete_in_selection,
                    on_add_to_queue: move |_| {
                        let selected = selected_tracks.read().clone();
                        if selected.is_empty() {
                            return;
                        }
                        let tracks: Vec<_> = tracks_sel_queue
                            .iter()
                            .filter(|t| selected.contains(&t.path))
                            .cloned()
                            .collect();
                        if !tracks.is_empty() {
                            ctrl.add_to_queue(tracks);
                        }
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    },
                    on_add_to_playlist: move |_| show_playlist_modal.set(true),
                    on_delete: move |_| {
                        let paths: Vec<PathBuf> = tracks_sel_delete.iter()
                            .filter(|t| selected_tracks.read().contains(&t.path))
                            .map(|t| t.path.clone())
                            .collect();
                        if let Some(ref h) = props.on_selection_delete {
                            h.call(paths);
                        }
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    },
                    on_cancel: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                }
            }

            if *show_playlist_modal.read() {
                crate::playlist_modal::PlaylistModal {
                    playlist_store: props.playlist_store,
                    is_jellyfin: false,
                    on_close: move |_| {
                        show_playlist_modal.set(false);
                        if is_selection_mode() {
                            is_selection_mode.set(false);
                            selected_tracks.write().clear();
                        }
                    },
                    on_add_to_playlist: move |playlist_id: String| {
                        let mut paths = Vec::new();
                        if is_selection_mode() {
                            paths = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            paths.push(path);
                        }
                        if !paths.is_empty() {
                            let mut store = props.playlist_store.write();
                            if let Some(pl) = store.playlists.iter_mut().find(|p| p.id == playlist_id) {
                                for path in paths {
                                    if !pl.tracks.contains(&path) {
                                        pl.tracks.push(path);
                                    }
                                }
                            }
                        }
                        show_playlist_modal.set(false);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_create_playlist: move |name: String| {
                        let mut paths = Vec::new();
                        if is_selection_mode() {
                            paths = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            paths.push(path);
                        }
                        if !paths.is_empty() {
                            let mut store = props.playlist_store.write();
                            store.playlists.push(reader::models::Playlist {
                                id: uuid::Uuid::new_v4().to_string(),
                                name,
                                tracks: paths,
                                cover_path: None,
                            });
                        }
                        show_playlist_modal.set(false);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                }
            }
        }
    }
}
