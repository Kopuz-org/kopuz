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

/// The library keys of the rows a view has picked out. Selections are tracked
/// by uid, which is what tells two rows apart; a key is what names a row to the
/// daemon, and only the row itself knows both.
pub fn keys_for_uids(tracks: &[api::TrackInfo], uids: &[String]) -> Vec<String> {
    uids.iter()
        .filter_map(|uid| tracks.iter().find(|track| &track.uid == uid))
        .map(|track| track.key.clone())
        .filter(|key| !key.is_empty())
        .collect()
}

/// Every track key of an album, in album order, then whatever the caller
/// wanted with them. The album's own tracks are a daemon read, so an action
/// menu does not need the list on screen to act on it.
pub fn with_album_keys(album_id: String, then: impl FnOnce(Vec<String>) + 'static) {
    let api = consume_api();
    spawn(async move {
        let page = match api
            .album_tracks(
                album_id,
                api::Page {
                    offset: 0,
                    limit: u32::MAX,
                },
            )
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, "reading an album's tracks failed");
                toast_error(&error.to_string());
                return;
            }
        };
        let mut tracks = page.items;
        tracks.sort_by(|left, right| {
            left.track_number
                .cmp(&right.track_number)
                .then_with(|| left.title.cmp(&right.title))
        });
        then(tracks.into_iter().map(|track| track.key).collect());
    });
}
