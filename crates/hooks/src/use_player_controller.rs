use config::AppConfig;
use dioxus::{logger::tracing, prelude::*};
use player::engine::SourceFactory;
use player::player::Player;
use reader::Track;
use std::sync::Arc;
use std::time::Duration;
use utils;

use crate::playback_ref::ResolvedStreamRef;

use player::decoder;

#[path = "player_controller_metadata.rs"]
mod metadata;
#[path = "player_controller_playback.rs"]
mod playback;
#[path = "player_controller_spotify.rs"]
mod spotify;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LoopMode {
    None,
    Queue,
    Track,
}

/// What the UI intends to be playing. The `token` is the engine session token;
/// event consumers filter by it, which is what lets one signal replace the old
/// three-way cancellation (task cancel + engine cancel + generation bump).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum PlaybackIntent {
    Stopped,
    /// `from_token`: the session still playing during a crossfade resolve — a
    /// failed or reverted crossfade falls back to it.
    Loading {
        token: u64,
        idx: usize,
        crossfade: bool,
        from_token: u64,
    },
    Committed {
        token: u64,
    },
}

impl PlaybackIntent {
    pub(crate) fn token(self) -> u64 {
        match self {
            Self::Stopped => 0,
            Self::Loading { token, .. } | Self::Committed { token } => token,
        }
    }

    pub(crate) fn is_loading(self) -> bool {
        matches!(self, Self::Loading { .. })
    }
}

impl LoopMode {
    pub fn next(&self) -> Self {
        match self {
            LoopMode::None => LoopMode::Queue,
            LoopMode::Queue => LoopMode::Track,
            LoopMode::Track => LoopMode::None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct PlayerController {
    pub player: Signal<Player>,
    pub is_playing: Signal<bool>,
    /// Derived from the intent (plus the browse spinner) — read-only, so it
    /// can't be left stuck by a cancel path that forgets to clear it.
    pub is_loading: Memo<bool>,
    pub history: Signal<Vec<usize>>,
    pub queue: Signal<Vec<Track>>,
    pub shuffle: Signal<bool>,
    pub shuffle_order: Signal<Vec<usize>>,
    pub loop_mode: Signal<LoopMode>,
    pub current_queue_index: Signal<usize>,
    pub current_song_title: Signal<String>,
    pub current_song_artist: Signal<String>,
    pub current_song_album: Signal<String>,
    pub current_song_khz: Signal<u32>,
    pub current_song_bitrate: Signal<u16>,
    pub current_song_duration: Signal<u64>,
    pub current_song_progress: Signal<u64>,
    /// Byte ranges already fetched for the active network track. The UI draws
    /// these behind playback position, like a browser media seek bar.
    pub buffered_ranges: Signal<Vec<BufferedRange>>,
    buffer_progress_tx: Signal<tokio::sync::mpsc::UnboundedSender<BufferProgressEvent>>,
    pub current_song_cover_url: Signal<String>,
    pub current_track_snapshot: Signal<Option<Track>>,
    pub volume: Signal<f32>,
    pub config: Signal<AppConfig>,
    /// Storage handle (in a `Signal` so the controller stays `Copy`) — used by
    /// the still-`Db`-taking factories (`local`/`for_track`) the player calls.
    pub db: Signal<db::Db>,
    /// The cached active [`MediaSource`](::server::source::ActiveSource) — the
    /// player reads this shared handle to resolve streams instead of rebuilding
    /// the source (and its HTTP client) on every play/skip.
    pub active_source: Signal<::server::source::ActiveSource>,
    pub(crate) intent: Signal<PlaybackIntent>,
    /// Monotonic session-token allocator (0 = none).
    pub(crate) next_token: Signal<u64>,
    /// The current token as a plain `Signal` (not a memo) so the scrobble
    /// scheduler can `origin_scope` off it.
    pub(crate) current_token: Signal<u64>,
    /// The token a crossfade last armed for; cleared on seek so a fresh fade
    /// can arm at the outgoing track's real end.
    pub(crate) armed_transition: Signal<Option<u64>>,
    /// Discover tiles want the spinner shown synchronously on click, before any
    /// load intent exists; folded into `is_loading`, cleared by `set_intent`.
    pub browse_loading: Signal<bool>,
    pub(crate) pending_resume: Signal<Option<PendingResumeState>>,
    pub pending_crossfade_ui: Signal<Option<PendingCrossfadeUiState>>,
    pub radio_task: Signal<Option<dioxus_core::Task>>,
    /// The in-flight load pipeline (resolve → source factory → engine Load).
    /// Starting a new transition cancels the previous one, so a superseded
    /// load can never write back stale state.
    pub(crate) load_task: Signal<Option<dioxus_core::Task>>,
    pub station_registry: Signal<radio::registry::StationRegistry>,
    /// User-visible playback error. Set when something needs the user's
    /// attention (expired YT cookies, a failed stream resolve, …).
    /// Rendered as a banner by whoever subscribes — currently the
    /// settings popup error sink mirrors it on next open.
    pub playback_error: Signal<Option<String>>,
    /// The Spotify playback host (a browser tab running the Web Playback SDK),
    /// lazily started on the first Spotify play. `None` until then. Spotify audio
    /// plays in the browser, not the Symphonia engine, so its transport is routed
    /// here instead of to `player`.
    pub spotify_host: Signal<Option<::server::spotify::host::SpotifyHost>>,
    /// The SDK's Connect device id, set once the host reports `Ready`. Playback is
    /// started against it via the Web API; the device picker hides it from the
    /// remote-device list (it renders as the in-app entry).
    pub spotify_device: Signal<Option<String>>,
    /// A Spotify track URI waiting for the device to become ready (first play).
    pub(crate) spotify_pending_uri: Signal<Option<String>>,
    /// Whether the browser tab reported playback is allowed (autoplay probe or
    /// the enable-playback click). Plays issued before this just storm autoplay
    /// errors and can wedge the SDK, so the first play waits on it.
    pub(crate) spotify_activated: Signal<bool>,
    /// Spotify Connect device playback is routed to instead of the in-app SDK
    /// device (`None`). While set, transport goes through the Web API and a
    /// poll loop owns progress/auto-advance.
    pub spotify_device_override: Signal<Option<String>>,
    /// Millisecond-precision progress anchor for external playback: the last
    /// reported position and when it arrived. Reads interpolate elapsed time on
    /// top, so second-granular state ticks don't lag the lyric clock.
    pub(crate) spotify_progress_anchor: Signal<Option<(u64, std::time::Instant)>>,
    /// Whether a host launch is in flight — serializes rapid first plays so
    /// they can't spawn multiple hosts (and browser tabs).
    pub(crate) spotify_host_starting: Signal<bool>,
    /// Latest Spotify start request, canceled when superseded.
    pub(crate) spotify_start_task: Signal<Option<dioxus_core::Task>>,
    /// Track id kopuz last asked Spotify to play, and when.
    pub(crate) spotify_commanded: Signal<Option<(String, std::time::Instant)>>,
    /// Whether the current track is playing through the Spotify host rather than
    /// the engine — transport methods and the progress pump branch on this.
    pub(crate) external_active: Signal<bool>,
    /// Set once the user explicitly picks a playback target from the device
    /// panel (including "this app"). Suppresses automatic adoption of an
    /// externally active Connect device so it can't override a deliberate choice.
    pub(crate) spotify_device_chosen: Signal<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingResumeState {
    track_path: String,
    progress_secs: u64,
}

/// A crossfade whose UI hasn't committed to the incoming track yet — held until
/// the engine's `TrackSwitched`; a seek/prev before then reverts to `from_token`.
#[derive(Clone, Copy, Debug)]
pub struct PendingCrossfadeUiState {
    pub next_idx: usize,
    pub to_token: u64,
    pub from_token: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferedRange {
    pub start: u64,
    pub end: u64,
    pub total: u64,
}

#[derive(Clone, Copy, Debug)]
struct BufferProgressEvent {
    token: u64,
    start: u64,
    end: u64,
    total: Option<u64>,
}

fn merge_buffered_range(ranges: &mut Vec<BufferedRange>, incoming: BufferedRange) {
    if incoming.total == 0 || incoming.start >= incoming.end {
        return;
    }
    if ranges
        .first()
        .is_some_and(|range| range.total != incoming.total)
    {
        ranges.clear();
    }
    ranges.push(BufferedRange {
        end: incoming.end.min(incoming.total),
        ..incoming
    });
    ranges.sort_unstable_by_key(|range| range.start);

    let mut merged: Vec<BufferedRange> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
            continue;
        }
        merged.push(range);
    }
    *ranges = merged;
}

impl PlayerController {
    fn buffer_progress_callback(&self, token: u64) -> utils::stream_buffer::BufferProgressCallback {
        let progress_tx = self.buffer_progress_tx.peek().clone();
        Arc::new(move |start, end, total| {
            let _ = progress_tx.send(BufferProgressEvent {
                token,
                start,
                end,
                total,
            });
        })
    }

    fn track_key(track: &Track) -> String {
        track.id.uid().to_string()
    }

    pub(crate) fn shift_indices_at_or_after(indices: &mut [usize], at: usize, by: usize) {
        for idx in indices {
            if *idx >= at {
                *idx += by;
            }
        }
    }

    /// Retrieves the queue index for a given index, taking into account the shuffle state.
    pub fn get_queue_index(&self, idx: usize) -> Option<usize> {
        if *self.shuffle.peek() {
            self.shuffle_order.peek().get(idx).cloned()
        } else {
            Some(idx)
        }
    }

    /// Retrieves the current track index in the queue, taking into account the shuffle state.
    /// Useful when it is not required to be a reactive value
    pub fn get_current_track_index(&self) -> Option<usize> {
        self.get_queue_index(*self.current_queue_index.peek())
    }

    /// Retrieves the track at a given index in the queue, taking into account the shuffle state.
    pub fn get_track_at(&self, idx: usize) -> Option<Track> {
        let idx = self.get_queue_index(idx)?;
        self.queue.peek().get(idx).cloned()
    }

    /// Whether the queue is walking an album at `idx`: a neighbour in play
    /// order belongs to the same album. Shuffle scatters an album, so this
    /// reads false there, which is what ReplayGain's auto mode wants: a
    /// shuffled mix should level track to track.
    pub(crate) fn album_context_at(&self, idx: usize, album_id: &str) -> bool {
        if album_id.is_empty() {
            return false;
        }
        let shares_album = |other: usize| {
            self.get_track_at(other)
                .is_some_and(|t| t.album_id == album_id)
        };
        idx.checked_sub(1).is_some_and(shares_album) || shares_album(idx + 1)
    }

    /// Retrieves the current track
    pub fn current_track(&self) -> Option<Track> {
        self.get_track_at(*self.current_queue_index.peek())
    }

    /// Stamp a resolved stream's probed duration/bitrate (YT ships them late)
    /// onto the queue Track and, if it's still the shown track, the live
    /// signals — in a single `queue.write()`.
    /// Seek the current track. All progress-bar and lyric scrubbers route here.
    pub fn seek(&mut self, time: Duration) {
        if *self.external_active.peek() {
            if self.spotify_device_override.peek().is_some() {
                if let Some(access) = self.spotify_access() {
                    let ms = time.as_millis() as u64;
                    spawn(async move {
                        let _ = ::server::spotify::api::player_seek(&access, ms).await;
                    });
                }
            } else if let Some(host) = self.spotify_host.peek().clone() {
                host.seek(time.as_millis() as u64);
            }
            self.spotify_progress_anchor
                .set(Some((time.as_millis() as u64, std::time::Instant::now())));
            self.current_song_progress.set(time.as_secs());
            return;
        }
        // The scrub targets the visible track. During a crossfade that's the
        // outgoing session: revert the armed transition and seek it by its own
        // token, so a fade that just completed can't misdirect the seek.
        if let Some(from_token) = self.revert_transition() {
            self.player.peek().seek_for_session(time, from_token);
        } else {
            self.player.peek().seek(time);
        }
        self.current_song_progress.set(time.as_secs());
    }

    /// Zero for an external player: its position comes from the service, not us.
    pub fn output_latency_secs(&self) -> f64 {
        if *self.external_active.peek() {
            return 0.0;
        }
        self.player.peek().output_latency().as_secs_f64()
    }

    pub fn displayed_progress_secs_f64(&self) -> f64 {
        if *self.external_active.peek() {
            if let Some((ms, at)) = *self.spotify_progress_anchor.peek() {
                let mut pos = ms as f64 / 1000.0;
                if *self.is_playing.peek() {
                    pos += at.elapsed().as_secs_f64();
                }
                let dur = *self.current_song_duration.peek();
                if dur > 0 {
                    pos = pos.min(dur as f64);
                }
                return pos;
            }
            return *self.current_song_progress.peek() as f64;
        }
        // Mid-crossfade the bar shows the outgoing (fading) track's live position.
        if self.pending_crossfade_ui.peek().is_some()
            && let Some(fading) = self.player.peek().fading_position()
        {
            return fading.as_secs_f64();
        }
        self.player.peek().get_position().as_secs_f64()
    }
}

/// Factory for a resolved network stream (radio, YT range/sequential,
/// SoundCloud HLS, or a plain buffered stream). Returns a `SourceFactory` so the
/// symphonia types stay inferred inside the closure — hooks can't name them.
fn network_factory(
    stream_url: String,
    yt_format: Option<(::server::ytmusic::player::AudioFormat, bool)>,
    yt_user_agent: Option<String>,
    is_radio: bool,
    icy_tx: Option<tokio::sync::watch::Sender<utils::icy::IcyMeta>>,
    rt_handle: tokio::runtime::Handle,
    buffer_progress: Option<utils::stream_buffer::BufferProgressCallback>,
) -> SourceFactory {
    Box::new(move || {
        let build = || -> std::io::Result<_> {
            if is_radio {
                let stream = utils::stream_buffer::StreamBuffer::with_user_agent(
                    stream_url,
                    true,
                    yt_user_agent,
                    icy_tx,
                    rt_handle,
                );
                Ok(decoder::from_stream_with_hint(stream, "ogg"))
            } else if let Some((fmt, range_safe)) = yt_format {
                if range_safe {
                    // YT: HTTP Range-backed source. Symphonia can seek freely
                    // (Matroska Cues at the end, scrub anywhere) and startup
                    // probes only fetch the ~512 KiB they need.
                    let range = utils::range_source::RangeStreamSource::new_with_progress(
                        stream_url,
                        yt_user_agent,
                        buffer_progress,
                    )?;
                    let len = Some(range.total_size());
                    let (source, mut hint) = decoder::from_stream_with_len(range, len);
                    hint.with_extension(fmt.extension());
                    Ok((source, hint))
                } else {
                    // No-pot fallback: googlevideo 403s deep ranges, and the
                    // probe reads the webm tail — stream sequentially instead of
                    // failing outright (issue #386). No scrubbing.
                    let stream = utils::stream_buffer::StreamBuffer::with_user_agent_and_progress(
                        stream_url,
                        false,
                        yt_user_agent,
                        None,
                        rt_handle,
                        buffer_progress,
                    );
                    stream.wait_for_response_headers();
                    let len = stream.known_total_size();
                    let (source, mut hint) = decoder::from_stream_with_len(stream, len);
                    hint.with_extension(fmt.extension());
                    Ok((source, hint))
                }
            } else if let ResolvedStreamRef::SoundCloudHls(hls_url) =
                ResolvedStreamRef::parse(&stream_url)
            {
                // SoundCloud Go+ AAC: assemble the HLS playlist's fMP4 segments
                // into one in-memory buffer Symphonia can decode (no HLS demuxer).
                let bytes = utils::hls_source::assemble(hls_url, yt_user_agent.as_deref())?;
                let len = Some(bytes.len() as u64);
                let cursor = std::io::Cursor::new(bytes);
                let (source, mut hint) = decoder::from_stream_with_len(cursor, len);
                hint.with_extension("m4a");
                Ok((source, hint))
            } else if let ResolvedStreamRef::AppleMusicFmp4(payload) =
                ResolvedStreamRef::parse(&stream_url)
            {
                // Apple Music: Widevine key exchange, then fetch the encrypted
                // fMP4 and decrypt it into one in-memory buffer — same shape as
                // the SoundCloud path above. The key exchange has panicked on
                // malformed CDM responses, so it runs under catch_unwind rather
                // than taking the decode worker down with it.
                let (adam_id, storefront, language, token_b64) =
                    ResolvedStreamRef::apple_music_parts(payload).ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "malformed Apple Music stream ref",
                        )
                    })?;
                let token = String::from_utf8(
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, token_b64)
                        .unwrap_or_default(),
                )
                .unwrap_or_default();
                let (adam_id, storefront, language) = (
                    adam_id.to_string(),
                    storefront.to_string(),
                    language.to_string(),
                );
                let track = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    rt_handle.block_on(::server::applemusic::stream::resolve_and_decrypt(
                        &adam_id,
                        &token,
                        &storefront,
                        &language,
                        buffer_progress.clone(),
                    ))
                }))
                .unwrap_or_else(|panic| {
                    let msg = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    tracing::error!("am.playback: resolve_and_decrypt panicked: {msg}");
                    Err(format!("Apple Music decrypt panicked: {msg}"))
                })
                .map_err(std::io::Error::other)?;
                // Always seekable: samples decrypt on demand, so symphonia's probe
                // jumping to EOF for an `mfra` index costs the handful of samples
                // it actually reads rather than the whole track.
                let len = Some(track.total_size());
                let (source, mut hint) = decoder::from_stream_with_len(track, len);
                hint.with_extension("m4a");
                Ok((source, hint))
            } else {
                // Jellyfin and Subsonic/Navidrome normally support HTTP
                // ranges. Let format probes jump straight to tail metadata
                // instead of making a sequential buffer download everything
                // between the start and end of the file.
                match utils::range_source::RangeStreamSource::new_with_progress(
                    stream_url.clone(),
                    yt_user_agent.clone(),
                    buffer_progress.clone(),
                ) {
                    Ok(range) => {
                        let len = Some(range.total_size());
                        Ok(decoder::from_stream_with_len(range, len))
                    }
                    Err(error) => {
                        tracing::debug!(%error, "HTTP ranges unavailable; using progressive stream");
                        let stream =
                            utils::stream_buffer::StreamBuffer::with_user_agent_and_progress(
                                stream_url,
                                false,
                                yt_user_agent,
                                None,
                                rt_handle,
                                buffer_progress,
                            );
                        stream.wait_for_response_headers();
                        let len = stream.known_total_size();
                        Ok(decoder::from_stream_with_len(stream, len))
                    }
                }
            }
        };
        build().map_err(|e| e.to_string())
    })
}

#[allow(clippy::too_many_arguments)]
pub fn use_player_controller(
    player: Signal<Player>,
    is_playing: Signal<bool>,
    queue: Signal<Vec<Track>>,
    current_queue_index: Signal<usize>,
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_album: Signal<String>,
    current_song_khz: Signal<u32>,
    current_song_bitrate: Signal<u16>,
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
    current_song_cover_url: Signal<String>,
    current_track_snapshot: Signal<Option<Track>>,
    volume: Signal<f32>,
    config: Signal<AppConfig>,
    config_loaded_ok: Signal<bool>,
    db_handle: db::Db,
) -> PlayerController {
    let intent = use_signal(|| PlaybackIntent::Stopped);
    let next_token = use_signal(|| 0u64);
    let current_token = use_signal(|| 0u64);
    let buffered_ranges = use_signal(Vec::<BufferedRange>::new);
    let (progress_tx, progress_rx) = use_hook(|| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<BufferProgressEvent>();
        (tx, std::rc::Rc::new(std::cell::RefCell::new(Some(rx))))
    });
    let buffer_progress_tx = use_signal(move || progress_tx);
    let progress_rx_slot = progress_rx.clone();
    let mut buffered_ranges_sink = buffered_ranges;
    use_effect(move || {
        let Some(mut progress_rx) = progress_rx_slot.borrow_mut().take() else {
            return;
        };
        spawn(async move {
            while let Some(event) = progress_rx.recv().await {
                if *current_token.peek() != event.token {
                    continue;
                }
                let Some(total) = event.total.filter(|total| *total > 0) else {
                    continue;
                };
                buffered_ranges_sink.with_mut(|ranges| {
                    merge_buffered_range(
                        ranges,
                        BufferedRange {
                            start: event.start,
                            end: event.end,
                            total,
                        },
                    );
                });
            }
        });
    });
    let armed_transition = use_signal(|| None);
    let browse_loading = use_signal(|| false);
    let is_loading = use_memo(move || intent.read().is_loading() || *browse_loading.read());
    let history = use_signal(Vec::new);
    let shuffle = use_signal(|| false);
    let shuffle_order = use_signal(Vec::<usize>::new);
    let loop_mode = use_signal(|| LoopMode::None);
    let pending_resume = use_signal(|| None::<PendingResumeState>);
    let pending_crossfade_ui = use_signal(|| None::<PendingCrossfadeUiState>);
    let radio_task = use_signal(|| None::<dioxus_core::Task>);
    let load_task = use_signal(|| None::<dioxus_core::Task>);
    let station_registry = use_context::<Signal<radio::registry::StationRegistry>>();
    let playback_error = use_signal(|| None::<String>);
    let spotify_host = use_signal(|| None::<::server::spotify::host::SpotifyHost>);
    let spotify_device = use_signal(|| None::<String>);
    let spotify_pending_uri = use_signal(|| None::<String>);
    let spotify_activated = use_signal(|| false);
    let spotify_device_override = use_signal(|| None::<String>);
    let spotify_progress_anchor = use_signal(|| None::<(u64, std::time::Instant)>);
    let spotify_host_starting = use_signal(|| false);
    let spotify_start_task = use_signal(|| None::<dioxus_core::Task>);
    let spotify_commanded = use_signal(|| None::<(String, std::time::Instant)>);
    let external_active = use_signal(|| false);
    let spotify_device_chosen = use_signal(|| false);
    let db = use_signal(move || db_handle);
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();

    // Scrobbles queued while offline (issue #335): retry once on startup, in
    // case connectivity came back between sessions.
    let mut drained = use_signal(|| false);
    use_effect(move || {
        if !*config_loaded_ok.read() || *drained.peek() {
            return;
        }
        drained.set(true);
        let creds = {
            let cfg = config.peek();
            scrobble::queue::Credentials {
                lastfm: (!cfg.lastfm_api_key.is_empty() && !cfg.lastfm_api_secret.is_empty()).then(
                    || {
                        (
                            cfg.lastfm_api_key.clone(),
                            cfg.lastfm_api_secret.clone(),
                            cfg.lastfm_session_key.clone(),
                        )
                    },
                ),
                librefm_session_key: (!cfg.librefm_session_key.is_empty())
                    .then(|| cfg.librefm_session_key.clone()),
                listenbrainz_token: (!cfg.musicbrainz_token.trim().is_empty())
                    .then(|| cfg.musicbrainz_token.clone()),
            }
        };
        let db_handle = db.peek().clone();
        spawn(async move {
            scrobble::queue::drain(&db_handle, &creds).await;
        });
    });

    PlayerController {
        player,
        is_playing,
        is_loading,
        history,
        queue,
        shuffle,
        shuffle_order,
        loop_mode,
        current_queue_index,
        current_song_title,
        current_song_artist,
        current_song_album,
        current_song_khz,
        current_song_bitrate,
        current_song_duration,
        current_song_progress,
        buffered_ranges,
        buffer_progress_tx,
        current_song_cover_url,
        current_track_snapshot,
        volume,
        config,
        db,
        active_source,
        intent,
        next_token,
        current_token,
        armed_transition,
        browse_loading,
        pending_resume,
        pending_crossfade_ui,
        radio_task,
        load_task,
        station_registry,
        playback_error,
        spotify_host,
        spotify_device,
        spotify_pending_uri,
        spotify_activated,
        spotify_device_override,
        spotify_progress_anchor,
        spotify_host_starting,
        spotify_start_task,
        spotify_commanded,
        external_active,
        spotify_device_chosen,
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferedRange, merge_buffered_range};

    #[test]
    fn buffered_ranges_merge_adjacent_and_overlapping_chunks() {
        let mut ranges = Vec::new();
        for (start, end) in [(500, 750), (0, 250), (200, 500)] {
            merge_buffered_range(
                &mut ranges,
                BufferedRange {
                    start,
                    end,
                    total: 1_000,
                },
            );
        }

        assert_eq!(
            ranges,
            vec![BufferedRange {
                start: 0,
                end: 750,
                total: 1_000,
            }]
        );
    }

    #[test]
    fn buffered_ranges_preserve_gaps_and_reset_for_a_new_total() {
        let mut ranges = Vec::new();
        merge_buffered_range(
            &mut ranges,
            BufferedRange {
                start: 0,
                end: 100,
                total: 1_000,
            },
        );
        merge_buffered_range(
            &mut ranges,
            BufferedRange {
                start: 900,
                end: 1_000,
                total: 1_000,
            },
        );
        assert_eq!(ranges.len(), 2);

        merge_buffered_range(
            &mut ranges,
            BufferedRange {
                start: 0,
                end: 50,
                total: 500,
            },
        );
        assert_eq!(
            ranges,
            vec![BufferedRange {
                start: 0,
                end: 50,
                total: 500,
            }]
        );
    }
}
