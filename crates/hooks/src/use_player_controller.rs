use config::AppConfig;
use dioxus::logger::tracing::Instrument;
use dioxus::{logger::tracing, prelude::*};
use player::engine::{SourceFactory, Transition};
use player::player::{LoadArgs, NowPlayingMeta, Player};
use reader::Track;
use std::time::Duration;
use utils;

use crate::playback_ref::{PlaybackItemRef, ResolvedStreamRef};
use crate::scrobble_scheduler::{self, ScrobbleOptions};

use player::decoder;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LoopMode {
    None,
    Queue,
    Track,
}

/// What the UI intends to be playing, and the token that identifies it. The
/// engine session token is minted here (monotonic) and passed through Load; the
/// controller filters engine events by it, so a superseded or cancelled load's
/// events fall away instead of needing three separate cancellation mechanisms.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum PlaybackIntent {
    /// Nothing should be audible (end of queue, backend switch, failed load).
    Stopped,
    /// A load pipeline is in flight for `token`, targeting queue slot `idx`.
    Loading {
        token: u64,
        idx: usize,
        crossfade: bool,
    },
    /// The engine confirmed session `token`; the UI displays it (or, mid-fade,
    /// its predecessor).
    Committed { token: u64 },
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
    /// A track load is in flight. Derived from the intent (plus the browse
    /// spinner), so it is read-only and can never be left stuck by a bail or
    /// cancel — there is no setter to forget.
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
    /// What the UI intends to be playing; the single source of `is_loading` and
    /// the authority every cancellation site routes through.
    pub(crate) intent: Signal<PlaybackIntent>,
    /// Monotonic session-token allocator (0 = none).
    pub(crate) next_token: Signal<u64>,
    /// Mirror of the current intent's token as a plain `Signal` (not a memo) so
    /// the scrobble scheduler can scope tasks to it via `origin_scope`.
    pub(crate) current_token: Signal<u64>,
    /// The session token a crossfade last armed for; cleared on a seek so a
    /// fresh fade can arm at the outgoing track's real end.
    pub(crate) armed_transition: Signal<Option<u64>>,
    /// A browse action (Discover tile) wants the spinner shown synchronously on
    /// click, before any load intent exists. Folded into `is_loading`; cleared
    /// by `set_intent` when a real transition takes over.
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingResumeState {
    track_path: String,
    progress_secs: u64,
}

/// A crossfade whose UI hasn't yet committed to the incoming track. The UI keeps
/// showing the outgoing (from) session until the engine's `TrackSwitched`
/// confirms the fade completed; a seek/prev before then reverts to `from_token`.
#[derive(Clone, Copy, Debug)]
pub struct PendingCrossfadeUiState {
    pub next_idx: usize,
    pub to_token: u64,
    pub from_token: u64,
}

impl PlayerController {
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

    /// Retrieves the current track
    pub fn current_track(&self) -> Option<Track> {
        self.get_track_at(*self.current_queue_index.peek())
    }

    fn cover_url_for_track(&self, track: &Track) -> String {
        // Dispatch on the track's own source through the cover seam. Every track
        // self-describes its cover (a local row's path is projected from its album
        // by the DB read layer), so this sync path needs no album lookup.
        ::server::cover::track(&self.config.read(), track, 800)
            .map(|cover| cover.as_ref().to_string())
            .unwrap_or_else(|| utils::default_cover_url().as_ref().to_string())
    }

    pub(crate) fn clear_current_track_metadata(&mut self) {
        self.current_song_title.set(String::new());
        self.current_song_artist.set(String::new());
        self.current_song_album.set(String::new());
        self.current_song_khz.set(0);
        self.current_song_bitrate.set(0);
        self.current_song_duration.set(0);
        self.current_song_progress.set(0);
        self.current_song_cover_url.set(String::new());
        self.current_track_snapshot.set(None);
    }

    pub(crate) fn hydrate_current_track_metadata(&mut self, idx: usize, progress_secs: u64) {
        if let Some(track) = self.get_track_at(idx) {
            let progress_secs = progress_secs.min(track.duration);
            self.current_queue_index.set(idx);
            self.current_song_title.set(track.title.clone());
            self.current_song_artist.set(track.artist.clone());
            self.current_song_album.set(track.album.clone());
            self.current_song_khz.set(track.khz);
            self.current_song_bitrate.set(track.bitrate);
            self.current_song_duration.set(track.duration);
            self.current_song_progress.set(progress_secs);
            self.current_song_cover_url
                .set(self.cover_url_for_track(&track));
            self.current_track_snapshot.set(Some(track));
        } else {
            self.current_queue_index.set(0);
            self.clear_current_track_metadata();
        }
    }

    fn pending_resume_seek(&self, track: &Track) -> (Option<u64>, bool) {
        let pending = self.pending_resume.read().clone();
        let restore_seek_secs = pending.as_ref().and_then(|pending| {
            if pending.track_path == Self::track_key(track) {
                Some(pending.progress_secs.min(track.duration))
            } else {
                None
            }
        });

        (restore_seek_secs, pending.is_some())
    }

    pub(crate) fn clear_pending_crossfade_ui(&mut self) {
        self.pending_crossfade_ui.set(None);
    }

    pub(crate) fn schedule_pending_crossfade_ui(
        &mut self,
        next_idx: usize,
        to_token: u64,
        from_token: u64,
    ) {
        self.pending_crossfade_ui.set(Some(PendingCrossfadeUiState {
            next_idx,
            to_token,
            from_token,
        }));
    }

    /// The engine finished a fade (`TrackSwitched`): commit the deferred UI to
    /// the incoming track, if this switch is the one we armed.
    pub(crate) fn commit_transition(&mut self, token: u64) {
        let Some(pending) = *self.pending_crossfade_ui.peek() else {
            return;
        };
        if pending.to_token != token {
            return;
        }
        self.pending_crossfade_ui.set(None);
        let pos = self.player.peek().get_position().as_secs();
        self.hydrate_current_track_metadata(pending.next_idx, pos);
    }

    /// Undo an armed-but-uncommitted crossfade: the engine's seek/prev revives
    /// the outgoing (visible) session, so drop the deferred UI, re-enable
    /// arming, pop the history entry the arm pushed, and point the intent back
    /// at the outgoing session. Returns the outgoing token if a fade was reverted.
    pub(crate) fn revert_transition(&mut self) -> Option<u64> {
        let pending = (*self.pending_crossfade_ui.peek())?;
        self.clear_pending_crossfade_ui();
        self.armed_transition.set(None);
        let idx = *self.current_queue_index.peek();
        self.history.with_mut(|h| {
            if h.last() == Some(&idx) {
                h.pop();
            }
        });
        self.set_intent(PlaybackIntent::Committed {
            token: pending.from_token,
        });
        Some(pending.from_token)
    }

    pub(crate) fn set_pending_resume_for_track(&mut self, track: &Track, progress_secs: u64) {
        self.pending_resume.set(Some(PendingResumeState {
            track_path: Self::track_key(track),
            progress_secs: progress_secs.min(track.duration),
        }));
    }

    pub(crate) fn cancel_load_task(&mut self) {
        if let Some(task) = self.load_task.take() {
            task.cancel();
        }
        self.player.peek().cancel_pending_load();
    }

    /// Mint the next session token.
    pub(crate) fn allocate_token(&mut self) -> u64 {
        let token = self.next_token.peek().wrapping_add(1);
        self.next_token.set(token);
        token
    }

    /// The single writer of playback intent. Keeps `is_loading` and the
    /// `current_token` mirror in lockstep so no cancellation path can leave a
    /// stale loading flag or token behind.
    pub(crate) fn set_intent(&mut self, next: PlaybackIntent) {
        // A real transition supersedes any browse spinner; is_loading is derived
        // from the intent (Loading ⇒ true) via a memo, so nothing to set here.
        self.browse_loading.set(false);
        self.current_token.set(next.token());
        self.intent.set(next);
    }

    /// Report a load failure for `token`, unless a newer load already
    /// superseded it. Shared by the pipeline's two failure paths.
    pub(crate) fn fail_load(&mut self, token: u64, error: impl std::fmt::Display) {
        if self.intent.peek().token() != token {
            return;
        }
        self.playback_error
            .set(Some(format!("Couldn't load this track:\n{error}")));
        self.set_intent(PlaybackIntent::Stopped);
    }

    /// Seek the current track. All progress-bar / lyric scrubbers route here.
    /// The engine services seeks even after a track ended (the parked decode
    /// worker re-seeks in place), so no re-open dance is needed.
    pub fn seek(&mut self, time: Duration) {
        // During a crossfade the bar shows the outgoing track while the engine
        // fades in the next one. Seeking targets that visible track: revert the
        // armed transition (the engine's seek cancels the fade and revives the
        // outgoing session) and seek it by its own token so a fade that has just
        // completed can't land the seek on the wrong track.
        if let Some(from_token) = self.revert_transition() {
            self.player.peek().seek_for_session(time, from_token);
        } else {
            self.player.peek().seek(time);
        }
        self.current_song_progress.set(time.as_secs());
    }

    pub fn displayed_progress_secs_f64(&self) -> f64 {
        // During a crossfade the bar shows the outgoing (fading) track, whose
        // live position the engine reports; otherwise the current session's.
        if self.pending_crossfade_ui.peek().is_some()
            && let Some(fading) = self.player.peek().fading_position()
        {
            return fading.as_secs_f64();
        }
        self.player.peek().get_position().as_secs_f64()
    }

    /// Remap a queue index after moving one item within the queue.
    ///
    /// `index` is the position to remap, `from` is the original position of the moved item,
    /// and `to` is its destination after the move.
    ///
    /// Returns the new position for `index` after applying the move:
    /// - if `index == from`, this is the moved item itself, so it now lives at `to`
    /// - if the item moved forward (`from < to`), every item that was between `from + 1`
    ///   and `to` shifts left by one slot
    /// - if the item moved backward (`to < from`), every item that was between `to`
    ///   and `from - 1` shifts right by one slot
    /// - all other indices are unaffected
    pub(crate) fn remap_queue_index(index: usize, from: usize, to: usize) -> usize {
        if index == from {
            to
        } else if from < to && index > from && index <= to {
            index - 1
        } else if to < from && index >= to && index < from {
            index + 1
        } else {
            index
        }
    }

    pub fn should_crossfade(&self) -> bool {
        self.config.peek().crossfade_seconds > 0
            && *self.is_playing.peek()
            && self.player.peek().can_resume()
    }

    pub fn has_next_track(&self) -> bool {
        // Delegates to the same predicate the advance uses, so the crossfade
        // arm can't fire when the advance would instead end the queue.
        Self::has_following_track(
            *self.current_queue_index.peek(),
            self.queue.peek().len(),
            *self.loop_mode.peek(),
        )
    }

    pub fn play_track(&mut self, idx: usize) {
        let current_idx = *self.current_queue_index.peek();
        self.history.with_mut(|h| {
            if h.last() != Some(&current_idx) {
                h.push(current_idx);
            }
        });

        if *self.shuffle.peek() {
            // workaround: shuffle enable/disable needed to play the selected track when shuffle is enabled
            self.shuffle.set(false);
            self.play_track_no_history_without_crossfade(idx);
            self.shuffle.set(true);
            self.rebuild_shuffle_order();
        } else {
            self.play_track_no_history_without_crossfade(idx);
        }
    }

    pub fn play_track_no_history(&mut self, idx: usize) {
        self.play_track_no_history_with_transition(idx, false);
    }

    pub fn play_track_no_history_without_crossfade(&mut self, idx: usize) {
        self.play_track_no_history_with_transition(idx, false);
    }

    #[tracing::instrument(name = "player.transition", skip(self), fields(idx, crossfade = allow_crossfade))]
    pub(crate) fn play_track_no_history_with_transition(
        &mut self,
        idx: usize,
        allow_crossfade: bool,
    ) {
        // ── Phase 1: classify (no mutation beyond stale-cache eviction) ──
        // Every early bail below happens here, before any state is touched, so a
        // rejected request can't leave the loading flag or intent half-set.
        let Some(track) = self.get_track_at(idx) else {
            return;
        };

        let path_str = track.id.uid().to_string();
        let (restore_seek_secs, clear_pending_resume_on_success) = self.pending_resume_seek(&track);
        let use_crossfade = allow_crossfade
            && self.should_crossfade()
            && restore_seek_secs.is_none_or(|secs| secs == 0);
        let crossfade_duration = Duration::from_secs(self.config.peek().crossfade_seconds as u64);
        let item_ref = PlaybackItemRef::parse(&path_str);
        let is_radio_item = item_ref.is_radio();
        let is_server_item = item_ref.is_server();
        let id = item_ref.primary_id().unwrap_or_default().to_string();
        let stream_id = item_ref.stream_id().unwrap_or_default().to_string();

        // ── classify the source ─────────────────────────────────────────

        // Offline cache first (server items only).
        let offline_path: Option<std::path::PathBuf> = if is_server_item {
            let raw = self
                .config
                .read()
                .offline_tracks
                .get(&id)
                .map(std::path::PathBuf::from)
                .filter(|p| p.exists());
            // Evict stale entries saved with the wrong ".audio"/".bin" fallback
            if let Some(ref p) = raw {
                let bad_ext = matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("audio") | Some("bin")
                );
                if bad_ext {
                    let _ = std::fs::remove_file(p);
                    self.config.write().offline_tracks.remove(&id);
                    None
                } else {
                    raw
                }
            } else {
                raw
            }
        } else {
            None
        };

        // Remote stream reference + synchronous cover URL for server/radio
        // items that aren't cached offline. Streams resolve in the load task;
        // only the cover is built here so artwork shows immediately on click.
        let remote_ref: Option<(String, String)> = if offline_path.is_some() {
            None
        } else if is_radio_item {
            self.station_registry
                .read()
                .get(&id)
                .and_then(|s| s.streams.iter().find(|str| str.id == stream_id))
                .map(|s| s.url.clone())
                .map(|stream_url| (stream_url, String::new()))
        } else if is_server_item {
            // The stream itself is always resolved async in the load task (a URL
            // for Jellyfin/Subsonic, a deciphered stream for YT/SoundCloud), so
            // the ref is a pending marker for every server source. Only the cover
            // is built synchronously, through the shared cover seam rather than a
            // per-service match here. A server with no creds no longer bails
            // silently — resolve_stream returns a real error that surfaces as a
            // banner.
            if self.config.read().server.is_some() {
                let cover_url = ::server::cover::track(&self.config.read(), &track, 800)
                    .map(|cover| cover.as_ref().to_string())
                    .unwrap_or_default();
                Some((ResolvedStreamRef::pending_marker(&id), cover_url))
            } else {
                None
            }
        } else {
            None
        };

        let local_path: Option<std::path::PathBuf> = if is_server_item || is_radio_item {
            None
        } else {
            track.id.local_path().map(|p| p.to_path_buf())
        };

        // A server item with no server configured has nothing to resolve — stop
        // silently, as the old sync path did.
        if offline_path.is_none() && local_path.is_none() && remote_ref.is_none() {
            return;
        }

        // ── Phase 2: commit to loading this track ────────────────────────
        // Past this point state is mutated, so it only runs once the request is
        // known playable. Cancelling the prior load is a resource optimization
        // (the token guards correctness); the new intent sets is_loading.
        self.playback_error.set(None);
        self.cancel_radio_task();
        self.cancel_load_task();
        if !use_crossfade {
            self.clear_pending_crossfade_ui();
        }
        // The session currently playing (the crossfade's outgoing track); the
        // deferred UI reverts to it if the fade is cancelled by a seek/prev.
        let from_token = self.intent.peek().token();
        let token = self.allocate_token();
        self.set_intent(PlaybackIntent::Loading {
            token,
            idx,
            crossfade: use_crossfade,
        });

        let cover_url: String = if offline_path.is_some() {
            self.cover_url_for_track(&track)
        } else if let Some((_, cover)) = &remote_ref {
            cover.clone()
        } else {
            String::new()
        };
        let artwork = if is_server_item || is_radio_item {
            Some(cover_url.clone())
        } else {
            // For a local track `track.cover` is its album-art file path
            // (projected from the album by the DB read layer).
            track.cover.clone()
        };

        // ── UI transition ───────────────────────────────────────────────
        if !use_crossfade {
            if is_server_item || is_radio_item {
                // Deliberate UX: silence while a (possibly slow) load resolves.
                // Pure local files switch seamlessly inside the engine instead.
                self.player.peek().stop_for_transition();
                self.is_playing.set(false);
            }
            self.hydrate_current_track_metadata(idx, restore_seek_secs.unwrap_or(0));
            if is_server_item || is_radio_item {
                self.current_song_cover_url.set(cover_url.clone());
            }
        }

        // ── the load pipeline ───────────────────────────────────────────
        // One cancellable task for every source kind: resolve the stream URL
        // if needed, hand the engine a source factory (executed on its decode
        // worker thread, so network buffering never blocks the UI), then apply
        // the post-load bookkeeping once the engine confirms playback.
        let mut ctrl = *self;
        let phys_idx = self.get_queue_index(idx);
        let file_path = offline_path.or(local_path);
        let station_id = id;

        let task = spawn(
            async move {
                let factory: SourceFactory = if let Some(path) = file_path {
                    Box::new(move || decoder::open_file(&path).map_err(|e| e.to_string()))
                } else {
                    let (stream_ref, _) = remote_ref.expect("classified as remote above");
                    let (stream_url, yt_format, yt_user_agent) =
                        match ResolvedStreamRef::parse(&stream_ref) {
                            ResolvedStreamRef::Pending(item_id) => {
                                // The one genuinely per-source op: resolve the
                                // playable stream through the active source's
                                // backend (a URL for Jellyfin/Subsonic, a
                                // deciphered stream for YT).
                                let source = ctrl.active_source.peek().clone();
                                match source.resolve_stream(item_id).await {
                                    Ok(info) => {
                                        // YT carries probed duration/bitrate; stamp
                                        // them onto the queue Track + live signals.
                                        // (This task is cancelled when a newer load
                                        // starts, so the write-backs can't be stale.)
                                        if let Some(secs) = info.duration_secs
                                            && secs > 0
                                        {
                                            if let Some(p) = phys_idx
                                                && let Some(t) = ctrl.queue.write().get_mut(p)
                                            {
                                                t.duration = secs;
                                            }
                                            if *ctrl.current_queue_index.peek() == idx {
                                                ctrl.current_song_duration.set(secs);
                                            }
                                        }
                                        if let Some(bps) = info.bitrate {
                                            let kbps = (bps / 1000) as u16;
                                            if let Some(p) = phys_idx
                                                && let Some(t) = ctrl.queue.write().get_mut(p)
                                            {
                                                t.bitrate = kbps;
                                            }
                                            if *ctrl.current_queue_index.peek() == idx {
                                                ctrl.current_song_bitrate.set(kbps);
                                            }
                                        }
                                        (info.url, info.format, info.user_agent)
                                    }
                                    Err(e) => {
                                        tracing::error!(error = %e, "stream URL resolve failed");
                                        ctrl.fail_load(token, &e);
                                        return;
                                    }
                                }
                            }
                            ResolvedStreamRef::SoundCloudHls(_) | ResolvedStreamRef::Direct(_) => {
                                (stream_ref, None, None)
                            }
                        };

                    // The factory runs on the engine's decode worker (a plain
                    // thread with no runtime); StreamBuffer's download needs a
                    // runtime, so hand it a handle from this async task.
                    let rt_handle = tokio::runtime::Handle::current();
                    Box::new(move || {
                        let build = || -> std::io::Result<_> {
                            if is_radio_item {
                                let stream = utils::stream_buffer::StreamBuffer::with_user_agent(
                                    stream_url,
                                    true,
                                    yt_user_agent,
                                    rt_handle,
                                );
                                Ok(decoder::from_stream_with_hint(stream, "ogg"))
                            } else if let Some((fmt, range_safe)) = yt_format {
                                if range_safe {
                                    // YT: HTTP Range-backed source. Symphonia can
                                    // seek freely (Matroska Cues at the end, scrub
                                    // anywhere) and startup probes only fetch the
                                    // ~512 KiB they need.
                                    let range = utils::range_source::RangeStreamSource::new(
                                        stream_url,
                                        yt_user_agent,
                                    )?;
                                    let len = Some(range.total_size());
                                    let (source, mut hint) =
                                        decoder::from_stream_with_len(range, len);
                                    hint.with_extension(fmt.extension());
                                    Ok((source, hint))
                                } else {
                                    // No-pot fallback: googlevideo 403s deep ranges,
                                    // and the probe reads the webm tail — stream
                                    // sequentially instead of failing outright
                                    // (issue #386). No scrubbing.
                                    let stream =
                                        utils::stream_buffer::StreamBuffer::with_user_agent(
                                            stream_url,
                                            false,
                                            yt_user_agent,
                                            rt_handle,
                                        );
                                    stream.wait_for_total_size();
                                    let len = stream.known_total_size();
                                    let (source, mut hint) =
                                        decoder::from_stream_with_len(stream, len);
                                    hint.with_extension(fmt.extension());
                                    Ok((source, hint))
                                }
                            } else if let ResolvedStreamRef::SoundCloudHls(hls_url) =
                                ResolvedStreamRef::parse(&stream_url)
                            {
                                // SoundCloud Go+ AAC: assemble the HLS playlist's
                                // fMP4 segments into one in-memory buffer Symphonia
                                // can decode (it has no HLS demuxer).
                                let bytes =
                                    utils::hls_source::assemble(hls_url, yt_user_agent.as_deref())?;
                                let len = Some(bytes.len() as u64);
                                let cursor = std::io::Cursor::new(bytes);
                                let (source, mut hint) = decoder::from_stream_with_len(cursor, len);
                                hint.with_extension("m4a");
                                Ok((source, hint))
                            } else {
                                let stream = utils::stream_buffer::StreamBuffer::with_user_agent(
                                    stream_url,
                                    false,
                                    yt_user_agent,
                                    rt_handle,
                                );
                                stream.wait_for_total_size();
                                let len = stream.known_total_size();
                                Ok(decoder::from_stream_with_len(stream, len))
                            }
                        };
                        build().map_err(|e| e.to_string())
                    })
                };

                let meta = NowPlayingMeta {
                    title: track.title.clone(),
                    artist: track.artist.clone(),
                    album: track.album.clone(),
                    duration: std::time::Duration::from_secs(track.duration),
                    artwork,
                };
                let transition = if use_crossfade {
                    Transition::Crossfade(crossfade_duration)
                } else {
                    Transition::Immediate
                };
                let start_at = restore_seek_secs
                    .filter(|secs| *secs > 0)
                    .map(Duration::from_secs);
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                ctrl.player.write().load(LoadArgs {
                    token,
                    factory,
                    meta,
                    transition,
                    start_at,
                    reply: Some(reply_tx),
                });

                match reply_rx.await {
                    Ok(Ok(outcome)) => {
                        // The engine confirmed this session; the UI now owns it.
                        ctrl.set_intent(PlaybackIntent::Committed { token });
                        if clear_pending_resume_on_success {
                            ctrl.pending_resume.set(None);
                        }
                        if use_crossfade {
                            if outcome.crossfaded {
                                // Defer the UI to the incoming track until the
                                // engine's TrackSwitched confirms the fade ended.
                                ctrl.schedule_pending_crossfade_ui(idx, token, from_token);
                            } else {
                                // The engine fell back to an immediate switch
                                // (config mismatch, paused, drained outgoing) —
                                // commit the UI now instead of deferring to a
                                // fade midpoint that will never come.
                                ctrl.hydrate_current_track_metadata(idx, 0);
                            }
                        }

                        if is_radio_item {
                            if let Some(provider) =
                                ctrl.station_registry.read().create_provider(&station_id)
                            {
                                let mut current_song_title = ctrl.current_song_title;
                                let mut current_song_artist = ctrl.current_song_artist;
                                let mut current_song_album = ctrl.current_song_album;
                                let mut current_song_cover_url = ctrl.current_song_cover_url;
                                let task = spawn(async move {
                                    use radio::provider::RadioMetadataProvider;
                                    let mut rx = provider.start(&stream_id);
                                    while let Some(meta) = rx.recv().await {
                                        current_song_title.set(meta.title.clone());
                                        current_song_artist.set(meta.artist.clone());
                                        current_song_album.set(meta.station.clone());
                                        current_song_cover_url
                                            .set(meta.cover_url.unwrap_or_default());
                                    }
                                });
                                ctrl.radio_task.set(Some(task));
                            } else {
                                tracing::warn!(
                                    "[radio] No metadata provider for station: {}",
                                    station_id
                                );
                            }
                        } else {
                            let (item_id, source, options) = if is_server_item {
                                (
                                    Some(station_id.clone()),
                                    Some(ctrl.active_source),
                                    ScrobbleOptions::REMOTE_NATIVE,
                                )
                            } else {
                                (None, None, ScrobbleOptions::LOCAL)
                            };
                            scrobble_scheduler::schedule(
                                track.clone(),
                                item_id,
                                ctrl.config,
                                ctrl.current_token,
                                token,
                                ctrl.is_playing,
                                source,
                                options,
                                ctrl.db.peek().clone(),
                            );

                            // Server tracks: download the cover to a temp file so
                            // the OS media controls can show artwork (they need a
                            // local path, not a URL).
                            if is_server_item && !cover_url.is_empty() {
                                let cover_url = cover_url.clone();
                                let track = track.clone();
                                let mut player = ctrl.player;
                                let current_token = ctrl.current_token;
                                spawn(
                                    async move {
                                        if let Ok(response) = reqwest::get(&cover_url).await
                                            && let Ok(bytes) = response.bytes().await
                                        {
                                            let temp_dir = std::env::temp_dir();
                                            let random_id: u64 = rand::random();
                                            let file_path = temp_dir
                                                .join(format!("kopuz_cover_{}.jpg", random_id));

                                            if tokio::fs::write(&file_path, bytes).await.is_ok()
                                                && *current_token.read() == token
                                            {
                                                let path_str =
                                                    file_path.to_string_lossy().to_string();
                                                let new_meta = NowPlayingMeta {
                                                    title: track.title,
                                                    artist: track.artist,
                                                    album: track.album,
                                                    duration: std::time::Duration::from_secs(
                                                        track.duration,
                                                    ),
                                                    artwork: Some(path_str),
                                                };
                                                player.write().update_metadata(new_meta);
                                            }
                                        }
                                    }
                                    .instrument(tracing::info_span!("player.cover_fetch")),
                                );
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::error!(error = %e, "playback failed");
                        ctrl.fail_load(token, &e);
                    }
                    Err(_) => {
                        // Cancelled engine-side (superseded or stopped) — the
                        // token no longer matches, so any stray write-back is
                        // ignored; whichever flow cancelled owns the intent.
                    }
                }
            }
            .instrument(tracing::info_span!("player.load_pipeline", idx)),
        );

        self.load_task.set(Some(task));
    }
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
    }
}
