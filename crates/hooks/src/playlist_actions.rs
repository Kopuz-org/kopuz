//! Playlist mutations, from anywhere in the UI.
//!
//! Every "add to playlist" menu in the app used to spell the same thing out:
//! take the active source, call it, and bump a generation by hand if it
//! worked. The daemon owns the operation and reports the change itself, so
//! what is left at a call site is which playlist and which tracks.
//!
//! A failure toasts rather than being swallowed: the previous inline versions
//! only bumped `if …is_ok()`, so a rejected write looked like nothing
//! happening at all.

use dioxus::prelude::*;

use crate::api::consume_api;
use crate::toast::toast_error;

/// Add tracks to an existing playlist. Empty keys are a no-op, matching the
/// old call sites, which all guarded on it.
pub fn add_tracks(playlist_id: String, keys: Vec<String>) {
    if keys.is_empty() {
        return;
    }
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.add_playlist_tracks(playlist_id, keys).await {
            tracing::warn!(%error, "adding to a playlist failed");
            toast_error(&error.to_string());
        }
    });
}

/// Create a playlist seeded with `keys`.
pub fn create_with(name: String, keys: Vec<String>) {
    if keys.is_empty() {
        return;
    }
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.create_playlist(name, keys).await {
            tracing::warn!(%error, "creating a playlist failed");
            toast_error(&error.to_string());
        }
    });
}

pub fn rename(playlist_id: String, name: String) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.rename_playlist(playlist_id, name).await {
            tracing::warn!(%error, "renaming a playlist failed");
            toast_error(&error.to_string());
        }
    });
}

pub fn delete(playlist_id: String) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.delete_playlist(playlist_id).await {
            tracing::warn!(%error, "deleting a playlist failed");
            toast_error(&error.to_string());
        }
    });
}

/// Remove one entry by position; a playlist may hold the same track twice.
pub fn remove_track(playlist_id: String, index: usize) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.remove_playlist_track(playlist_id, index as u32).await {
            tracing::warn!(%error, "removing from a playlist failed");
            toast_error(&error.to_string());
        }
    });
}

pub fn reorder(playlist_id: String, from: usize, to: usize) {
    let api = consume_api();
    spawn(async move {
        let reorder = api::PlaylistReorder {
            from: from as u32,
            to: to as u32,
        };
        if let Err(error) = api.reorder_playlist(playlist_id, reorder).await {
            tracing::warn!(%error, "reordering a playlist failed");
            toast_error(&error.to_string());
        }
    });
}

pub fn create_folder(name: String) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.create_playlist_folder(name).await {
            tracing::warn!(%error, "creating a folder failed");
            toast_error(&error.to_string());
        }
    });
}

/// Create a folder and put a playlist straight into it -- one user action, so
/// the playlist does not briefly exist outside the folder it was made for.
/// `then` runs whichever way it goes, for closing the picker.
pub fn create_folder_with(name: String, playlist_id: String, then: impl FnOnce() + 'static) {
    let api = consume_api();
    spawn(async move {
        let result = match api.create_playlist_folder(name).await {
            Ok(folder) => api.move_playlist(playlist_id, Some(folder)).await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            tracing::warn!(%error, "creating a folder for a playlist failed");
            toast_error(&error.to_string());
        }
        then();
    });
}

pub fn rename_folder(folder_id: String, name: String) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.rename_playlist_folder(folder_id, name).await {
            tracing::warn!(%error, "renaming a folder failed");
            toast_error(&error.to_string());
        }
    });
}

pub fn delete_folder(folder_id: String) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.delete_playlist_folder(folder_id).await {
            tracing::warn!(%error, "deleting a folder failed");
            toast_error(&error.to_string());
        }
    });
}

/// Move a playlist into a folder, or out of every folder with `None`.
pub fn move_to_folder(playlist_id: String, folder_id: Option<String>) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.move_playlist(playlist_id, folder_id).await {
            tracing::warn!(%error, "moving a playlist failed");
            toast_error(&error.to_string());
        }
    });
}
