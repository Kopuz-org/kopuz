//! "Start radio" as one action, shared by every surface that offers it.
//!
//! A track seed and a playlist seed are the same operation from the UI's side:
//! hand the source an id, get a generated queue back, play it. Keeping the
//! label, the icon, the capability gate and the failure handling in one place is
//! what stops the track row and the playlist surfaces from drifting apart.

use std::sync::Arc;

use api::KopuzApi;
use dioxus::prelude::*;
use hooks::PlayerController;
use hooks::toast::{toast, toast_error};
use reader::models::Track;
use server::source::ActiveSource;
use std::future::Future;
use tracing::Instrument;

pub const RADIO_ICON: &str = "fa-solid fa-tower-broadcast";

pub fn radio_label() -> String {
    i18n::t("start_radio").to_string()
}

/// Await a radio queue and hand it to the player. An empty or failed mix leaves
/// the current queue alone, so the user's music keeps playing rather than
/// stopping on a network hiccup.
///
/// Both outcomes toast. Building a mix is a remote round trip that can take tens
/// of seconds (a Subsonic server without analysis-backed similarity resolves it
/// through Last.fm), so without the opening notice a click looks like it did
/// nothing, and without the failure notice a timeout is indistinguishable from
/// success.
fn spawn_radio<F>(seed: String, fut: F, mut ctrl: PlayerController)
where
    F: Future<Output = Result<Vec<Track>, api::ApiError>> + 'static,
{
    spawn(
        async move {
            toast(&i18n::t("radio_starting"));
            match fut.await {
                Ok(tracks) if !tracks.is_empty() => ctrl.play_queue_at(tracks, 0),
                Ok(_) => {
                    tracing::warn!(seed = %seed, "radio returned empty queue");
                    toast_error(&i18n::t("radio_empty"));
                }
                Err(e) => {
                    tracing::warn!(seed = %seed, error = %e, "radio failed");
                    toast_error(&i18n::t("radio_failed"));
                }
            }
        }
        .instrument(tracing::info_span!("radio.start")),
    );
}

/// Start radio seeded from a track and play the generated queue. The radio
/// operation lives in the source layer ([`MediaSource::start_radio`](server::source::MediaSource::start_radio));
/// this just resolves the track's id and hands the result to the player.
pub fn play_track_radio(track: Track, api: Arc<dyn KopuzApi>, ctrl: PlayerController) {
    let seed = track.id.key().into_owned();
    let fetch = seed.clone();
    spawn_radio(
        seed,
        async move {
            api.track_radio(fetch).await.map(|tracks| {
                tracks
                    .into_iter()
                    .map(hooks::use_db_queries::track_from_api)
                    .collect()
            })
        },
        ctrl,
    );
}

/// Start radio seeded from a playlist. What comes back is the source's mix for
/// that playlist, not its track list, so it replaces the queue outright.
pub fn play_playlist_radio(playlist_id: String, api: Arc<dyn KopuzApi>, ctrl: PlayerController) {
    let seed = playlist_id.clone();
    spawn_radio(
        seed,
        async move {
            api.playlist_radio(playlist_id).await.map(|tracks| {
                tracks
                    .into_iter()
                    .map(hooks::use_db_queries::track_from_api)
                    .collect()
            })
        },
        ctrl,
    );
}

/// The `on_start_radio` handler for a track row: `Some` iff the active source
/// supports radio ([`Capabilities::radio`](server::source::Capabilities)), else
/// `None` (so the row hides the "Start radio" action). Lets every call site wire
/// radio in one line without repeating the capability gate or context plumbing.
///
/// Reads context via `consume_context`, never a `use_*` hook: call sites invoke
/// this once per visible row, so a hook here would register a per-row-count
/// number of hooks and panic the parent on rules-of-hooks when the row count
/// changes (e.g. an empty server library filling in after a sync).
pub fn track_radio_handler(track: Track) -> Option<EventHandler<()>> {
    let ctrl = consume_context::<PlayerController>();
    let active_source = consume_context::<Signal<ActiveSource>>();
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let supported = active_source.read().capabilities().radio.track;
    supported
        .then(|| EventHandler::new(move |_| play_track_radio(track.clone(), api.clone(), ctrl)))
}

/// The playlist counterpart of [`track_radio_handler`]. Same `consume_context`
/// rationale, since playlist cards are rendered in a loop too.
///
/// Gated on its own flag: a Subsonic server can seed radio from a song but not
/// from a playlist, so sharing the track flag put an action on Navidrome
/// playlist cards that could only return `Unsupported`.
pub fn playlist_radio_handler(playlist_id: String) -> Option<EventHandler<()>> {
    let ctrl = consume_context::<PlayerController>();
    let active_source = consume_context::<Signal<ActiveSource>>();
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let supported = active_source.read().capabilities().radio.playlist;
    supported.then(|| {
        EventHandler::new(move |_| play_playlist_radio(playlist_id.clone(), api.clone(), ctrl))
    })
}
