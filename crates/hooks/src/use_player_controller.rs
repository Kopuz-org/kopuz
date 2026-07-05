use config::AppConfig;
use config::MusicService;
use dioxus::logger::tracing::Instrument;
use dioxus::{logger::tracing, prelude::*};
use player::player::{NowPlayingMeta, Player};
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
    pub is_loading: Signal<bool>,
    pub skip_in_progress: Signal<bool>,
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
    pub play_generation: Signal<usize>,
    pub(crate) pending_resume: Signal<Option<PendingResumeState>>,
    pub pending_crossfade_ui: Signal<Option<PendingCrossfadeUiState>>,
    pub radio_task: Signal<Option<dioxus_core::Task>>,
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

#[derive(Clone, Debug)]
pub struct PendingCrossfadeUiState {
    pub next_idx: usize,
    pub switch_after_secs: u64,
    pub outgoing_duration_secs: u64,
    pub outgoing_progress_secs: u64,
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

    pub(crate) fn clear_pending_resume(&mut self) {
        self.pending_resume.set(None);
    }

    pub(crate) fn clear_pending_crossfade_ui(&mut self) {
        self.pending_crossfade_ui.set(None);
    }

    fn build_pending_crossfade_ui(
        next_idx: usize,
        outgoing_duration_secs: u64,
        outgoing_progress_secs: u64,
    ) -> PendingCrossfadeUiState {
        PendingCrossfadeUiState {
            next_idx,
            switch_after_secs: outgoing_duration_secs.saturating_sub(outgoing_progress_secs),
            outgoing_duration_secs,
            outgoing_progress_secs,
        }
    }

    pub(crate) fn schedule_pending_crossfade_ui(
        &mut self,
        next_idx: usize,
        outgoing_duration_secs: u64,
        outgoing_progress_secs: u64,
    ) {
        self.pending_crossfade_ui
            .set(Some(Self::build_pending_crossfade_ui(
                next_idx,
                outgoing_duration_secs,
                outgoing_progress_secs,
            )));
    }

    pub fn commit_pending_crossfade_ui(&mut self, next_progress_secs: u64) -> bool {
        let Some(pending) = self.pending_crossfade_ui.read().clone() else {
            return false;
        };

        self.pending_crossfade_ui.set(None);
        self.hydrate_current_track_metadata(pending.next_idx, next_progress_secs);
        true
    }

    pub(crate) fn set_pending_resume_for_track(&mut self, track: &Track, progress_secs: u64) {
        self.pending_resume.set(Some(PendingResumeState {
            track_path: Self::track_key(track),
            progress_secs: progress_secs.min(track.duration),
        }));
    }

    fn apply_restore_seek(&mut self, seek_secs: u64) {
        self.player.write().seek(Duration::from_secs(seek_secs));
        self.current_song_progress.set(seek_secs);
    }

    /// Seek the current track. All progress-bar / lyric scrubbers route here.
    ///
    /// If the track has already ended, its decode thread has exited and a plain
    /// `Player::seek` would be dropped (the playhead snaps back to the end). Re-open
    /// the current track with a pending-resume at this position so it plays from
    /// where the playhead was dropped instead.
    pub fn seek(&mut self, time: Duration) {
        if self.player.read().track_ended() {
            let idx = *self.current_queue_index.peek();
            if let Some(track) = self.get_track_at(idx) {
                let secs = time.as_secs().min(track.duration);
                self.set_pending_resume_for_track(&track, secs);
                self.current_song_progress.set(secs);
                self.play_track_no_history_with_transition(idx, false);
                return;
            }
        }
        self.player.write().seek(time);
        self.current_song_progress.set(time.as_secs());
    }

    pub fn displayed_progress_secs_f64(&self) -> f64 {
        let pos = self.player.peek().get_position().as_secs_f64();

        if let Some(pending) = self.pending_crossfade_ui.read().clone()
            && pos <= pending.switch_after_secs as f64
        {
            return (pending.outgoing_progress_secs as f64 + pos)
                .min(pending.outgoing_duration_secs as f64);
        }

        pos
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
        let idx = *self.current_queue_index.peek();
        let queue_len = self.queue.peek().len();

        if queue_len == 0 {
            return false;
        }

        let loop_mode = *self.loop_mode.peek();
        let shuffle = *self.shuffle.peek();

        match loop_mode {
            LoopMode::Track => true,
            _ => {
                if shuffle && queue_len > 1 {
                    !self.shuffle_order.peek().is_empty() || loop_mode == LoopMode::Queue
                } else if (shuffle && queue_len == 1) || idx + 1 < queue_len {
                    true
                } else {
                    loop_mode == LoopMode::Queue
                }
            }
        }
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
        self.play_generation.with_mut(|g| *g += 1);
        let current_gen = *self.play_generation.peek();
        // Starting a new track clears the previous track's error banner —
        // otherwise a 403 from a skipped YT track lingers on screen.
        self.playback_error.set(None);
        self.cancel_radio_task();

        if let Some(track) = self.get_track_at(idx) {
            let path_str = track.id.uid().to_string();
            let (restore_seek_secs, clear_pending_resume_on_success) =
                self.pending_resume_seek(&track);
            let use_crossfade = allow_crossfade
                && self.should_crossfade()
                && restore_seek_secs.is_none_or(|secs| secs == 0);
            let outgoing_duration_secs = *self.current_song_duration.peek();
            let outgoing_progress_secs =
                (*self.current_song_progress.peek()).min(outgoing_duration_secs);
            if !use_crossfade {
                self.clear_pending_crossfade_ui();
            }
            let crossfade_duration =
                Duration::from_secs(self.config.peek().crossfade_seconds as u64);
            let item_ref = PlaybackItemRef::parse(&path_str);
            let is_radio_item = item_ref.is_radio();
            let is_server_item = item_ref.is_server();

            if is_server_item || is_radio_item {
                let id = item_ref.primary_id().unwrap_or_default().to_string();
                let stream_id = item_ref.stream_id().unwrap_or_default().to_string();

                // Check offline cache first
                {
                    let offline_path = if is_server_item {
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
                    if let Some(local_path) = offline_path
                        && let Ok((source, hint)) = decoder::open_file(&local_path)
                    {
                        if !use_crossfade {
                            self.current_queue_index.set(idx);
                            self.player.write().stop_for_transition();
                            self.is_playing.set(false);
                        }

                        let cover_url = self.cover_url_for_track(&track);
                        if !use_crossfade {
                            self.hydrate_current_track_metadata(
                                idx,
                                restore_seek_secs.unwrap_or(0),
                            );
                            self.current_song_cover_url.set(cover_url.clone());
                        }
                        self.is_loading.set(true);

                        let mut player = self.player;
                        let mut is_playing = self.is_playing;
                        let mut is_loading = self.is_loading;
                        let mut skip_in_progress = self.skip_in_progress;
                        let play_generation = self.play_generation;
                        let volume = self.volume;
                        let mut current_song_progress = self.current_song_progress;
                        let mut pending_crossfade_ui = self.pending_crossfade_ui;
                        let mut pending_resume = self.pending_resume;
                        let cfg_signal = self.config;

                        spawn(async move {
                            if *play_generation.read() == current_gen {
                                let meta = NowPlayingMeta {
                                    title: track.title.clone(),
                                    artist: track.artist.clone(),
                                    album: track.album.clone(),
                                    duration: std::time::Duration::from_secs(track.duration),
                                    artwork: Some(cover_url),
                                };
                                let result = if use_crossfade {
                                    player.write().crossfade_to(
                                        source,
                                        meta,
                                        hint,
                                        crossfade_duration,
                                    )
                                } else {
                                    player.write().play(source, meta, hint)
                                };
                                if let Err(e) = result {
                                    tracing::error!(error = %e, "offline playback failed");
                                    is_loading.set(false);
                                    skip_in_progress.set(false);
                                    return;
                                }
                                player.write().set_volume(*volume.peek());
                                if let Some(seek_secs) = restore_seek_secs
                                    && seek_secs > 0
                                {
                                    player.write().seek(Duration::from_secs(seek_secs));
                                }
                                if use_crossfade {
                                    pending_crossfade_ui.set(Some(
                                        PlayerController::build_pending_crossfade_ui(
                                            idx,
                                            outgoing_duration_secs,
                                            outgoing_progress_secs,
                                        ),
                                    ));
                                } else {
                                    current_song_progress.set(restore_seek_secs.unwrap_or(0));
                                }
                                is_playing.set(true);
                                is_loading.set(false);
                                skip_in_progress.set(false);

                                if clear_pending_resume_on_success {
                                    pending_resume.set(None);
                                }
                                let _ = cfg_signal;
                            }
                        });
                        return;
                    }
                }

                if let Some((stream_url, cover_url)) = {
                    if is_radio_item {
                        self.station_registry
                            .read()
                            .get(&id)
                            .and_then(|s| s.streams.iter().find(|str| str.id == stream_id))
                            .map(|s| s.url.clone())
                            .map(|stream_url| (stream_url, String::new()))
                    } else {
                        let conf = self.config.read();
                        conf.server.as_ref().map(|server| match server.service {
                            MusicService::Jellyfin => {
                                // Stream resolves in the spawn below via the source
                                // backend; only the cover is built synchronously so
                                // artwork shows immediately on click.
                                let cover_url = utils::jellyfin_image::resolve_track_cover(
                                    track.cover.as_deref(),
                                    &track.id.key(),
                                    &track.album_id,
                                    &server.url,
                                    server.access_token.as_deref(),
                                    800,
                                    90,
                                )
                                .unwrap_or_default();

                                (ResolvedStreamRef::pending_marker(&id), cover_url)
                            }
                            MusicService::Subsonic | MusicService::Custom => {
                                // Stream resolves in the spawn below; build only the
                                // cover here. No creds → empty stream so the sync bail
                                // below stops loading silently, as before.
                                if let (Some(password), Some(username)) =
                                    (&server.access_token, &server.user_id)
                                {
                                    let subsonic_path = match track.cover.as_deref() {
                                        Some(c) => format!("{}:{}", track.id.uid(), c),
                                        None => track.id.uid(),
                                    };
                                    let cover_url =
                                        utils::subsonic_image::subsonic_image_url_from_path(
                                            &subsonic_path,
                                            &server.url,
                                            server.access_token.as_deref(),
                                            800,
                                            90,
                                        )
                                        .or_else(|| {
                                            ::server::subsonic::cover_art_url(
                                                &server.url,
                                                username,
                                                password,
                                                &id,
                                                Some(800),
                                            )
                                            .ok()
                                        })
                                        .unwrap_or_default();
                                    (ResolvedStreamRef::pending_marker(&id), cover_url)
                                } else {
                                    (String::new(), String::new())
                                }
                            }
                            // YT can't resolve its stream URL synchronously (the
                            // /player call is async + multi-client fallback). Tag
                            // the URL with a sentinel that the async spawn below
                            // recognises and replaces with the real URL. The
                            // cover URL, however, is already encoded in the
                            // track's path (`ytmusic:VID:urlhex_HEX`) so we
                            // can produce it synchronously here so the
                            // bottombar shows artwork immediately on click.
                            MusicService::YtMusic => {
                                let cover_url = utils::jellyfin_image::resolve_track_cover(
                                    track.cover.as_deref(),
                                    &track.id.key(),
                                    &track.album_id,
                                    "",
                                    None,
                                    800,
                                    90,
                                )
                                .unwrap_or_default();
                                (ResolvedStreamRef::pending_marker(&id), cover_url)
                            }
                            // SoundCloud also resolves its stream async (progressive
                            // MP3 / HLS) via the source; the cover is the plain
                            // artwork URL already in `track.cover`.
                            MusicService::SoundCloud => (
                                ResolvedStreamRef::pending_marker(&id),
                                track.cover.clone().unwrap_or_default(),
                            ),
                        })
                    }
                } {
                    if stream_url.is_empty() {
                        self.is_loading.set(false);
                        self.skip_in_progress.set(false);
                        return;
                    }

                    if !use_crossfade {
                        self.player.write().stop_for_transition();
                        self.is_playing.set(false);
                    }

                    let mut player = self.player;
                    let mut is_playing = self.is_playing;
                    let mut is_loading = self.is_loading;
                    let mut skip_in_progress = self.skip_in_progress;
                    let mut playback_error = self.playback_error;
                    let play_generation = self.play_generation;
                    let volume = self.volume;
                    let mut current_song_progress = self.current_song_progress;
                    let mut pending_crossfade_ui = self.pending_crossfade_ui;
                    let mut pending_resume = self.pending_resume;
                    let cfg_signal = self.config;
                    let active_source = self.active_source;
                    let mut radio_task = self.radio_task;
                    let mut current_song_title = self.current_song_title;
                    let mut current_song_artist = self.current_song_artist;
                    let mut current_song_album = self.current_song_album;
                    let mut current_song_cover_url = self.current_song_cover_url;
                    let station_registry = self.station_registry;
                    let mut queue_for_yt = self.queue;
                    let current_queue_index_for_yt = self.current_queue_index;
                    let mut current_song_duration_for_yt = self.current_song_duration;
                    let mut current_song_bitrate_for_yt = self.current_song_bitrate;
                    // Physical queue slot for the metadata write-backs below — the
                    // resolve closure's `get_mut` indexes the raw Vec, so the
                    // display/shuffle `idx` would hit the wrong track in shuffle.
                    let phys_idx = self.get_queue_index(idx);

                    if !use_crossfade {
                        self.hydrate_current_track_metadata(idx, restore_seek_secs.unwrap_or(0));
                        self.current_song_cover_url.set(cover_url.clone());
                    }

                    self.is_loading.set(true);

                    let is_radio = PlaybackItemRef::parse(&track.id.uid()).is_radio();

                    spawn(async move {
                        let (stream_url, yt_format, yt_user_agent) =
                            match ResolvedStreamRef::parse(&stream_url) {
                                ResolvedStreamRef::Pending(item_id) => {
                                    // The one genuinely per-source op: resolve the playable
                                    // stream through the active source's backend (a URL for
                                    // Jellyfin/Subsonic, a deciphered stream for YT).
                                    let source = active_source.peek().clone();
                                    let resolved = source.resolve_stream(item_id).await;
                                    match resolved {
                                        Ok(info) => {
                                            // Stale-resolve guard: don't stamp duration / mutate
                                            // queue if the user clicked another track while we
                                            // were awaiting the resolve. (YT carries probed
                                            // duration/bitrate; other sources leave them None.)
                                            if *play_generation.read() == current_gen
                                                && let Some(secs) = info.duration_secs
                                                && secs > 0
                                            {
                                                if let Some(p) = phys_idx
                                                    && let Some(t) = queue_for_yt.write().get_mut(p)
                                                {
                                                    t.duration = secs;
                                                }
                                                if *current_queue_index_for_yt.peek() == idx {
                                                    current_song_duration_for_yt.set(secs);
                                                }
                                            }
                                            // Surface the resolved bitrate (kbps) for the debug
                                            // readout; write it back onto the queue Track too (YT
                                            // tracks carry no bitrate metadata) so a re-hydrate
                                            // doesn't reset it.
                                            if *play_generation.read() == current_gen
                                                && let Some(bps) = info.bitrate
                                            {
                                                let kbps = (bps / 1000) as u16;
                                                if let Some(p) = phys_idx
                                                    && let Some(t) = queue_for_yt.write().get_mut(p)
                                                {
                                                    t.bitrate = kbps;
                                                }
                                                if *current_queue_index_for_yt.peek() == idx {
                                                    current_song_bitrate_for_yt.set(kbps);
                                                }
                                            }
                                            (info.url, info.format, info.user_agent)
                                        }
                                        Err(e) => {
                                            // Same guard: a stale error must NOT post a banner
                                            // for or clear is_loading on the user's new track.
                                            if *play_generation.read() != current_gen {
                                                return;
                                            }
                                            tracing::error!(error = %e, "stream URL resolve failed");
                                            playback_error.set(Some(format!(
                                                "Couldn't load this track:\n{e}"
                                            )));
                                            is_loading.set(false);
                                            skip_in_progress.set(false);
                                            return;
                                        }
                                    }
                                }
                                ResolvedStreamRef::SoundCloudHls(_)
                                | ResolvedStreamRef::Direct(_) => (stream_url, None, None),
                            };
                        let yt_format_for_blocking = yt_format;
                        let stream_url_for_blocking = stream_url.clone();
                        let yt_ua_for_blocking = yt_user_agent.clone();
                        let source_res = tokio::task::spawn_blocking(move || {
                            if is_radio {
                                let stream = utils::stream_buffer::StreamBuffer::with_user_agent(
                                    stream_url_for_blocking,
                                    true,
                                    yt_ua_for_blocking,
                                );
                                let (source, hint) = decoder::from_stream_with_hint(stream, "ogg");
                                Ok::<_, std::io::Error>((source, hint))
                            } else if let Some((fmt, range_safe)) = yt_format_for_blocking {
                                if range_safe {
                                    // YT: HTTP Range-backed source. Symphonia
                                    // can seek freely (Matroska Cues at the
                                    // end, scrub anywhere) and startup probes
                                    // only fetch the ~512 KiB they need.
                                    let range = utils::range_source::RangeStreamSource::new(
                                        stream_url_for_blocking,
                                        yt_ua_for_blocking,
                                    )?;
                                    let len = Some(range.total_size());
                                    let (source, mut hint) =
                                        decoder::from_stream_with_len(range, len);
                                    hint.with_extension(fmt.extension());
                                    Ok::<_, std::io::Error>((source, hint))
                                } else {
                                    // No-pot fallback: googlevideo 403s deep
                                    // ranges, and the probe reads the webm tail
                                    // — stream sequentially instead of failing
                                    // outright (issue #386). No scrubbing.
                                    let stream =
                                        utils::stream_buffer::StreamBuffer::with_user_agent(
                                            stream_url_for_blocking,
                                            false,
                                            yt_ua_for_blocking,
                                        );
                                    stream.wait_for_total_size();
                                    let len = stream.known_total_size();
                                    let (source, mut hint) =
                                        decoder::from_stream_with_len(stream, len);
                                    hint.with_extension(fmt.extension());
                                    Ok::<_, std::io::Error>((source, hint))
                                }
                            } else if let ResolvedStreamRef::SoundCloudHls(hls_url) =
                                ResolvedStreamRef::parse(&stream_url_for_blocking)
                            {
                                // SoundCloud Go+ AAC: assemble the HLS playlist's
                                // fMP4 segments into one in-memory buffer Symphonia
                                // can decode (it has no HLS demuxer).
                                let bytes = utils::hls_source::assemble(
                                    hls_url,
                                    yt_ua_for_blocking.as_deref(),
                                )?;
                                let len = Some(bytes.len() as u64);
                                let cursor = std::io::Cursor::new(bytes);
                                let (source, mut hint) = decoder::from_stream_with_len(cursor, len);
                                hint.with_extension("m4a");
                                Ok::<_, std::io::Error>((source, hint))
                            } else {
                                let stream = utils::stream_buffer::StreamBuffer::with_user_agent(
                                    stream_url_for_blocking,
                                    false,
                                    yt_ua_for_blocking,
                                );
                                stream.wait_for_total_size();
                                let len = stream.known_total_size();
                                let (source, hint) = decoder::from_stream_with_len(stream, len);
                                Ok::<_, std::io::Error>((source, hint))
                            }
                        })
                        .await;

                        if let Err(_) | Ok(Err(_)) = &source_res {
                            if *play_generation.read() == current_gen {
                                let msg = match &source_res {
                                    Ok(Err(e)) => format!("Couldn't load this track:\n{e}"),
                                    _ => "Playback failed unexpectedly".to_string(),
                                };
                                playback_error.set(Some(msg));
                                is_loading.set(false);
                                skip_in_progress.set(false);
                            }
                            return;
                        }

                        if let Ok(Ok((source, hint))) = source_res {
                            if *play_generation.read() == current_gen {
                                let meta = NowPlayingMeta {
                                    title: track.title.clone(),
                                    artist: track.artist.clone(),
                                    album: track.album.clone(),
                                    duration: std::time::Duration::from_secs(track.duration),
                                    artwork: Some(cover_url.clone()),
                                };

                                let result = if use_crossfade {
                                    player.write().crossfade_to(
                                        source,
                                        meta,
                                        hint,
                                        crossfade_duration,
                                    )
                                } else {
                                    player.write().play(source, meta, hint)
                                };
                                if let Err(e) = result {
                                    tracing::error!(error = %e, "playback failed");
                                    is_loading.set(false);
                                    skip_in_progress.set(false);
                                    return;
                                }
                                player.write().set_volume(*volume.peek());
                                if let Some(seek_secs) = restore_seek_secs
                                    && seek_secs > 0
                                {
                                    player.write().seek(Duration::from_secs(seek_secs));
                                    current_song_progress.set(seek_secs);
                                }
                                if use_crossfade {
                                    pending_crossfade_ui.set(Some(
                                        PlayerController::build_pending_crossfade_ui(
                                            idx,
                                            outgoing_duration_secs,
                                            outgoing_progress_secs,
                                        ),
                                    ));
                                } else {
                                    current_song_progress.set(restore_seek_secs.unwrap_or(0));
                                }
                                if clear_pending_resume_on_success {
                                    pending_resume.set(None);
                                }
                                is_loading.set(false);
                                is_playing.set(true);
                                skip_in_progress.set(false);

                                let item_uid = track.id.uid();
                                let item_ref = PlaybackItemRef::parse(&item_uid);
                                let is_radio_item = item_ref.is_radio();
                                let station_id =
                                    item_ref.primary_id().unwrap_or_default().to_string();
                                let stream_id =
                                    item_ref.stream_id().unwrap_or_default().to_string();

                                if let Some(task) = radio_task.take() {
                                    task.cancel();
                                }

                                if is_radio_item {
                                    if let Some(provider) =
                                        station_registry.read().create_provider(&station_id)
                                    {
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

                                        radio_task.set(Some(task));
                                    } else {
                                        tracing::warn!(
                                            "[radio] No metadata provider for station: {}",
                                            station_id
                                        );
                                    }
                                }
                                // Don't scrobble if the track is a radio item
                                if !is_radio_item {
                                    scrobble_scheduler::schedule(
                                        track.clone(),
                                        Some(id.clone()),
                                        cfg_signal,
                                        play_generation,
                                        current_gen,
                                        is_playing,
                                        Some(active_source),
                                        ScrobbleOptions::REMOTE_NATIVE,
                                    );

                                    let cover_url = cover_url.clone();
                                    let track = track.clone();
                                    let mut player = player;
                                    let play_generation = play_generation;

                                    spawn(async move {
                                        if let Ok(response) = reqwest::get(&cover_url).await
                                            && let Ok(bytes) = response.bytes().await
                                        {
                                            let temp_dir = std::env::temp_dir();
                                            let random_id: u64 = rand::random();
                                            let file_path = temp_dir
                                                .join(format!("kopuz_cover_{}.jpg", random_id));

                                            if tokio::fs::write(&file_path, bytes).await.is_ok()
                                                && *play_generation.read() == current_gen
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
                                    }.instrument(tracing::info_span!("player.cover_fetch")));
                                }
                            }
                        } else {
                            is_loading.set(false);
                            skip_in_progress.set(false);
                        }
                    }.instrument(tracing::info_span!("player.resolve_stream", idx)));
                }
            } else {
                if !use_crossfade {
                    self.current_queue_index.set(idx);
                }
                if let Some(track_path) = track.id.local_path()
                    && let Ok((source, hint)) = decoder::open_file(track_path)
                {
                    {
                        // For a local track `track.cover` is its album-art file
                        // path (projected from the album by the DB read layer).
                        let artwork = track.cover.clone();

                        let meta = NowPlayingMeta {
                            title: track.title.clone(),
                            artist: track.artist.clone(),
                            album: track.album.clone(),
                            duration: std::time::Duration::from_secs(track.duration),
                            artwork,
                        };

                        let result = if use_crossfade {
                            self.player
                                .write()
                                .crossfade_to(source, meta, hint, crossfade_duration)
                        } else {
                            self.player.write().play(source, meta, hint)
                        };
                        if let Err(e) = result {
                            tracing::error!(error = %e, "playback failed");
                            self.skip_in_progress.set(false);
                            return;
                        }
                        self.player.write().set_volume(*self.volume.peek());

                        self.skip_in_progress.set(false);

                        if !use_crossfade {
                            self.hydrate_current_track_metadata(
                                idx,
                                restore_seek_secs.unwrap_or(0),
                            );
                        } else {
                            self.schedule_pending_crossfade_ui(
                                idx,
                                outgoing_duration_secs,
                                outgoing_progress_secs,
                            );
                        }

                        self.is_playing.set(true);

                        if let Some(seek_secs) = restore_seek_secs
                            && seek_secs > 0
                        {
                            self.apply_restore_seek(seek_secs);
                        }
                        if clear_pending_resume_on_success {
                            self.clear_pending_resume();
                        }
                        if !is_radio_item {
                            scrobble_scheduler::schedule(
                                track.clone(),
                                None,
                                self.config,
                                self.play_generation,
                                current_gen,
                                self.is_playing,
                                None,
                                ScrobbleOptions::LOCAL,
                            );
                        }
                    }
                }
            }
        }
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
    db_handle: db::Db,
) -> PlayerController {
    let play_generation = use_signal(|| 0);
    let is_loading = use_signal(|| false);
    let skip_in_progress = use_signal(|| false);
    let history = use_signal(Vec::new);
    let shuffle = use_signal(|| false);
    let shuffle_order = use_signal(Vec::<usize>::new);
    let loop_mode = use_signal(|| LoopMode::None);
    let pending_resume = use_signal(|| None::<PendingResumeState>);
    let pending_crossfade_ui = use_signal(|| None::<PendingCrossfadeUiState>);
    let radio_task = use_signal(|| None::<dioxus_core::Task>);
    let station_registry = use_context::<Signal<radio::registry::StationRegistry>>();
    let playback_error = use_signal(|| None::<String>);
    let db = use_signal(move || db_handle);
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();

    PlayerController {
        player,
        is_playing,
        is_loading,
        skip_in_progress,
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
        play_generation,
        pending_resume,
        pending_crossfade_ui,
        radio_task,
        station_registry,
        playback_error,
    }
}
