use api::TrackInfo as Track;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use std::collections::HashSet;

#[derive(Props, Clone, PartialEq)]
pub struct TrackListViewProps {
    pub name: String,
    pub description: String,
    #[props(default)]
    pub on_description_click: Option<EventHandler<()>>,
    pub cover_url: Option<utils::CoverUrl>,
    pub tracks: Vec<Track>,
    #[props(default = false)]
    pub is_album: bool,
    #[props(default)]
    pub release_year: Option<u16>,
    pub on_close: EventHandler<()>,
    pub on_cover_click: Option<EventHandler<()>>,
    pub on_delete_track: Option<EventHandler<usize>>,
    pub on_selection_delete: Option<EventHandler<Vec<String>>>,
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
    #[props(default = false)]
    pub enable_metadata: bool,
    pub actions: Option<Element>,
    #[props(default)]
    pub on_start_radio: Option<EventHandler<()>>,
}

#[component]
pub fn TrackListView(props: TrackListViewProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<String>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<String>);
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(HashSet::<String>::new);
    let mut metadata_track = use_signal(|| None::<Track>);

    let view_metadata_handler = if props.enable_metadata {
        let tracks_meta = props.tracks.clone();
        Some(EventHandler::new(move |idx: usize| {
            if let Some(t) = tracks_meta.get(idx) {
                metadata_track.set(Some(t.clone()));
                active_menu_track.set(None);
            }
        }))
    } else {
        None
    };

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
        div { class: "w-full max-w-[1600px] mx-auto select-none flex-1 min-h-0 flex flex-col",
            if !cfg!(target_os = "android") {
                div { class: "flex items-center mb-8 shrink-0",
                    crate::back_button::BackButton {
                        class: "",
                        on_click: move |_| props.on_close.call(()),
                    }
                }
            }

            crate::showcase::Showcase {
                name: props.name.clone(),
                description: props.description.clone(),
                on_description_click: props.on_description_click,
                cover_url: props.cover_url.clone(),
                tracks: props.tracks.clone(),
                is_album: props.is_album,
                release_year: props.release_year,
                is_selection_mode: is_selection_mode(),
                selected_tracks: selected_tracks.read().clone(),
                all_selected: !props.tracks.is_empty()
                    && props.tracks.iter().all(|t| selected_tracks.read().contains(&t.key)),
                on_cover_click: props.on_cover_click,
                actions: props.actions,
                on_start_radio: props.on_start_radio,
                on_select_all: move |selected: bool| {
                    if selected {
                        selected_tracks
                            .set(tracks_select_all.iter().map(|t| t.key.clone()).collect());
                        is_selection_mode.set(true);
                    }
                    else {
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    }
                },
                on_long_press: move |idx: usize| {
                    if let Some(t) = tracks_long_press.get(idx) {
                        is_selection_mode.set(true);
                        selected_tracks.write().insert(t.key.clone());
                    }
                },
                on_select: move |(idx, sel): (usize, bool)| {
                    if let Some(t) = tracks_select.get(idx) {
                        if sel {
                            is_selection_mode.set(true);
                            selected_tracks.write().insert(t.key.clone());
                        } else {
                            selected_tracks.write().remove(&t.key);
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
                    ctrl.play_queue_at(tracks_play.clone(), idx);
                },
                on_add_to_playlist: move |idx: usize| {
                    if let Some(t) = tracks_add.get(idx) {
                        selected_track_for_playlist.set(Some(t.key.clone()));
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
                        if active_menu_track.read().as_ref() == Some(&t.uid) {
                            active_menu_track.set(None);
                        } else {
                            active_menu_track.set(Some(t.uid.clone()));
                        }
                    }
                },
                on_close_menu: move |_| active_menu_track.set(None),
                on_view_metadata: view_metadata_handler,
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
                            .filter(|t| selected.contains(&t.key))
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
                        let keys: Vec<String> = tracks_sel_delete
                            .iter()
                            .filter(|t| selected_tracks.read().contains(&t.key))
                            .map(|t| t.key.clone())
                            .collect();
                        if let Some(ref h) = props.on_selection_delete {
                            h.call(keys);
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

            if let Some(track) = metadata_track.read().clone() {
                crate::metadata_modal::MetadataModal {
                    track: track.clone(),
                    on_close: move |_| metadata_track.set(None),
                    on_save: move |patch: api::TrackMetadataPatch| {
                        hooks::library_actions::edit_track(patch);
                        metadata_track.set(None);
                    },
                }
            }

            if *show_playlist_modal.read() {
                crate::playlist_modal::PlaylistModal {
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
                        let refs: Vec<String> =
                            paths.clone();
                        hooks::playlist_actions::add_tracks(playlist_id, refs);
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
                        let refs: Vec<String> =
                            paths.clone();
                        hooks::playlist_actions::create_with(name, refs);
                        show_playlist_modal.set(false);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                }
            }
        }
    }
}
