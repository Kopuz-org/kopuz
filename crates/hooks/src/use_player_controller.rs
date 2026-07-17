use config::{AppConfig, MusicService};
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
    /// Whether the current track is playing through the Spotify host rather than
    /// the engine — transport methods and the progress pump branch on this.
    pub(crate) external_active: Signal<bool>,
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

    /// Stamp a resolved stream's probed duration/bitrate (YT ships them late)
    /// onto the queue Track and, if it's still the shown track, the live
    /// signals — in a single `queue.write()`.
    fn stamp_probed_stream_info(
        &mut self,
        phys_idx: Option<usize>,
        idx: usize,
        duration_secs: Option<u64>,
        bitrate: Option<u32>,
    ) {
        let duration = duration_secs.filter(|s| *s > 0);
        let kbps = bitrate.map(|bps| (bps / 1000) as u16);

        if let Some(p) = phys_idx
            && let Some(track) = self.queue.write().get_mut(p)
        {
            if let Some(secs) = duration {
                track.duration = secs;
            }
            if let Some(k) = kbps {
                track.bitrate = k;
            }
        }
        if *self.current_queue_index.peek() == idx {
            if let Some(secs) = duration {
                self.current_song_duration.set(secs);
            }
            if let Some(k) = kbps {
                self.current_song_bitrate.set(k);
            }
        }
    }

    /// Follow a radio station's live now-playing metadata into the UI signals
    /// for as long as it plays.
    fn start_radio_metadata(&mut self, station_id: String, stream_id: String) {
        let Some(provider) = self.station_registry.read().create_provider(&station_id) else {
            tracing::warn!("[radio] no metadata provider for station: {station_id}");
            return;
        };
        let mut current_song_title = self.current_song_title;
        let mut current_song_artist = self.current_song_artist;
        let mut current_song_album = self.current_song_album;
        let mut current_song_cover_url = self.current_song_cover_url;
        let task = spawn(async move {
            use radio::provider::RadioMetadataProvider;
            let mut rx = provider.start(&stream_id);
            while let Some(meta) = rx.recv().await {
                current_song_title.set(meta.title.clone());
                current_song_artist.set(meta.artist.clone());
                current_song_album.set(meta.station.clone());
                current_song_cover_url.set(meta.cover_url.unwrap_or_default());
            }
        });
        self.radio_task.set(Some(task));
    }

    /// Download a server track's cover to a temp file and hand the OS media
    /// controls its local path (they need a path, not a URL). No-ops if `token`
    /// is superseded before the download finishes.
    fn spawn_server_artwork_fetch(&self, cover_url: String, track: Track, token: u64) {
        let mut player = self.player;
        let current_token = self.current_token;
        spawn(
            async move {
                if let Ok(response) = reqwest::get(&cover_url).await
                    && let Ok(bytes) = response.bytes().await
                {
                    let file_path = std::env::temp_dir()
                        .join(format!("kopuz_cover_{}.jpg", rand::random::<u64>()));
                    if tokio::fs::write(&file_path, bytes).await.is_ok()
                        && *current_token.read() == token
                    {
                        player.write().update_metadata(NowPlayingMeta {
                            title: track.title,
                            artist: track.artist,
                            album: track.album,
                            duration: std::time::Duration::from_secs(track.duration),
                            artwork: Some(file_path.to_string_lossy().to_string()),
                        });
                    }
                }
            }
            .instrument(tracing::info_span!("player.cover_fetch")),
        );
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

    /// Commit the deferred crossfade UI to the incoming track, on the engine's
    /// `TrackSwitched` for the switch we armed.
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
        // Push the incoming track's now-playing metadata, deferred from load.
        self.player.peek().commit_now_playing();
    }

    /// Undo an armed crossfade at either stage — load still resolving (cancel
    /// it) or fade running (drop the deferred UI; the caller's tokened seek
    /// revives the outgoing session engine-side). Pops the history entry the arm
    /// pushed and reverts the intent to the outgoing token, returned on success.
    pub(crate) fn revert_transition(&mut self) -> Option<u64> {
        // Read both stage markers out before any signal write below.
        let fading = (*self.pending_crossfade_ui.peek()).map(|p| p.from_token);
        let resolving = match *self.intent.peek() {
            PlaybackIntent::Loading {
                crossfade: true,
                from_token,
                ..
            } => Some(from_token),
            _ => None,
        };

        let from_token = if let Some(from_token) = fading {
            self.clear_pending_crossfade_ui();
            from_token
        } else if let Some(from_token) = resolving {
            self.cancel_load_task();
            from_token
        } else {
            return None;
        };

        self.armed_transition.set(None);
        let idx = *self.current_queue_index.peek();
        self.history.with_mut(|h| {
            if h.last() == Some(&idx) {
                h.pop();
            }
        });
        self.set_intent(PlaybackIntent::Committed { token: from_token });
        Some(from_token)
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

    pub(crate) fn allocate_token(&mut self) -> u64 {
        let token = self.next_token.peek().wrapping_add(1);
        self.next_token.set(token);
        token
    }

    /// The one writer of playback intent — keeps the `current_token` mirror and
    /// the browse spinner in step so no cancel path leaves them stale.
    pub(crate) fn set_intent(&mut self, next: PlaybackIntent) {
        self.browse_loading.set(false);
        self.current_token.set(next.token());
        self.intent.set(next);
    }

    /// Banner + stay on the visible track (never auto-advance): a failed
    /// crossfade reverts to the still-playing outgoing session; a failed
    /// immediate load leaves its already-hydrated track shown, paused. Ignored
    /// if a newer load already superseded `token`.
    pub(crate) fn fail_load(&mut self, token: u64, error: impl std::fmt::Display) {
        let intent = *self.intent.peek();
        if intent.token() != token {
            return;
        }
        self.playback_error
            .set(Some(format!("Couldn't load this track:\n{error}")));
        match intent {
            PlaybackIntent::Loading {
                crossfade: true,
                from_token,
                ..
            } => {
                self.set_intent(PlaybackIntent::Committed { token: from_token });
            }
            _ => {
                self.set_intent(PlaybackIntent::Stopped);
                self.is_playing.set(false);
            }
        }
    }

    /// The current Spotify OAuth access token, if Spotify is the active server.
    pub(crate) fn spotify_access(&self) -> Option<String> {
        let cfg = self.config.peek();
        let server = cfg.server.as_ref()?;
        if server.service != MusicService::Spotify {
            return None;
        }
        let packed = server.access_token.clone()?;
        let access = ::server::spotify::auth::unpack_token(&packed).0;
        (!access.is_empty()).then_some(access)
    }

    /// Start the browser playback host if it isn't running yet (first Spotify play).
    fn ensure_spotify_host(&mut self) {
        if self.spotify_host.peek().is_some() || *self.spotify_host_starting.peek() {
            return;
        }
        let Some(access) = self.spotify_access() else {
            return;
        };
        let browser = self.config.peek().spotify_browser.clone();
        self.spotify_host_starting.set(true);
        let mut host_sig = self.spotify_host;
        let mut starting = self.spotify_host_starting;
        let mut error = self.playback_error;
        let mut external = self.external_active;
        let mut playing = self.is_playing;
        let mut pending = self.spotify_pending_uri;
        spawn(async move {
            match ::server::spotify::host::SpotifyHost::start(access, browser).await {
                Ok(host) => host_sig.set(Some(host)),
                Err(e) => {
                    tracing::warn!(error = %e, "spotify host failed to start");
                    error.set(Some(e));
                    if *external.peek() {
                        external.set(false);
                        playing.set(false);
                        pending.set(None);
                    }
                }
            }
            starting.set(false);
        });
    }

    /// The Spotify access token for UI data fetches (Connect device list).
    pub fn spotify_access_token(&self) -> Option<String> {
        self.spotify_access()
    }

    /// Route Spotify playback to a Connect device (`None` = the in-app SDK
    /// device), transferring the live session over when one is active.
    pub fn spotify_select_device(&mut self, device_id: Option<String>) {
        self.spotify_device_override.set(device_id.clone());
        if !*self.external_active.peek() {
            return;
        }
        let Some(access) = self.spotify_access() else {
            return;
        };
        let Some(target) = device_id.or_else(|| self.spotify_device.peek().clone()) else {
            return;
        };
        let play = *self.is_playing.peek();
        spawn(async move {
            if let Err(e) = ::server::spotify::api::transfer_playback(&access, &target, play).await
            {
                tracing::warn!(error = %e, "spotify device transfer failed");
            }
        });
    }

    pub(crate) fn spotify_transport_pause(&mut self) {
        if self.spotify_device_override.peek().is_some() {
            if let Some(access) = self.spotify_access() {
                spawn(async move {
                    let _ = ::server::spotify::api::player_pause(&access).await;
                });
            }
        } else if let Some(host) = self.spotify_host.peek().clone() {
            host.pause();
        }
    }

    pub(crate) fn spotify_transport_resume(&mut self) {
        if self.spotify_device_override.peek().is_some() {
            if let Some(access) = self.spotify_access() {
                spawn(async move {
                    let _ = ::server::spotify::api::player_resume(&access).await;
                });
            }
        } else if let Some(host) = self.spotify_host.peek().clone() {
            host.resume();
        }
    }

    /// Begin playing a Spotify track id. A chosen Connect device gets the URI
    /// directly; otherwise ensure the host and start it on the SDK device — or
    /// stash it as pending until the device is ready AND the tab has confirmed
    /// playback is allowed. The pump fires the pending URI on those events.
    fn spotify_play(&mut self, item_id: &str) {
        let uri = format!("spotify:track:{item_id}");
        if let Some(device) = self.spotify_device_override.peek().clone() {
            if let Some(access) = self.spotify_access() {
                self.spotify_pending_uri.set(None);
                let mut error = self.playback_error;
                spawn(async move {
                    if let Err(e) =
                        ::server::spotify::api::start_playback(&access, &device, &[uri]).await
                    {
                        tracing::warn!(error = %e, "spotify start_playback failed");
                        error.set(Some(e));
                    }
                });
            }
            return;
        }
        self.ensure_spotify_host();
        if !*self.spotify_activated.peek() {
            self.spotify_pending_uri.set(Some(uri));
            return;
        }
        match (self.spotify_access(), self.spotify_device.peek().clone()) {
            (Some(access), Some(device)) => {
                self.spotify_pending_uri.set(None);
                let uri = uri.clone();
                let mut error = self.playback_error;
                spawn(async move {
                    if let Err(e) =
                        ::server::spotify::api::start_playback(&access, &device, &[uri]).await
                    {
                        tracing::warn!(error = %e, "spotify start_playback failed");
                        error.set(Some(e));
                    }
                });
            }
            _ => self.spotify_pending_uri.set(Some(uri)),
        }
    }

    /// Stop external (Spotify) playback and hand control back to the engine.
    /// Also drops any deferred play so a later `Ready`/`Activated` event can't
    /// start an obsolete track over whatever plays next.
    pub(crate) fn stop_external_playback(&mut self) {
        if !*self.external_active.peek() {
            return;
        }
        self.spotify_pending_uri.set(None);
        self.spotify_transport_pause();
        self.external_active.set(false);
    }

    /// Seek the current track. All progress-bar / lyric scrubbers route here.
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
        // ── Phase 1: classify — no mutation (bar stale-cache eviction), so
        // every early bail below leaves no half-set loading state behind. ──
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

        let is_spotify_item = track.id.service() == Some(MusicService::Spotify);
        if !is_spotify_item {
            self.stop_external_playback();
        } else if self.spotify_access().is_some() {
            self.playback_error.set(None);
            self.cancel_radio_task();
            self.cancel_load_task();
            self.clear_pending_crossfade_ui();
            self.player.peek().stop_for_transition();
            let token = self.allocate_token();
            self.set_intent(PlaybackIntent::Committed { token });
            self.external_active.set(true);
            self.spotify_progress_anchor
                .set(Some((0, std::time::Instant::now())));
            self.hydrate_current_track_metadata(idx, 0);
            self.is_playing.set(true);
            self.spotify_play(&id);
            scrobble_scheduler::schedule(
                track.clone(),
                Some(id.clone()),
                self.config,
                self.current_token,
                token,
                self.is_playing,
                Some(self.active_source),
                ScrobbleOptions::REMOTE_NATIVE,
                self.db.peek().clone(),
            );
            return;
        }

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
            // Every server source resolves its stream async in the load task, so
            // the ref is a pending marker; only the cover is built now, through
            // the cover seam. No creds no longer bails silently — resolve_stream
            // surfaces a real error instead.
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

        // ── Phase 2: commit — mutates state, so only past every bail above ──
        // (Cancelling the prior load is just a resource optimization; the token
        // is what guards correctness.)
        self.playback_error.set(None);
        self.cancel_radio_task();
        self.cancel_load_task();
        if !use_crossfade {
            self.clear_pending_crossfade_ui();
        }
        let from_token = self.intent.peek().token();
        let token = self.allocate_token();
        self.set_intent(PlaybackIntent::Loading {
            token,
            idx,
            crossfade: use_crossfade,
            from_token,
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
        let station_id = id.clone();
        let cached_item_id = id;

        let task = spawn(
            async move {
                let factory: SourceFactory = if let Some(path) = local_path {
                    Box::new(move || decoder::open_file(&path).map_err(|e| e.to_string()))
                } else if let Some(path) = offline_path {
                    // Cached server file: on open failure fall back to the live
                    // stream instead of failing. The resolve blocks on this
                    // task's runtime handle — legal on the decode worker.
                    let source = ctrl.active_source.peek().clone();
                    let rt_handle = tokio::runtime::Handle::current();
                    Box::new(move || match decoder::open_file(&path) {
                        Ok(parts) => Ok(parts),
                        Err(e) => {
                            tracing::warn!(error = %e, "cached file failed to open; falling back to the server stream");
                            let info = rt_handle
                                .block_on(source.resolve_stream(&cached_item_id))
                                .map_err(|e| e.to_string())?;
                            network_factory(
                                info.url,
                                info.format,
                                info.user_agent,
                                false,
                                rt_handle.clone(),
                            )()
                        }
                    })
                } else {
                    let (stream_ref, _) = remote_ref.expect("classified as remote above");
                    let (stream_url, yt_format, yt_user_agent) =
                        match ResolvedStreamRef::parse(&stream_ref) {
                            ResolvedStreamRef::Pending(item_id) => {
                                // The one genuinely per-source op: resolve the
                                // playable stream through the active source
                                // (a URL for Jellyfin/Subsonic, a deciphered
                                // stream for YT).
                                let source = ctrl.active_source.peek().clone();
                                match source.resolve_stream(item_id).await {
                                    Ok(info) => {
                                        ctrl.stamp_probed_stream_info(
                                            phys_idx,
                                            idx,
                                            info.duration_secs,
                                            info.bitrate,
                                        );
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

                    // The factory runs on the decode worker (no runtime), so
                    // hand StreamBuffer's download a handle from this task.
                    let rt_handle = tokio::runtime::Handle::current();
                    network_factory(stream_url, yt_format, yt_user_agent, is_radio_item, rt_handle)
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
                        ctrl.set_intent(PlaybackIntent::Committed { token });
                        if clear_pending_resume_on_success {
                            ctrl.pending_resume.set(None);
                        }
                        if use_crossfade {
                            if outcome.crossfaded {
                                // Defer the UI until TrackSwitched confirms the fade.
                                ctrl.schedule_pending_crossfade_ui(idx, token, from_token);
                            } else {
                                // Crossfade fell back to an immediate switch —
                                // commit now; no fade midpoint is coming.
                                ctrl.hydrate_current_track_metadata(idx, 0);
                            }
                        }

                        if is_radio_item {
                            ctrl.start_radio_metadata(station_id, stream_id);
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

                            if is_server_item && !cover_url.is_empty() {
                                ctrl.spawn_server_artwork_fetch(
                                    cover_url.clone(),
                                    track.clone(),
                                    token,
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

/// Factory for a resolved network stream (radio, YT range/sequential,
/// SoundCloud HLS, or a plain buffered stream). Returns a `SourceFactory` so the
/// symphonia types stay inferred inside the closure — hooks can't name them.
fn network_factory(
    stream_url: String,
    yt_format: Option<(::server::ytmusic::player::AudioFormat, bool)>,
    yt_user_agent: Option<String>,
    is_radio: bool,
    rt_handle: tokio::runtime::Handle,
) -> SourceFactory {
    Box::new(move || {
        let build = || -> std::io::Result<_> {
            if is_radio {
                let stream = utils::stream_buffer::StreamBuffer::with_user_agent(
                    stream_url,
                    true,
                    yt_user_agent,
                    rt_handle,
                );
                Ok(decoder::from_stream_with_hint(stream, "ogg"))
            } else if let Some((fmt, range_safe)) = yt_format {
                if range_safe {
                    // YT: HTTP Range-backed source. Symphonia can seek freely
                    // (Matroska Cues at the end, scrub anywhere) and startup
                    // probes only fetch the ~512 KiB they need.
                    let range =
                        utils::range_source::RangeStreamSource::new(stream_url, yt_user_agent)?;
                    let len = Some(range.total_size());
                    let (source, mut hint) = decoder::from_stream_with_len(range, len);
                    hint.with_extension(fmt.extension());
                    Ok((source, hint))
                } else {
                    // No-pot fallback: googlevideo 403s deep ranges, and the
                    // probe reads the webm tail — stream sequentially instead of
                    // failing outright (issue #386). No scrubbing.
                    let stream = utils::stream_buffer::StreamBuffer::with_user_agent(
                        stream_url,
                        false,
                        yt_user_agent,
                        rt_handle,
                    );
                    stream.wait_for_total_size();
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
    let spotify_host = use_signal(|| None::<::server::spotify::host::SpotifyHost>);
    let spotify_device = use_signal(|| None::<String>);
    let spotify_pending_uri = use_signal(|| None::<String>);
    let spotify_activated = use_signal(|| false);
    let spotify_device_override = use_signal(|| None::<String>);
    let spotify_progress_anchor = use_signal(|| None::<(u64, std::time::Instant)>);
    let spotify_host_starting = use_signal(|| false);
    let external_active = use_signal(|| false);
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
        spotify_host,
        spotify_device,
        spotify_pending_uri,
        spotify_activated,
        spotify_device_override,
        spotify_progress_anchor,
        spotify_host_starting,
        external_active,
    }
}
