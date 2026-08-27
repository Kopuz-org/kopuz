//! The album counterpart of [`crate::track_actions::TrackActionsMenu`].
//!
//! Home renders albums as bare cards rather than through a row component, so
//! until now they carried no actions at all. The set is deliberately the subset
//! that needs nothing but an album id: deleting or editing an album belongs to
//! the album page, which has the state for it.

use crate::NavigationController;
use crate::dots_menu::{DotsMenu, MenuAction};
use dioxus::prelude::*;
use hooks::PlayerController;
use hooks::db_reactivity::Table;
use hooks::toast::toast_error;
use reader::Track;
use server::source::{ActiveSource, SourceError};

#[derive(Clone, Copy, PartialEq)]
enum Action {
    PlayNext,
    AddToQueue,
    AddToPlaylist,
    GoToArtist,
    Download,
    Delete,
}

/// Album order as the album page shows it. A source is free to hand back its
/// own order, so queueing without this can interleave discs or open at track 7.
///
/// The error is carried rather than flattened into an empty list: every caller
/// treats "no tracks" as nothing to do, so a swallowed failure looks exactly
/// like an empty album and the click reports nothing at all.
async fn album_tracks_in_order(
    source: &ActiveSource,
    album_id: &str,
) -> Result<Vec<Track>, SourceError> {
    let mut tracks = source.album_tracks(album_id).await?;
    tracks.sort_by(|a, b| {
        a.disc_number
            .cmp(&b.disc_number)
            .then_with(|| a.track_number.cmp(&b.track_number))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(tracks)
}

/// The refs an album contributes to a playlist, in album order.
async fn album_track_refs(
    source: &ActiveSource,
    album_id: &str,
) -> Result<Vec<String>, SourceError> {
    Ok(album_tracks_in_order(source, album_id)
        .await?
        .iter()
        .map(|track| track.id.key().into_owned())
        .collect())
}

/// Say so when the album's tracks could not be read, then stand down.
///
/// Shown, not just logged: each of these runs off a direct click, so failing
/// quietly leaves the user watching a menu close over nothing.
fn report_lookup_failure(album_id: &str, error: &SourceError) {
    tracing::warn!(%error, album = %album_id, "album menu: could not list its tracks");
    toast_error(&i18n::t_with(
        "error_fetch_songs",
        &[
            ("album_id", album_id.to_string()),
            ("error", error.to_string()),
        ],
    ));
}

#[derive(Props, Clone, PartialEq)]
pub struct AlbumActionsMenuProps {
    pub album_id: String,
    pub album_title: String,
    #[props(default)]
    pub artist: String,

    /// Open state owned by the parent, for rows that keep at most one card menu
    /// open at a time. Leave unset and the menu owns its own state.
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
    #[props(default = "bottom".to_string())]
    pub placement: String,

    /// Deleting an album means different things per page (files and rows
    /// locally, a cache drop on a server), so the page keeps that handler and
    /// the label that goes with it.
    #[props(default)]
    pub on_delete: Option<EventHandler<()>>,
    #[props(default)]
    pub delete_label: Option<String>,
    #[props(default)]
    pub on_download: Option<EventHandler<()>>,
    #[props(default = false)]
    pub is_downloaded: bool,
    #[props(default = false)]
    pub is_downloading: bool,
}

#[component]
pub fn AlbumActionsMenu(props: AlbumActionsMenuProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let nav_ctrl = use_context::<NavigationController>();
    let active_source = use_context::<Signal<ActiveSource>>();
    let gens = hooks::db_reactivity::use_generations();
    let mut local_open = use_signal(|| false);
    let mut show_playlist_modal = use_signal(|| false);

    let capabilities = active_source.read().capabilities();
    let is_open = props.is_open.unwrap_or_else(|| *local_open.read());

    let on_open = props.on_open;
    let on_close = props.on_close;
    let on_delete = props.on_delete;
    let on_download = props.on_download;
    let is_downloading = props.is_downloading;
    let mut close = move || match on_close {
        Some(handler) => handler.call(()),
        None => local_open.set(false),
    };

    let mut entries: Vec<(Action, MenuAction)> = vec![
        (
            Action::PlayNext,
            MenuAction::new(i18n::t("play_next"), "fa-solid fa-forward-step"),
        ),
        (
            Action::AddToQueue,
            MenuAction::new(i18n::t("add_all_to_queue"), "fa-solid fa-list-ul"),
        ),
    ];

    if capabilities.playlists != ::server::source::PlaylistOps::None {
        entries.push((
            Action::AddToPlaylist,
            MenuAction::new(i18n::t("add_all_to_playlist"), "fa-solid fa-plus"),
        ));
    }

    if !props.artist.trim().is_empty() {
        entries.push((
            Action::GoToArtist,
            MenuAction::new(i18n::t("go_to_artist"), "fa-solid fa-user"),
        ));
    }

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

    if on_delete.is_some() {
        let label = props
            .delete_label
            .clone()
            .unwrap_or_else(|| i18n::t("delete_album").to_string());
        entries.push((
            Action::Delete,
            MenuAction::new(label, "fa-solid fa-trash").destructive(),
        ));
    }

    let dispatch: Vec<Action> = entries.iter().map(|(action, _)| *action).collect();
    let actions: Vec<MenuAction> = entries.into_iter().map(|(_, item)| item).collect();

    let dispatch_album = props.album_id.clone();
    // The card shows the album's whole credit, which can name several artists
    // ("Alice feat. Bob"). The artist page matches albums by the components a
    // credit splits into, so navigating to the joined string lands on a page
    // that matches nothing; go to the first artist the credit names. Splitting
    // through the same helper the page filters with is what keeps the two ends
    // agreeing.
    let dispatch_artist = reader::artist::split_credit(&props.artist)
        .into_iter()
        .next()
        .unwrap_or_else(|| props.artist.clone());
    let add_album = props.album_id.clone();
    let create_album = props.album_id.clone();

    rsx! {
        DotsMenu {
            actions,
            is_open,
            aria_label: i18n::t_with("more_actions_for", &[("name", props.album_title.clone())]),
            button_class: props.button_class.clone(),
            anchor: props.anchor.clone(),
            placement: props.placement.clone(),
            on_open: move |_| {
                match on_open {
                    Some(handler) => handler.call(()),
                    None => local_open.set(true),
                }
            },
            on_close: move |_| close(),
            on_action: move |idx: usize| {
                let Some(action) = dispatch.get(idx).copied() else {
                    return;
                };
                match action {
                    Action::PlayNext | Action::AddToQueue => {
                        let source = active_source.peek().clone();
                        let album_id = dispatch_album.clone();
                        spawn(async move {
                            let tracks = match album_tracks_in_order(&source, &album_id).await {
                                Ok(tracks) => tracks,
                                Err(error) => {
                                    report_lookup_failure(&album_id, &error);
                                    return;
                                }
                            };
                            // A genuinely empty album is a no-op, not a failure.
                            if tracks.is_empty() {
                                return;
                            }
                            if action == Action::PlayNext {
                                ctrl.queue_play_next(tracks);
                            } else {
                                ctrl.add_to_queue(tracks);
                            }
                        });
                    }
                    Action::AddToPlaylist => show_playlist_modal.set(true),
                    Action::GoToArtist => nav_ctrl.navigate_to_artist(dispatch_artist.clone()),
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
                on_close: move |_| show_playlist_modal.set(false),
                on_add_to_playlist: move |playlist_id: String| {
                    let source = active_source.peek().clone();
                    let album_id = add_album.clone();
                    spawn(async move {
                        let refs = match album_track_refs(&source, &album_id).await {
                            Ok(refs) => refs,
                            Err(error) => {
                                report_lookup_failure(&album_id, &error);
                                return;
                            }
                        };
                        if refs.is_empty() {
                            return;
                        }
                        match source.add_to_playlist(&playlist_id, &refs).await {
                            Ok(_) => gens.bump(Table::Playlists),
                            Err(error) => tracing::warn!(%error, "album: add to playlist failed"),
                        }
                    });
                    show_playlist_modal.set(false);
                },
                on_create_playlist: move |name: String| {
                    let source = active_source.peek().clone();
                    let album_id = create_album.clone();
                    spawn(async move {
                        let refs = match album_track_refs(&source, &album_id).await {
                            Ok(refs) => refs,
                            Err(error) => {
                                report_lookup_failure(&album_id, &error);
                                return;
                            }
                        };
                        if refs.is_empty() {
                            return;
                        }
                        match source.create_playlist(&name, &refs).await {
                            Ok(_) => gens.bump(Table::Playlists),
                            Err(error) => tracing::warn!(%error, "album: create playlist failed"),
                        }
                    });
                    show_playlist_modal.set(false);
                },
            }
        }
    }
}
