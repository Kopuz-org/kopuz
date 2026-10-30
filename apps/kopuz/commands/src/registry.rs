//! The list. Order here is the order a menu draws and the order the palette falls back to.

use api::PlaylistCapability;
use components::format::t;
use components::icons;
use gpui_kit::App;
use store::{Kind, Navigator, Route, Target, actions};

use crate::ops;
use crate::{Binding, Command, Group, Section};

const TRACKS: &[Kind] = &[Kind::Tracks, Kind::QueueItem];
const ONLY_TRACKS: &[Kind] = &[Kind::Tracks];
const ALBUM: &[Kind] = &[Kind::Album];
const ARTIST: &[Kind] = &[Kind::Artist];
const PLAYLIST: &[Kind] = &[Kind::Playlist];
const FOLDER: &[Kind] = &[Kind::Folder];
const QUEUE_ITEM: &[Kind] = &[Kind::QueueItem];
const NOTHING: &[Kind] = &[Kind::None];
const NO_KEYS: &[Binding] = &[];

const SHELL: &str = "Shell";
const LIST: &str = "TrackList";

fn yes(_: &Target, _: &App) -> bool {
    true
}

fn has_tracks(target: &Target, _: &App) -> bool {
    !ops::track_keys(target).is_empty()
}

fn one_track(target: &Target, _: &App) -> bool {
    ops::track_keys(target).len() == 1
}

fn many_tracks(target: &Target, cx: &App) -> bool {
    ops::track_keys(target).len() > 1 && ops::caps(cx).edit_tags
}

pub fn all() -> &'static [Command] {
    COMMANDS
}

pub fn get(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.id == id)
}

static COMMANDS: &[Command] = &[
    // Tracks, in the order the shared row menu has always drawn them.
    Command {
        id: "track.play",
        labels: &["play"],
        label: |_, _| t("play"),
        icon: Some(icons::PLAY),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "enter",
            context: LIST,
        }],
        danger: false,
        applies: ONLY_TRACKS,
        available: has_tracks,
        run: |target, _, cx| actions::play_tracks(ops::track_keys(&target), Some(0), cx),
        action: Some(|| Box::new(components::track_list::ActivateRow)),
    },
    Command {
        id: "track.play_next",
        labels: &["play_next"],
        label: |_, _| t("play_next"),
        icon: Some(icons::QUEUE_NEXT),
        group: Group::Queue,
        section: Section::Act,
        keys: &[Binding {
            keystroke: "shift-enter",
            context: LIST,
        }],
        danger: false,
        applies: TRACKS,
        available: has_tracks,
        run: |target, _, cx| actions::play_next(ops::track_keys(&target), cx),
        action: Some(|| Box::new(components::track_list::PlayNextSelection)),
    },
    Command {
        id: "track.add_to_queue",
        labels: &["add_to_queue"],
        label: |_, _| t("add_to_queue"),
        icon: Some(icons::QUEUE_ADD),
        group: Group::Queue,
        section: Section::Act,
        keys: &[Binding {
            keystroke: "ctrl-enter",
            context: LIST,
        }],
        danger: false,
        applies: TRACKS,
        available: has_tracks,
        run: |target, _, cx| actions::append(ops::track_keys(&target), cx),
        action: Some(|| Box::new(components::track_list::QueueSelection)),
    },
    Command {
        id: "track.add_to_playlist",
        labels: &["add_to_playlist"],
        label: |_, _| t("add_to_playlist"),
        icon: Some(icons::PLUS),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: TRACKS,
        // Ungated here once, so a source with no playlists still offered to add to one.
        available: |target, cx| {
            has_tracks(target, cx) && ops::caps(cx).playlists != PlaylistCapability::None
        },
        run: |target, window, cx| {
            crate::playlist_picker::open(ops::track_keys(&target), window, cx)
        },
        action: None,
    },
    Command {
        id: "track.favorite",
        labels: &["add_to_favorites", "remove_from_favorites"],
        label: |target, cx| match ops::all_favorite(&ops::track_keys(target), cx) {
            true => t("remove_from_favorites"),
            false => t("add_to_favorites"),
        },
        icon: Some(icons::HEART),
        group: Group::Library,
        section: Section::Act,
        keys: &[Binding {
            keystroke: "ctrl-d",
            context: LIST,
        }],
        danger: false,
        applies: TRACKS,
        available: has_tracks,
        run: |target, _, cx| ops::toggle_favorite(&target, cx),
        action: Some(|| Box::new(components::track_list::FavoriteSelection)),
    },
    Command {
        id: "track.radio",
        labels: &["start_radio"],
        label: |_, _| t("start_radio"),
        icon: Some(icons::RADIO),
        group: Group::Playback,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: TRACKS,
        available: |target, cx| one_track(target, cx) && ops::caps(cx).track_radio,
        run: |target, _, cx| {
            if let Some(key) = ops::first_key(&target) {
                actions::start_track_radio(key, cx);
            }
        },
        action: None,
    },
    Command {
        id: "track.download",
        labels: &["download_offline", "remove_download"],
        label: |target, cx| match ops::all_stored(&ops::track_keys(target), cx) {
            true => t("remove_download"),
            false => t("download_offline"),
        },
        icon: Some(icons::DOWNLOAD),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: TRACKS,
        available: |target, cx| has_tracks(target, cx) && ops::caps(cx).downloads,
        run: |target, _, cx| ops::toggle_download(&target, cx),
        action: None,
    },
    Command {
        id: "track.go_to_artist",
        labels: &["go_to_artist"],
        label: |_, _| t("go_to_artist"),
        icon: Some(icons::MIC),
        group: Group::Navigate,
        section: Section::Go,
        keys: NO_KEYS,
        danger: false,
        applies: TRACKS,
        available: one_track,
        run: |target, _, cx| ops::go_to_artist(&target, cx),
        action: None,
    },
    Command {
        id: "track.go_to_album",
        labels: &["go_to_album"],
        label: |_, _| t("go_to_album"),
        icon: Some(icons::DISC),
        group: Group::Navigate,
        section: Section::Go,
        keys: NO_KEYS,
        danger: false,
        applies: TRACKS,
        available: one_track,
        run: |target, _, cx| ops::go_to_album(&target, cx),
        action: None,
    },
    Command {
        id: "track.share",
        labels: &["share"],
        label: |_, _| t("share"),
        icon: Some(icons::SHARE),
        group: Group::Navigate,
        section: Section::Go,
        keys: NO_KEYS,
        danger: false,
        applies: TRACKS,
        available: one_track,
        run: |target, _, cx| {
            if let Some(key) = ops::first_key(&target) {
                actions::share_track(key, cx);
            }
        },
        action: None,
    },
    Command {
        id: "track.view_metadata",
        labels: &["view_metadata"],
        label: |_, _| t("view_metadata"),
        icon: Some(icons::EDIT),
        group: Group::View,
        section: Section::Go,
        keys: NO_KEYS,
        danger: false,
        applies: TRACKS,
        available: one_track,
        // Whether the tags can be written is the source's answer, not a constant.
        run: |target, window, cx| {
            if let Some(key) = ops::first_key(&target) {
                let editable = ops::caps(cx).edit_tags;
                store::TRACK_METADATA.open((key, editable), window, cx);
            }
        },
        action: None,
    },
    Command {
        id: "track.edit_metadata_batch",
        labels: &["edit_metadata"],
        label: |_, _| t("edit_metadata"),
        icon: Some(icons::EDIT),
        group: Group::View,
        section: Section::Go,
        keys: NO_KEYS,
        danger: false,
        applies: TRACKS,
        available: many_tracks,
        run: |target, window, cx| {
            store::BATCH_METADATA.open(ops::track_keys(&target), window, cx);
        },
        action: None,
    },
    Command {
        id: "track.move_up",
        labels: &["move_up"],
        label: |_, _| t("move_up"),
        icon: Some(icons::CHEVRON_UP),
        group: Group::Queue,
        section: Section::Edit,
        keys: NO_KEYS,
        danger: false,
        applies: ONLY_TRACKS,
        available: |target, _| {
            target
                .row()
                .is_some_and(|row| row.actions.reorder && row.index > 0)
        },
        run: |target, _, cx| {
            if let Some(row) = target.row() {
                row.reorder(row.index.saturating_sub(1), cx);
            }
        },
        action: None,
    },
    Command {
        id: "track.move_down",
        labels: &["move_down"],
        label: |_, _| t("move_down"),
        icon: Some(icons::CHEVRON_DOWN),
        group: Group::Queue,
        section: Section::Edit,
        keys: NO_KEYS,
        danger: false,
        applies: ONLY_TRACKS,
        available: |target, _| {
            target
                .row()
                .is_some_and(|row| row.actions.reorder && row.index + 1 < row.rows)
        },
        run: |target, _, cx| {
            if let Some(row) = target.row() {
                row.reorder(row.index + 1, cx);
            }
        },
        action: None,
    },
    Command {
        id: "track.remove_from_playlist",
        labels: &["remove_from_playlist"],
        label: |_, _| t("remove_from_playlist"),
        icon: Some(icons::CLOSE),
        group: Group::Library,
        section: Section::Remove,
        keys: NO_KEYS,
        danger: false,
        applies: ONLY_TRACKS,
        available: |target, _| {
            target
                .row()
                .is_some_and(|row| row.actions.remove_from_playlist)
        },
        run: |target, _, cx| {
            if let Some(row) = target.row() {
                row.remove_from_playlist(cx);
            }
        },
        action: None,
    },
    Command {
        id: "track.delete",
        labels: &["delete"],
        label: |_, _| t("delete"),
        icon: Some(icons::TRASH),
        group: Group::Library,
        section: Section::Remove,
        keys: &[Binding {
            keystroke: "delete",
            context: LIST,
        }],
        danger: true,
        applies: ONLY_TRACKS,
        available: |target, cx| {
            has_tracks(target, cx) && target.row().is_some_and(|row| row.actions.delete)
        },
        run: |target, _, cx| ops::delete_tracks(&target, cx),
        action: Some(|| Box::new(components::track_list::RemoveSelection)),
    },
    // Albums.
    Command {
        id: "album.play",
        labels: &["play"],
        label: |_, _| t("play"),
        icon: Some(icons::PLAY),
        group: Group::Playback,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: ALBUM,
        available: yes,
        run: |target, _, cx| ops::play_album(&target, cx),
        action: None,
    },
    Command {
        id: "album.add_to_queue",
        labels: &["add_all_to_queue"],
        label: |_, _| t("add_all_to_queue"),
        icon: Some(icons::QUEUE_ADD),
        group: Group::Queue,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: ALBUM,
        available: yes,
        run: |target, _, cx| {
            if let Some(id) = ops::album_id(&target) {
                ops::with_album_keys(id, cx, actions::append);
            }
        },
        action: None,
    },
    Command {
        id: "album.add_to_playlist",
        labels: &["add_all_to_playlist"],
        label: |_, _| t("add_all_to_playlist"),
        icon: Some(icons::PLUS),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: ALBUM,
        available: |_, cx| ops::caps(cx).playlists != PlaylistCapability::None,
        run: |target, window, cx| {
            let Some(id) = ops::album_id(&target) else {
                return;
            };
            let handle = window.window_handle();
            ops::with_album_keys(id, cx, move |keys, cx| {
                handle
                    .update(cx, |_, window, cx| {
                        crate::playlist_picker::open(keys, window, cx)
                    })
                    .ok();
            });
        },
        action: None,
    },
    Command {
        id: "album.download",
        labels: &["download_offline"],
        label: |_, _| t("download_offline"),
        icon: Some(icons::DOWNLOAD),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: ALBUM,
        available: |_, cx| ops::caps(cx).downloads,
        run: |target, _, cx| {
            if let Some(id) = ops::album_id(&target) {
                ops::with_album_keys(id, cx, |keys, cx| {
                    store::DownloadsModel::global(cx)
                        .update(cx, |downloads, cx| downloads.set_offline(keys, true, cx))
                });
            }
        },
        action: None,
    },
    Command {
        id: "album.remove_download",
        labels: &["remove_download"],
        label: |_, _| t("remove_download"),
        icon: Some(icons::DOWNLOAD),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: ALBUM,
        available: |_, cx| ops::caps(cx).downloads,
        run: |target, _, cx| {
            if let Some(id) = ops::album_id(&target) {
                ops::with_album_keys(id, cx, |keys, cx| {
                    store::DownloadsModel::global(cx)
                        .update(cx, |downloads, cx| downloads.set_offline(keys, false, cx))
                });
            }
        },
        action: None,
    },
    Command {
        id: "album.delete",
        labels: &["delete_album", "remove_from_cache"],
        label: |_, cx| match ops::caps(cx).delete_from_disk {
            true => t("delete_album"),
            false => t("remove_from_cache"),
        },
        icon: Some(icons::TRASH),
        group: Group::Library,
        section: Section::Remove,
        keys: NO_KEYS,
        // Deleting an album off disk used to happen on the first click, with nothing asked.
        danger: true,
        applies: ALBUM,
        available: yes,
        run: |target, _, cx| ops::delete_album(&target, cx),
        action: None,
    },
    // Artists.
    Command {
        id: "artist.play",
        labels: &["play"],
        label: |_, _| t("play"),
        icon: Some(icons::PLAY),
        group: Group::Playback,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: ARTIST,
        available: yes,
        run: |target, _, cx| ops::play_artist(&target, cx),
        action: None,
    },
    Command {
        id: "artist.add_to_queue",
        labels: &["add_all_to_queue"],
        label: |_, _| t("add_all_to_queue"),
        icon: Some(icons::QUEUE_ADD),
        group: Group::Queue,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: ARTIST,
        available: yes,
        run: |target, _, cx| {
            if let Target::Artist { key, .. } = &target {
                ops::with_artist_keys(key.clone(), cx, actions::append);
            }
        },
        action: None,
    },
    // Playlists.
    Command {
        id: "playlist.play",
        labels: &["play"],
        label: |_, _| t("play"),
        icon: Some(icons::PLAY),
        group: Group::Playback,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: PLAYLIST,
        available: yes,
        run: |target, _, cx| ops::play_playlist(&target, cx),
        action: None,
    },
    Command {
        id: "playlist.add_to_queue",
        labels: &["add_all_to_queue"],
        label: |_, _| t("add_all_to_queue"),
        icon: Some(icons::QUEUE_ADD),
        group: Group::Queue,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: PLAYLIST,
        available: |target, cx| {
            ops::playlist_id(target).is_some_and(|id| !ops::playlist_keys(&id, cx).is_empty())
        },
        run: |target, _, cx| {
            if let Some(id) = ops::playlist_id(&target) {
                actions::append(ops::playlist_keys(&id, cx), cx);
            }
        },
        action: None,
    },
    Command {
        id: "playlist.radio",
        labels: &["start_radio"],
        label: |_, _| t("start_radio"),
        icon: Some(icons::RADIO),
        group: Group::Playback,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: PLAYLIST,
        available: |_, cx| ops::caps(cx).playlist_radio,
        run: |target, _, cx| {
            if let Some(id) = ops::playlist_id(&target) {
                actions::start_playlist_radio(id, cx);
            }
        },
        action: None,
    },
    Command {
        id: "playlist.move_to_folder",
        labels: &["move_to_folder"],
        label: |_, _| t("move_to_folder"),
        icon: Some(icons::FOLDER),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: PLAYLIST,
        available: |_, cx| ops::caps(cx).folders,
        run: |target, window, cx| {
            if let Some(id) = ops::playlist_id(&target) {
                store::FOLDER_PICKER.open(id, window, cx);
            }
        },
        action: None,
    },
    Command {
        id: "playlist.remove_from_folder",
        labels: &["remove_from_folder"],
        label: |_, _| t("remove_from_folder"),
        icon: Some(icons::CLOSE),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: PLAYLIST,
        // A card had to be told it was inside a folder; the catalog already knows.
        available: |target, cx| ops::playlist_id(target).is_some_and(|id| ops::in_folder(&id, cx)),
        run: |target, _, cx| ops::remove_from_folder(&target, cx),
        action: None,
    },
    Command {
        id: "playlist.rename",
        labels: &["rename_playlist"],
        label: |_, _| t("rename_playlist"),
        icon: Some(icons::EDIT),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: PLAYLIST,
        available: |_, cx| ops::caps(cx).playlists != PlaylistCapability::None,
        run: |target, window, cx| ops::rename_playlist(&target, window, cx),
        action: None,
    },
    Command {
        id: "playlist.delete",
        labels: &["delete_playlist"],
        label: |_, _| t("delete_playlist"),
        icon: Some(icons::TRASH),
        group: Group::Library,
        section: Section::Remove,
        keys: NO_KEYS,
        danger: true,
        applies: PLAYLIST,
        available: |_, cx| ops::caps(cx).playlists != PlaylistCapability::None,
        run: |target, _, cx| ops::delete_playlist(&target, cx),
        action: None,
    },
    // Folders.
    Command {
        id: "folder.rename",
        labels: &["rename_folder"],
        label: |_, _| t("rename_folder"),
        icon: Some(icons::EDIT),
        group: Group::Library,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: FOLDER,
        available: yes,
        run: |target, window, cx| ops::rename_folder(&target, window, cx),
        action: None,
    },
    Command {
        id: "folder.delete",
        labels: &["delete_folder"],
        label: |_, _| t("delete_folder"),
        icon: Some(icons::TRASH),
        group: Group::Library,
        section: Section::Remove,
        keys: NO_KEYS,
        danger: true,
        applies: FOLDER,
        available: yes,
        run: |target, _, cx| ops::delete_folder(&target, cx),
        action: None,
    },
    // Queue rows: the track entries above apply too, these are what only a row of the queue has.
    Command {
        id: "queue.play",
        labels: &["play"],
        label: |_, _| t("play"),
        icon: Some(icons::PLAY),
        group: Group::Playback,
        section: Section::Act,
        keys: NO_KEYS,
        danger: false,
        applies: QUEUE_ITEM,
        available: yes,
        run: |target, _, cx| ops::queue_jump(&target, cx),
        action: None,
    },
    Command {
        id: "queue.move_up",
        labels: &["move_up"],
        label: |_, _| t("move_up"),
        icon: Some(icons::CHEVRON_UP),
        group: Group::Queue,
        section: Section::Edit,
        keys: NO_KEYS,
        danger: false,
        applies: QUEUE_ITEM,
        available: |target, _| ops::queue_index(target).is_some_and(|index| index > 0),
        run: |target, _, cx| ops::queue_move(&target, -1, cx),
        action: None,
    },
    Command {
        id: "queue.move_down",
        labels: &["move_down"],
        label: |_, _| t("move_down"),
        icon: Some(icons::CHEVRON_DOWN),
        group: Group::Queue,
        section: Section::Edit,
        keys: NO_KEYS,
        danger: false,
        applies: QUEUE_ITEM,
        available: |target, cx| {
            let count = store::QueueModel::global(cx).read(cx).len() as u32;
            ops::queue_index(target).is_some_and(|index| index + 1 < count)
        },
        run: |target, _, cx| ops::queue_move(&target, 1, cx),
        action: None,
    },
    Command {
        id: "queue.remove",
        labels: &["remove"],
        label: |_, _| t("remove"),
        icon: Some(icons::CLOSE),
        group: Group::Queue,
        section: Section::Remove,
        keys: NO_KEYS,
        danger: false,
        applies: QUEUE_ITEM,
        available: yes,
        run: |target, _, cx| ops::queue_remove(&target, cx),
        action: None,
    },
    // The app itself: no target, driven by key or palette.
    Command {
        id: "player.toggle",
        labels: &["play_pause"],
        label: |_, _| t("play_pause"),
        icon: Some(icons::PLAY),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "space",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| store::PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx)),
        action: None,
    },
    Command {
        id: "player.next",
        labels: &["next_track"],
        label: |_, _| t("next_track"),
        icon: Some(icons::SKIP_FORWARD),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-right",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| store::PlayerModel::global(cx).update(cx, |player, cx| player.next(cx)),
        action: Some(|| Box::new(crate::keys::NextTrack)),
    },
    Command {
        id: "player.previous",
        labels: &["previous_track"],
        label: |_, _| t("previous_track"),
        icon: Some(icons::SKIP_BACK),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-left",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| store::PlayerModel::global(cx).update(cx, |player, cx| player.previous(cx)),
        action: Some(|| Box::new(crate::keys::PreviousTrack)),
    },
    Command {
        id: "player.seek_forward",
        labels: &["seek_forward"],
        label: |_, _| t("seek_forward"),
        icon: Some(icons::CHEVRON_RIGHT),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "right",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| ops::seek(ops::SEEK_STEP_MS as i64, cx),
        action: Some(|| Box::new(crate::keys::SeekForward)),
    },
    Command {
        id: "player.seek_backward",
        labels: &["seek_backward"],
        label: |_, _| t("seek_backward"),
        icon: Some(icons::CHEVRON_LEFT),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "left",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| ops::seek(-(ops::SEEK_STEP_MS as i64), cx),
        action: Some(|| Box::new(crate::keys::SeekBackward)),
    },
    Command {
        id: "player.volume_up",
        labels: &["volume_up"],
        label: |_, _| t("volume_up"),
        icon: Some(icons::VOLUME),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-up",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| ops::nudge_volume(ops::VOLUME_STEP, cx),
        action: Some(|| Box::new(crate::keys::VolumeUp)),
    },
    Command {
        id: "player.volume_down",
        labels: &["volume_down"],
        label: |_, _| t("volume_down"),
        icon: Some(icons::VOLUME_LOW),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-down",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| ops::nudge_volume(-ops::VOLUME_STEP, cx),
        action: Some(|| Box::new(crate::keys::VolumeDown)),
    },
    Command {
        id: "player.mute",
        labels: &["toggle_mute"],
        label: |_, _| t("toggle_mute"),
        icon: Some(icons::VOLUME_MUTED),
        group: Group::Playback,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "m",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| {
            store::PlayerModel::global(cx).update(cx, |player, cx| player.toggle_mute(cx))
        },
        action: Some(|| Box::new(crate::keys::ToggleMute)),
    },
    Command {
        id: "view.lyrics",
        labels: &["lyrics"],
        label: |_, _| t("lyrics"),
        icon: Some(icons::LYRICS),
        group: Group::View,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "l",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| ops::toggle_panel(store::SidePanel::Lyrics, cx),
        action: Some(|| Box::new(crate::keys::ToggleLyrics)),
    },
    Command {
        id: "view.queue",
        labels: &["up_next"],
        label: |_, _| t("up_next"),
        icon: Some(icons::LIST),
        group: Group::View,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "q",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| ops::toggle_panel(store::SidePanel::Queue, cx),
        action: Some(|| Box::new(crate::keys::ToggleQueue)),
    },
    Command {
        id: "view.fullscreen",
        labels: &["toggle_fullscreen"],
        label: |_, _| t("toggle_fullscreen"),
        icon: Some(icons::EXPAND),
        group: Group::View,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "f",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| ops::toggle_fullscreen(cx),
        action: Some(|| Box::new(crate::keys::ToggleFullscreen)),
    },
    Command {
        id: "view.mini_player",
        labels: &["mini_player"],
        label: |_, _| t("mini_player"),
        icon: Some(icons::COLLAPSE),
        group: Group::View,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-m",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| ops::toggle_mini_player(cx),
        action: Some(|| Box::new(crate::keys::ToggleMiniPlayer)),
    },
    Command {
        id: "app.palette",
        labels: &["command_palette"],
        label: |_, _| t("command_palette"),
        icon: Some(icons::SEARCH),
        group: Group::App,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-k",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| {
            store::Chrome::global(cx).update(cx, |chrome, cx| {
                let open = chrome.is_quick_search_open();
                chrome.set_quick_search(!open, cx);
            })
        },
        action: None,
    },
    Command {
        id: "view.filter",
        labels: &["filter_view"],
        label: |_, _| t("filter_view"),
        icon: Some(icons::SEARCH),
        group: Group::View,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "/",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, _| tracing::warn!("no view owns a filter field yet"),
        action: None,
    },
    Command {
        id: "nav.back",
        labels: &["go_back"],
        label: |_, _| t("go_back"),
        icon: Some(icons::CHEVRON_LEFT),
        group: Group::Navigate,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "alt-left",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| Navigator::global(cx).update(cx, |navigator, cx| navigator.go_back(cx)),
        action: None,
    },
    Command {
        id: "nav.forward",
        labels: &["go_forward"],
        label: |_, _| t("go_forward"),
        icon: Some(icons::CHEVRON_RIGHT),
        group: Group::Navigate,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "alt-right",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| Navigator::global(cx).update(cx, |navigator, cx| navigator.go_forward(cx)),
        action: Some(|| Box::new(crate::keys::GoForward)),
    },
    Command {
        id: "nav.home",
        labels: &["home"],
        label: |_, _| t("home"),
        icon: Some(icons::HOME),
        group: Group::Navigate,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-1",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| Navigator::navigate(Route::Home, cx),
        action: Some(|| Box::new(crate::keys::GoHome)),
    },
    Command {
        id: "nav.library",
        labels: &["library"],
        label: |_, _| t("library"),
        icon: Some(icons::LIBRARY),
        group: Group::Navigate,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-2",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| Navigator::navigate(Route::Library, cx),
        action: Some(|| Box::new(crate::keys::GoLibrary)),
    },
    Command {
        id: "nav.discover",
        labels: &["discover"],
        label: |_, _| t("discover"),
        icon: Some(icons::COMPASS),
        group: Group::Navigate,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-3",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| Navigator::navigate(Route::Discover, cx),
        action: Some(|| Box::new(crate::keys::GoDiscover)),
    },
    Command {
        id: "nav.radio",
        labels: &["radio"],
        label: |_, _| t("radio"),
        icon: Some(icons::RADIO),
        group: Group::Navigate,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-4",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| Navigator::navigate(Route::Radio, cx),
        action: Some(|| Box::new(crate::keys::GoRadio)),
    },
    Command {
        id: "downloads.add_url",
        labels: &["downloader_add_url"],
        label: |_, _| t("downloader_add_url"),
        icon: Some(icons::DOWNLOAD),
        group: Group::Library,
        section: Section::Hidden,
        keys: NO_KEYS,
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, window, cx| store::URL_DOWNLOAD.open((), window, cx),
        action: None,
    },
    Command {
        id: "app.shortcuts",
        labels: &["keyboard_shortcuts"],
        label: |_, _| t("keyboard_shortcuts"),
        icon: Some(icons::SETTINGS),
        group: Group::App,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "?",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, window, cx| store::SHORTCUTS.open((), window, cx),
        action: Some(|| Box::new(crate::keys::ShowShortcuts)),
    },
    Command {
        id: "app.settings",
        labels: &["settings"],
        label: |_, _| t("settings"),
        icon: Some(icons::SETTINGS),
        group: Group::App,
        section: Section::Hidden,
        keys: &[Binding {
            keystroke: "ctrl-,",
            context: SHELL,
        }],
        danger: false,
        applies: NOTHING,
        available: yes,
        run: |_, _, cx| Navigator::navigate(Route::Settings { section: None }, cx),
        action: Some(|| Box::new(crate::keys::OpenSettings)),
    },
    Command {
        id: "app.add_source",
        labels: &["add_source"],
        label: |_, _| t("add_source"),
        icon: Some(icons::PLUS),
        group: Group::App,
        section: Section::Hidden,
        keys: NO_KEYS,
        danger: false,
        applies: NOTHING,
        available: |_, cx| store::SetupModel::global(cx).read(cx).is_decided(),
        run: |_, _, cx| store::SetupModel::start_over(cx),
        action: None,
    },
];
