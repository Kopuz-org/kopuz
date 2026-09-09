//! Library mutations, from anywhere in the UI.
//!
//! Editing a tag, deleting a track and setting a cover each used to be spelled
//! out at the call site: take the active source, open the audio file, unlink
//! the path, tell the source, then bump a generation counter by hand. The
//! daemon does all of that and announces the change, so what is left here is
//! which track and what to change.
//!
//! A failure toasts rather than being swallowed: the inline versions only
//! bumped `if …is_ok()`, so a refused write looked like nothing happening.

use dioxus::prelude::*;

use crate::api::consume_api;
use crate::toast::toast_error;

/// Rewrite one track's tags, and its embedded cover with them.
pub fn edit_track(patch: api::TrackMetadataPatch) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.update_track_metadata(patch).await {
            tracing::warn!(%error, "saving track tags failed");
            toast_error(&error.to_string());
        }
    });
}

/// Forget these tracks. `from_disk` also unlinks the files, which the daemon
/// refuses outside the configured library roots.
pub fn delete_tracks(keys: Vec<String>, from_disk: bool) {
    if keys.is_empty() {
        return;
    }
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.delete_tracks(keys, from_disk).await {
            tracing::warn!(%error, "deleting tracks failed");
            toast_error(&error.to_string());
        }
    });
}

pub fn delete_album(id: String, from_disk: bool) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.delete_album(id, from_disk).await {
            tracing::warn!(%error, "deleting an album failed");
            toast_error(&error.to_string());
        }
    });
}

/// Set a picture. The bytes come from a file the user picked; the daemon
/// stores them and tells the source, so a server that hosts covers gets it.
pub fn upload_artwork(target: api::ArtworkTarget, content_type: String, bytes: Vec<u8>) {
    let api = consume_api();
    spawn(async move {
        let upload = api::ArtworkUpload {
            target,
            content_type,
            bytes,
        };
        if let Err(error) = api.upload_artwork(upload).await {
            tracing::warn!(%error, "setting artwork failed");
            toast_error(&error.to_string());
        }
    });
}

pub fn remove_artwork(target: api::ArtworkTarget) {
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.remove_artwork(target).await {
            tracing::warn!(%error, "removing artwork failed");
            toast_error(&error.to_string());
        }
    });
}

/// The content type for a picture the user picked, by extension.
///
/// The daemon decodes what it is given and rejects anything that is not an
/// image, so this only has to be right often enough to pick a file name.
pub fn content_type_for(path: &std::path::Path) -> String {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "image/jpeg",
    }
    .to_string()
}

/// The metadata editor's edits as a patch.
///
/// The editor always sends the whole desired state, so an absent number means
/// "remove it" rather than "leave it": that is what the explicit clear flags
/// are for, since a patch's `None` means unchanged.
pub fn patch_from_edits(key: String, edits: reader::models::TrackEdits) -> api::TrackMetadataPatch {
    api::TrackMetadataPatch {
        key,
        title: Some(edits.title),
        artist: Some(edits.artist),
        album: Some(edits.album),
        clear_track_number: edits.track_number.is_none(),
        track_number: edits.track_number,
        clear_disc_number: edits.disc_number.is_none(),
        disc_number: edits.disc_number,
        cover: match edits.cover {
            reader::models::CoverChange::Keep => api::ArtworkChange::Keep,
            reader::models::CoverChange::Remove => api::ArtworkChange::Remove,
            reader::models::CoverChange::Set(bytes) => api::ArtworkChange::Set(bytes),
        },
    }
}
