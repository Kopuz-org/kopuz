//! The album commands a card menu or a detail button issues.

use api::TrackInfo;
use gpui_kit::{App, ClipboardItem};
use link::Link;
use store::{Notices, actions};

/// Album order: by disc, then by track number.
pub fn in_album_order(tracks: &mut [TrackInfo]) {
    tracks.sort_by(|left, right| {
        left.disc_number
            .unwrap_or(1)
            .cmp(&right.disc_number.unwrap_or(1))
            .then_with(|| {
                left.track_number
                    .unwrap_or(0)
                    .cmp(&right.track_number.unwrap_or(0))
            })
    });
}

pub fn keys_of(tracks: &[TrackInfo]) -> Vec<String> {
    tracks
        .iter()
        .filter(|track| !track.key.is_empty())
        .map(|track| track.key.clone())
        .collect()
}

pub fn delete_tracks(keys: Vec<String>, cx: &mut App) {
    actions::run(
        cx,
        move |api| async move { api.delete_tracks(keys, true).await },
    );
}

/// The album's public page, falling back to its first track's; the daemon knows which sources have one.
pub fn share_album(browse_id: Option<String>, first_key: Option<String>, cx: &mut App) {
    let call = Link::global(cx).call(move |api| async move {
        if let Some(id) = browse_id
            && let Some(url) = api.album_web_url(id).await?
        {
            return Ok(Some(url));
        }
        match first_key {
            Some(key) => api.track_web_url(key).await,
            None => Ok(None),
        }
    });
    cx.spawn(async move |cx| {
        let url = call.await;
        cx.update(|cx| match url {
            Ok(Some(url)) => {
                cx.write_to_clipboard(ClipboardItem::new_string(url));
                // TODO(i18n): no en.ftl key for a generic share result yet.
                Notices::info("Link copied", cx);
            }
            Ok(None) => Notices::error("No public page for this album", cx),
            Err(error) => Notices::failed(&error, cx),
        });
    })
    .detach();
}
