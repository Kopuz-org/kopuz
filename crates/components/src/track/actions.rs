//! The single track-actions menu.
//!
//! Every surface that shows a track (library rows, the queue, the player bar,
//! home cards) opens this one component, so the action set, ordering and
//! wording stay identical wherever a track appears. A surface passes handlers
//! only for the actions it can perform; everything it omits is left out of the
//! menu rather than shown disabled, because a surface never gains the ability
//! mid-session.

use crate::NavigationController;
use crate::dots_menu::{DotsMenu, MenuAction};
use crate::radio_actions::{RADIO_ICON, radio_label, track_radio_handler};
use crate::track_row::share_track;
use dioxus::prelude::*;
use hooks::PlayerController;
use hooks::db_reactivity::Table;
use reader::Track;

#[derive(Clone, Copy, PartialEq)]
enum Action {
    PlayNext,
    AddToQueue,
    RemoveFromQueue,
    AddToPlaylist,
    RemoveFromPlaylist,
    StartRadio,
    GoToArtist,
    GoToAlbum,
    Download,
    Share,
    ViewMetadata,
    Delete,
}

#[derive(Props, Clone, PartialEq)]
pub struct TrackActionsMenuProps {
    pub track: Track,

    /// Open state owned by the parent, for surfaces that keep at most one row
    /// menu open at a time. Leave unset and the menu owns its own state.
    #[props(default)]
    pub is_open: Option<bool>,
    #[props(default)]
    pub on_open: Option<EventHandler<()>>,
    #[props(default)]
    pub on_close: Option<EventHandler<()>>,

    #[props(default)]
    pub button_class: String,
    #[props(default = "right".to_string())]
    pub anchor: String,
    /// Passed straight to [`DotsMenu`]: "bottom" (default), or "top" for a
    /// trigger sitting at the bottom of the window like the player bar.
    #[props(default = "bottom".to_string())]
    pub placement: String,
    /// Trigger glyph, for surfaces whose chrome wants the horizontal ellipsis.
    #[props(default = "fa-solid fa-ellipsis-vertical".to_string())]
    pub icon: String,
    /// Overlay class for the built-in playlist modal. The fullscreen player
    /// needs `.overlay`, which outranks its own chrome.
    #[props(default)]
    pub playlist_overlay_class: Option<String>,

    /// Queue writes are always available, except on the queue itself where
    /// re-queueing the row the user is looking at reads as a no-op.
    #[props(default = true)]
    pub show_queue_actions: bool,

    /// Takes over "Add to playlist" for pages that already run a
    /// selection-aware playlist modal. Unset and the menu runs its own.
    #[props(default)]
    pub on_add_to_playlist: Option<EventHandler<()>>,
    #[props(default)]
    pub on_remove_from_queue: Option<EventHandler<()>>,
    #[props(default)]
    pub on_remove_from_playlist: Option<EventHandler<()>>,
    #[props(default)]
    pub on_download: Option<EventHandler<()>>,
    #[props(default)]
    pub on_view_metadata: Option<EventHandler<()>>,
    #[props(default)]
    pub on_delete: Option<EventHandler<()>>,
    #[props(default = false)]
    pub is_downloaded: bool,
    #[props(default = false)]
    pub is_downloading: bool,
}

#[component]
pub fn TrackActionsMenu(props: TrackActionsMenuProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let nav_ctrl = use_context::<NavigationController>();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let gens = hooks::db_reactivity::use_generations();
    let mut local_open = use_signal(|| false);
    let mut show_playlist_modal = use_signal(|| false);

    let capabilities = active_source.read().capabilities();
    let on_start_radio = track_radio_handler(props.track.clone());
    let is_open = props.is_open.unwrap_or_else(|| *local_open.read());

    // Handlers are hoisted out of `props` so the rsx closures capture Copy
    // values instead of the whole (non-Copy) props struct.
    let on_open = props.on_open;
    let on_close = props.on_close;
    let on_add_to_playlist = props.on_add_to_playlist;
    let on_remove_from_queue = props.on_remove_from_queue;
    let on_remove_from_playlist = props.on_remove_from_playlist;
    let on_download = props.on_download;
    let is_downloading = props.is_downloading;
    let on_view_metadata = props.on_view_metadata;
    let on_delete = props.on_delete;
    let playlist_overlay_class = props.playlist_overlay_class.clone();

    let mut close = move || match on_close {
        Some(handler) => handler.call(()),
        None => local_open.set(false),
    };

    let mut entries: Vec<(Action, MenuAction)> = Vec::new();

    if props.show_queue_actions {
        entries.push((
            Action::PlayNext,
            MenuAction::new(i18n::t("play_next"), "fa-solid fa-forward-step"),
        ));
        entries.push((
            Action::AddToQueue,
            MenuAction::new(i18n::t("add_to_queue"), "fa-solid fa-list-ul"),
        ));
    }

    if on_remove_from_queue.is_some() {
        entries.push((
            Action::RemoveFromQueue,
            MenuAction::new(i18n::t("remove_from_queue"), "fa-solid fa-xmark"),
        ));
    }

    if capabilities.playlists != ::server::source::PlaylistOps::None {
        entries.push((
            Action::AddToPlaylist,
            MenuAction::new(i18n::t("add_to_playlist"), "fa-solid fa-plus"),
        ));
    }

    if on_remove_from_playlist.is_some() {
        entries.push((
            Action::RemoveFromPlaylist,
            MenuAction::new(i18n::t("remove_from_playlist"), "fa-solid fa-minus"),
        ));
    }

    if on_start_radio.is_some() {
        entries.push((
            Action::StartRadio,
            MenuAction::new(radio_label(), RADIO_ICON),
        ));
    }

    if !props.track.artist.trim().is_empty() {
        entries.push((
            Action::GoToArtist,
            MenuAction::new(i18n::t("go_to_artist"), "fa-solid fa-user"),
        ));
    }

    if !props.track.album_id.trim().is_empty() {
        entries.push((
            Action::GoToAlbum,
            MenuAction::new(i18n::t("go_to_album"), "fa-solid fa-compact-disc"),
        ));
    }

    // `on_download` is only wired by sources that support downloads, so its
    // presence is the gate: no separate capability check needed here.
    if on_download.is_some() {
        let action = if props.is_downloading {
            MenuAction::new(i18n::t("downloading"), "fa-solid fa-spinner fa-spin")
        } else if props.is_downloaded {
            MenuAction::new(i18n::t("remove_download"), "fa-solid fa-trash-can").destructive()
        } else {
            MenuAction::new(i18n::t("download_offline"), "fa-solid fa-download")
        };
        entries.push((Action::Download, action));
    }

    entries.push((
        Action::Share,
        MenuAction::new(i18n::t("share_musicbrainz"), "fa-solid fa-share-nodes"),
    ));

    if on_view_metadata.is_some() {
        entries.push((
            Action::ViewMetadata,
            MenuAction::new(i18n::t("view_metadata"), "fa-solid fa-circle-info"),
        ));
    }

    // Only the local source can delete a file, so on every remote source this
    // entry could be shown but never do anything.
    if on_delete.is_some() && capabilities.delete_from_disk {
        entries.push((
            Action::Delete,
            MenuAction::new(i18n::t("delete_from_device"), "fa-solid fa-trash").destructive(),
        ));
    }

    let dispatch_entries: Vec<Action> = entries.iter().map(|(action, _)| *action).collect();
    let actions: Vec<MenuAction> = entries.into_iter().map(|(_, item)| item).collect();

    let dispatch_track = props.track.clone();
    // The track's credit can name several artists ("Alice feat. Bob"); the
    // artist page matches on the components a credit splits into, so navigate
    // to the first artist it names rather than the joined string.
    let nav_artist = reader::artist::split_credit(&props.track.artist)
        .into_iter()
        .next()
        .unwrap_or_else(|| props.track.artist.clone());
    let add_ref = props.track.id.key().into_owned();
    let create_ref = add_ref.clone();

    rsx! {
        DotsMenu {
            actions,
            is_open,
            aria_label: i18n::t_with("more_actions_for", &[("name", props.track.title.clone())]),
            button_class: props.button_class.clone(),
            anchor: props.anchor.clone(),
            placement: props.placement.clone(),
            icon: props.icon.clone(),
            on_open: move |_| {
                match on_open {
                    Some(handler) => handler.call(()),
                    None => local_open.set(true),
                }
            },
            on_close: move |_| close(),
            on_action: move |idx: usize| {
                let Some(action) = dispatch_entries.get(idx).copied() else {
                    return;
                };
                let track = dispatch_track.clone();
                match action {
                    Action::PlayNext => ctrl.queue_play_next(vec![track]),
                    Action::AddToQueue => ctrl.add_to_queue(vec![track]),
                    Action::RemoveFromQueue => {
                        if let Some(handler) = on_remove_from_queue {
                            handler.call(());
                        }
                    }
                    Action::AddToPlaylist => match on_add_to_playlist {
                        Some(handler) => handler.call(()),
                        None => show_playlist_modal.set(true),
                    },
                    Action::RemoveFromPlaylist => {
                        if let Some(handler) = on_remove_from_playlist {
                            handler.call(());
                        }
                    }
                    Action::StartRadio => {
                        if let Some(handler) = on_start_radio {
                            handler.call(());
                        }
                    }
                    Action::GoToArtist => nav_ctrl.navigate_to_artist(nav_artist.clone()),
                    Action::GoToAlbum => nav_ctrl.navigate_to_album(track.album_id.clone()),
                    Action::Download => {
                        // "Downloading..." is a status row, not an action. The
                        // queue discards a repeat request, but the menu should
                        // not be leaning on that to stay correct.
                        if !is_downloading
                            && let Some(handler) = on_download
                        {
                            handler.call(());
                        }
                    }
                    Action::Share => share_track(track, active_source.peek().clone()),
                    Action::ViewMetadata => {
                        if let Some(handler) = on_view_metadata {
                            handler.call(());
                        }
                    }
                    Action::Delete => {
                        if let Some(handler) = on_delete {
                            handler.call(());
                        }
                    }
                }
                close();
            },
        }

        if *show_playlist_modal.read() {
            crate::playlist_modal::PlaylistModal {
                overlay_class: playlist_overlay_class.clone(),
                on_close: move |_| show_playlist_modal.set(false),
                on_add_to_playlist: move |playlist_id: String| {
                    let refs = vec![add_ref.clone()];
                    let source = active_source.peek().clone();
                    spawn(async move {
                        match source.add_to_playlist(&playlist_id, &refs).await {
                            Ok(_) => gens.bump(Table::Playlists),
                            Err(e) => tracing::warn!(error = %e, "add to playlist failed"),
                        }
                    });
                    show_playlist_modal.set(false);
                },
                on_create_playlist: move |name: String| {
                    let refs = vec![create_ref.clone()];
                    let source = active_source.peek().clone();
                    spawn(async move {
                        match source.create_playlist(&name, &refs).await {
                            Ok(_) => gens.bump(Table::Playlists),
                            Err(e) => tracing::warn!(error = %e, "create playlist failed"),
                        }
                    });
                    show_playlist_modal.set(false);
                },
            }
        }
    }
}
