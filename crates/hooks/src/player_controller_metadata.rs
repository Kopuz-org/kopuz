//! Track metadata hydration and playback-transition state.

use dioxus::logger::tracing::Instrument;
use dioxus::prelude::*;
use player::player::NowPlayingMeta;
use reader::Track;

use crate::use_player_controller::{
    PendingCrossfadeUiState, PendingResumeState, PlaybackIntent, PlayerController,
};

impl PlayerController {
    pub(super) fn stamp_probed_stream_info(
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
    ///
    /// `icy_rx` carries `StreamTitle` updates from the audio connection. Only
    /// wired up for stations without a live (REST/WebSocket) provider, so the
    /// two sources never fight.
    pub(super) fn start_radio_metadata(
        &mut self,
        station_id: String,
        stream_id: String,
        icy_rx: Option<tokio::sync::watch::Receiver<utils::icy::IcyMeta>>,
    ) {
        let Some(provider) = self.station_registry.read().create_provider(&station_id) else {
            tracing::warn!("[radio] no metadata provider for station: {station_id}");
            return;
        };
        // Station artwork / name fallbacks for song updates without their own.
        let station_cover: Option<String> =
            self.station_registry
                .read()
                .get(&station_id)
                .and_then(|m| match &m.metadata {
                    Some(radio::manifest::MetadataSourceDef::Static(s)) => s.cover_url.clone(),
                    _ => None,
                });
        let station_name: Option<String> = self
            .station_registry
            .read()
            .get(&station_id)
            .map(|m| m.name.clone())
            .filter(|n| !n.trim().is_empty());
        let mut current_song_title = self.current_song_title;
        let mut current_song_artist = self.current_song_artist;
        let mut current_song_album = self.current_song_album;
        let mut current_song_cover_url = self.current_song_cover_url;
        let task = spawn(async move {
            use radio::provider::RadioMetadataProvider;
            let mut rx = provider.start(&stream_id);
            // Signals are Copy: each loop gets its own mutable handle.
            let mut icy_song_title = current_song_title;
            let mut icy_song_artist = current_song_artist;
            let mut icy_song_cover = current_song_cover_url;
            let provider_loop = async {
                while let Some(meta) = rx.recv().await {
                    current_song_title.set(meta.title.clone());
                    current_song_artist.set(meta.artist.clone());
                    current_song_album.set(meta.station.clone());
                    current_song_cover_url.set(meta.cover_url.unwrap_or_default());
                }
            };
            let icy_loop = async {
                let Some(mut icy_rx) = icy_rx else { return };
                while icy_rx.changed().await.is_ok() {
                    let meta = icy_rx.borrow_and_update().clone();
                    if meta.title.trim().is_empty() {
                        continue;
                    }
                    let (artist, title) = utils::icy::split_artist_title(&meta.title);
                    icy_song_title.set(title);
                    // No artist in title: show station.
                    if let Some(artist) = artist.or_else(|| station_name.clone()) {
                        icy_song_artist.set(artist);
                    }
                    let cover = meta
                        .cover_url
                        .or_else(|| station_cover.clone())
                        .unwrap_or_default();
                    icy_song_cover.set(cover);
                }
            };
            tokio::join!(provider_loop, icy_loop);
        });
        self.radio_task.set(Some(task));
    }

    /// Download a server track's cover to a temp file and hand the OS media
    /// controls its local path (they need a path, not a URL). No-ops if `token`
    /// is superseded before the download finishes.
    pub(super) fn spawn_server_artwork_fetch(&self, cover_url: String, track: Track, token: u64) {
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

    pub(super) fn cover_url_for_track(&self, track: &Track) -> String {
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
        self.buffered_ranges.set(Vec::new());
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

    /// Adopt a Spotify Connect track started elsewhere. Upsert it into the
    /// queue and select its logical position so the normal queue-state save can
    /// restore it after restart instead of falling back to the last track
    /// clicked in kopuz.
    ///
    /// Under shuffle, Spotify reports every track change — including the ones
    /// kopuz itself commanded — so reshuffling on all of them tears up the run
    /// mid-listen. A track the queue already held keeps its permutation
    /// position; only a track the queue has never seen is a genuine pick from
    /// another client, and that one starts a new run, because leaving a
    /// just-appended index in the old permutation puts it at the end and makes
    /// Next stop immediately.
    pub(crate) fn hydrate_external_track_metadata(&mut self, track: Track, progress_secs: u64) {
        let queued_idx = self
            .queue
            .peek()
            .iter()
            .position(|queued| queued.id == track.id);
        let physical_idx = match queued_idx {
            Some(idx) => {
                self.queue.write()[idx] = track;
                idx
            }
            None => {
                let idx = self.queue.peek().len();
                self.queue.write().push(track);
                idx
            }
        };

        let logical_idx = if *self.shuffle.peek() {
            match queued_idx.and_then(|_| self.shuffle_position_of(physical_idx)) {
                Some(position) => position,
                None => {
                    self.current_queue_index.set(physical_idx);
                    self.rebuild_shuffle_order();
                    0
                }
            }
        } else {
            physical_idx
        };
        self.hydrate_current_track_metadata(logical_idx, progress_secs);
    }

    /// Replace the provisional one-track external queue with the complete
    /// Spotify playlist/album once its context finishes loading.
    pub(crate) fn hydrate_external_context(
        &mut self,
        tracks: Vec<Track>,
        current_track_id: &str,
        progress_secs: u64,
    ) {
        let Some(physical_idx) = tracks
            .iter()
            .position(|track| track.id.key() == current_track_id)
        else {
            return;
        };

        self.queue.set(tracks);
        self.history.write().clear();
        self.current_queue_index.set(physical_idx);
        let logical_idx = if *self.shuffle.peek() {
            self.rebuild_shuffle_order();
            0
        } else {
            physical_idx
        };
        self.hydrate_current_track_metadata(logical_idx, progress_secs);
    }

    pub(super) fn pending_resume_seek(&self, track: &Track) -> (Option<u64>, bool) {
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
        } else {
            let from_token = resolving?;
            self.cancel_load_task();
            from_token
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
        self.buffered_ranges.set(Vec::new());
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
}
