//! Commands a page or a row menu issues, each with the toast the Dioxus frontend showed.

use std::future::Future;
use std::sync::Arc;

use api::ApiError;
use api::{KopuzApi, QueueContext, QueueMode, SetQueueRequest};
use gpui_kit::App;
use link::Link;

use crate::notices::Notices;
use crate::player::PlayerModel;

/// Run a daemon call, toasting its failure; the answer is dropped.
pub fn run<T, Fut>(cx: &mut App, f: impl FnOnce(Arc<dyn KopuzApi>) -> Fut)
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    let call = Link::global(cx).call(f);
    cx.spawn(async move |cx| {
        if let Err(error) = call.await {
            cx.update(|cx| Notices::failed(&error, cx));
        }
    })
    .detach();
}

fn set_queue(request: SetQueueRequest, cx: &mut App) {
    run(cx, move |api| async move { api.set_queue(request).await });
}

/// Replace the queue with `keys` and start at `start`.
pub fn play_tracks(keys: Vec<String>, start: Option<u32>, cx: &mut App) {
    set_queue(
        SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks { keys },
            start_index: start,
            shuffle: None,
        },
        cx,
    );
}

/// A row click: jump within the queue when it already is this listing, so shuffle re-pins instead of starting over.
pub fn play_listing(keys: Vec<String>, index: usize, cx: &mut App) {
    let queue = crate::QueueModel::global(cx);
    if queue.read(cx).holds(&keys) {
        queue.update(cx, |queue, cx| queue.jump_physical(index as u32, cx));
    } else {
        play_tracks(keys, Some(index as u32), cx);
    }
}

/// Start at `key` with the whole library queued around it, or over `fallback` from `at` when the library lacks it.
pub fn play_in_library(key: String, fallback: Vec<String>, at: u32, cx: &mut App) {
    let filter = api::TrackFilter::default();
    let listing = filter.clone();
    let call = Link::global(cx).call(move |api| async move {
        let page = api.tracks(listing, crate::all()).await?;
        Ok(page.items.iter().position(|track| track.key == key))
    });
    cx.spawn(async move |cx| {
        let found = call.await;
        cx.update(|cx| match found {
            Ok(Some(index)) => {
                play_context(QueueContext::Filter { filter }, Some(index as u32), cx)
            }
            Ok(None) => play_tracks(fallback, Some(at), cx),
            Err(error) => Notices::failed(&error, cx),
        });
    })
    .detach();
}

/// Play a whole listing: from a random row while shuffle is on, from the top otherwise.
pub fn play_all(keys: Vec<String>, cx: &mut App) {
    let shuffled = PlayerModel::global(cx).read(cx).shuffle();
    let start = if shuffled && !keys.is_empty() {
        Some(pseudo_random(keys.len()))
    } else {
        None
    };
    play_tracks(keys, start, cx);
}

pub fn append(keys: Vec<String>, cx: &mut App) {
    set_queue(
        SetQueueRequest {
            mode: QueueMode::Append,
            context: QueueContext::Tracks { keys },
            start_index: None,
            shuffle: None,
        },
        cx,
    );
}

pub fn play_next(keys: Vec<String>, cx: &mut App) {
    set_queue(
        SetQueueRequest {
            mode: QueueMode::PlayNext,
            context: QueueContext::Tracks { keys },
            start_index: None,
            shuffle: None,
        },
        cx,
    );
}

/// Play something the daemon materializes itself: an album, an artist, a playlist, a station.
pub fn play_context(context: QueueContext, start: Option<u32>, cx: &mut App) {
    set_queue(
        SetQueueRequest {
            mode: QueueMode::Replace,
            context,
            start_index: start,
            shuffle: None,
        },
        cx,
    );
}

/// Start the source's mix from a track; gated by `capabilities().track_radio`.
pub fn start_track_radio(key: String, cx: &mut App) {
    Notices::info(i18n::t("radio_starting"), cx);
    start_radio(QueueContext::TrackRadio { key }, cx);
}

/// Start the source's mix from a playlist; gated by `capabilities().playlist_radio`.
pub fn start_playlist_radio(id: String, cx: &mut App) {
    Notices::info(i18n::t("radio_starting"), cx);
    start_radio(QueueContext::PlaylistRadio { id }, cx);
}

/// Start the source's mix from an album; gated by `capabilities().album_radio`.
pub fn start_album_radio(id: String, cx: &mut App) {
    Notices::info(i18n::t("radio_starting"), cx);
    start_radio(QueueContext::AlbumRadio { id }, cx);
}

/// Start the source's mix from an artist; gated by `capabilities().artist_radio`.
pub fn start_artist_radio(artist: String, cx: &mut App) {
    Notices::info(i18n::t("radio_starting"), cx);
    start_radio(QueueContext::ArtistRadio { artist }, cx);
}

/// Start the source's mix from several tracks; gated by `capabilities().mix_radio`.
pub fn start_mix_radio(keys: Vec<String>, cx: &mut App) {
    Notices::info(i18n::t("radio_starting"), cx);
    start_radio(QueueContext::MixRadio { keys }, cx);
}

/// Start the source's mix from everything in the queue; gated by `capabilities().mix_radio`.
pub fn start_queue_mix(cx: &mut App) {
    let keys = crate::QueueModel::global(cx)
        .read(cx)
        .items()
        .iter()
        .map(|track| track.key.clone())
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return;
    }

    start_mix_radio(keys, cx);
}

/// Remember a track as the start of a sonic path; the next "to here" ends it.
pub fn mark_sonic_path_start(key: String, cx: &mut App) {
    crate::SonicPathStart::global(cx).update(cx, |start, cx| start.set(key, cx));
    Notices::info(i18n::t("sonic_path_marked"), cx);
}

/// End the remembered sonic path at `to`, forgetting its start.
pub fn finish_sonic_path(to: String, cx: &mut App) {
    let from = crate::SonicPathStart::global(cx).update(cx, |start, cx| start.take(cx));
    if let Some(from) = from {
        start_sonic_path(from, to, cx);
    }
}

/// Queue the songs stepping from one track to another; gated by `capabilities().sonic_path`.
pub fn start_sonic_path(from: String, to: String, cx: &mut App) {
    Notices::info(i18n::t("radio_starting"), cx);
    start_radio(QueueContext::SonicPath { from, to }, cx);
}

fn start_radio(context: QueueContext, cx: &mut App) {
    let call = Link::global(cx).call(move |api| async move {
        api.set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context,
            start_index: Some(0),
            shuffle: None,
        })
        .await
    });
    cx.spawn(async move |cx| match call.await {
        Ok(ack) if ack.basis == Some(api::MixBasis::Metadata) => {
            cx.update(|cx| Notices::info(i18n::t("radio_metadata_basis"), cx));
        }
        Ok(_) => {}
        Err(_) => cx.update(|cx| Notices::error(i18n::t("radio_failed"), cx)),
    })
    .detach();
}

/// Copy a track's public page to the clipboard, or say the source has none.
pub fn share_track(key: String, cx: &mut App) {
    let call = Link::global(cx).call(move |api| async move { api.track_web_url(key).await });
    cx.spawn(async move |cx| {
        let url = call.await;
        cx.update(|cx| match url {
            Ok(Some(url)) => {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(url));
                Notices::info(i18n::t("share_copied"), cx);
            }
            Ok(None) => Notices::error(i18n::t("share_no_page"), cx),
            Err(error) => Notices::failed(&error, cx),
        });
    })
    .detach();
}

/// Copy an album's public page, falling back to its first track's; the daemon knows which sources have one.
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
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(url));
                Notices::info(i18n::t("share_copied"), cx);
            }
            Ok(None) => Notices::error(i18n::t("share_no_album_page"), cx),
            Err(error) => Notices::failed(&error, cx),
        });
    })
    .detach();
}

/// Tell the source to stop suggesting this track, then move on from it whether or not it listened.
pub fn dont_recommend(key: String, cx: &mut App) {
    let call = Link::global(cx).call(move |api| async move { api.dont_recommend(key).await });
    cx.spawn(async move |cx| {
        if let Err(error) = call.await {
            cx.update(|cx| Notices::failed(&error, cx));
        }
        cx.update(|cx| PlayerModel::global(cx).update(cx, |player, cx| player.next(cx)));
    })
    .detach();
}

/// Drop a queued row, with six seconds to put it back.
pub fn queue_remove(index: u32, cx: &mut App) {
    let removed = crate::QueueModel::global(cx).update(cx, |queue, cx| queue.remove(index, cx));
    let Some(track) = removed else { return };
    let key = track.key.clone();
    Notices::undo(
        i18n::t_with(
            "removed_from_queue",
            &[("title", track.title.clone().into())],
        ),
        move |cx| {
            let key = key.clone();
            crate::QueueModel::global(cx)
                .update(cx, |queue, cx| queue.insert(index, vec![key], cx));
        },
        cx,
    );
}

/// Favourite or unfavourite a whole selection; taking hearts away is offered back.
pub fn set_favorites(keys: Vec<String>, favorite: bool, cx: &mut App) {
    if keys.is_empty() {
        return;
    }
    crate::FavoritesModel::global(cx).update(cx, |favorites, cx| {
        for key in keys.clone() {
            favorites.set(key, favorite, cx);
        }
    });
    if favorite {
        return;
    }
    let undone = keys.clone();
    Notices::undo(
        i18n::t_with("unfavorited_count", &[("count", keys.len().into())]),
        move |cx| {
            let keys = undone.clone();
            crate::FavoritesModel::global(cx).update(cx, |favorites, cx| {
                for key in keys {
                    favorites.set(key, true, cx);
                }
            });
        },
        cx,
    );
}

/// Give each track `stars` (0 clears) on the active source; a refusal is toasted.
pub fn set_star_rating(keys: Vec<String>, stars: u8, cx: &mut App) {
    for key in keys {
        run(cx, move |api| async move {
            api.set_star_rating(key, stars).await
        });
    }
}

/// A random index below `len`, from the hasher's per-process seed.
fn pseudo_random(len: usize) -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let value = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    (value % len as u64) as u32
}
