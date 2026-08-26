use crate::dots_menu::{DotsMenu, MenuAction};
use crate::metadata_modal::MetadataModal;
use crate::playlist_modal::PlaylistModal;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_player_controller::PlayerController;
use reader::Track;
use server::source::PlaylistOps;

#[component]
pub(crate) fn TrackActions(track: Track) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let api = use_context::<std::sync::Arc<dyn api::KopuzApi>>();
    let generations = hooks::db_reactivity::use_generations();
    let mut is_open = use_signal(|| false);
    let mut show_playlist_modal = use_signal(|| false);
    let mut show_metadata = use_signal(|| false);

    let capabilities = active_source.read().capabilities();
    let mut actions = vec![
        MenuAction::new(i18n::t("play_next").to_string(), "fa-solid fa-forward-step"),
        MenuAction::new(i18n::t("add_to_queue").to_string(), "fa-solid fa-list-ul"),
    ];
    let play_next_idx = 0;
    let add_to_queue_idx = 1;

    let playlist_idx = if capabilities.playlists != PlaylistOps::None {
        let idx = actions.len();
        actions.push(MenuAction::new(
            i18n::t("add_to_playlist").to_string(),
            "fa-solid fa-plus",
        ));
        Some(idx)
    } else {
        None
    };

    let share_idx = actions.len();
    actions.push(MenuAction::new(
        i18n::t("share_musicbrainz").to_string(),
        "fa-solid fa-share-nodes",
    ));

    let radio_idx = if capabilities.radio.track {
        let idx = actions.len();
        actions.push(MenuAction::new(
            crate::radio_actions::radio_label(),
            crate::radio_actions::RADIO_ICON,
        ));
        Some(idx)
    } else {
        None
    };

    let metadata_idx = actions.len();
    actions.push(MenuAction::new(
        i18n::t("view_metadata").to_string(),
        "fa-solid fa-circle-info",
    ));
    let api_menu = api.clone();
    let api_add = api.clone();
    let api_create = api.clone();

    rsx! {
        DotsMenu {
            actions,
            is_open: is_open(),
            on_open: move |_| is_open.set(true),
            on_close: move |_| is_open.set(false),
            button_class: "w-11 h-11 bg-white/10 text-white/70 hover:bg-white/15 hover:text-white active:scale-95".to_string(),
            anchor: "right".to_string(),
            placement: "top".to_string(),
            icon: "fa-solid fa-ellipsis".to_string(),
            on_action: {
                let action_track = track.clone();
                move |idx: usize| {
                    is_open.set(false);
                    if idx == play_next_idx {
                        ctrl.queue_play_next(vec![action_track.clone()]);
                    } else if idx == add_to_queue_idx {
                        ctrl.add_to_queue(vec![action_track.clone()]);
                    } else if playlist_idx == Some(idx) {
                        show_playlist_modal.set(true);
                    } else if idx == share_idx {
                        let source = active_source.peek().clone();
                        crate::track_row::share_track(action_track.clone(), source);
                    } else if radio_idx == Some(idx) {
                        crate::track_row::play_radio(action_track.clone(), api_menu.clone(), ctrl);
                    } else if idx == metadata_idx {
                        show_metadata.set(true);
                    }
                }
            },
        }

        if *show_playlist_modal.read() {
            PlaylistModal {
                overlay_class: Some("overlay".to_string()),
                on_close: move |_| show_playlist_modal.set(false),
                on_add_to_playlist: {
                    let playlist_track = track.clone();
                    move |playlist_id: String| {
                        let item_ref = playlist_track.id.key().into_owned();
                        let api = api_add.clone();
                        spawn(async move {
                            match api.add_playlist_tracks(playlist_id, vec![item_ref]).await {
                                Ok(_) => generations.bump(Table::Playlists),
                                Err(error) => tracing::warn!(%error, "failed to add fullscreen track to playlist"),
                            }
                        });
                        show_playlist_modal.set(false);
                    }
                },
                on_create_playlist: {
                    let playlist_track = track.clone();
                    move |name: String| {
                        let item_ref = playlist_track.id.key().into_owned();
                        let api = api_create.clone();
                        spawn(async move {
                            match api.create_playlist(name, vec![item_ref]).await {
                                Ok(_) => generations.bump(Table::Playlists),
                                Err(error) => tracing::warn!(%error, "failed to create playlist from fullscreen track"),
                            }
                        });
                        show_playlist_modal.set(false);
                    }
                },
            }
        }

        if *show_metadata.read() {
            MetadataModal {
                track: track.clone(),
                on_close: move |_| show_metadata.set(false),
            }
        }
    }
}
