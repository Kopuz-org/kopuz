//! The player controller, as a mirror over the embedded daemon session.
//!
//! The signal surface and method names the UI consumes are unchanged from the
//! old in-hooks state machine, but every field is now a projection of the
//! daemon's `PlayerState` stream and every transport method forwards a
//! session command. The one exception is Spotify external playback, whose
//! browser host still lives frontend-side: while `external_active` is set the
//! local queue signals are authoritative, and handing control back to the
//! engine pushes them into the daemon.

use std::{sync::Arc, time::Duration};

use config::AppConfig;
use dioxus::prelude::*;
use reader::Track;

#[path = "player_controller_spotify.rs"]
mod spotify;

pub use api::LoopMode;

#[derive(Clone, Copy)]
pub struct PlayerController {
    pub(crate) api: Signal<Arc<dyn api::KopuzApi>>,
    pub is_playing: Signal<bool>,
    pub is_loading: Memo<bool>,
    pub(crate) loading: Signal<bool>,
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
    pub buffered_ranges: Signal<Vec<BufferedRange>>,
    pub current_song_cover_url: Signal<String>,
    pub current_track_snapshot: Signal<Option<Track>>,
    pub volume: Signal<f32>,
    pub config: Signal<AppConfig>,
    pub playback_error: Signal<Option<String>>,
    pub browse_loading: Signal<bool>,
    pub(crate) engine_anchor: Signal<Option<(u64, std::time::Instant, bool)>>,
    pub(crate) fading_progress: Signal<Option<f64>>,
    pub(crate) output_latency_ms: Signal<u64>,
    pub(crate) spotify_scrobble_token: Signal<u64>,
    pub(crate) spotify_token: Signal<Option<String>>,

    pub(crate) spotify_host: Signal<Option<::server::spotify::host::SpotifyHost>>,
    pub spotify_device: Signal<Option<String>>,
    pub(crate) spotify_pending_uri: Signal<Option<String>>,
    pub(crate) spotify_activated: Signal<bool>,
    pub spotify_device_override: Signal<Option<String>>,
    pub(crate) spotify_progress_anchor: Signal<Option<(u64, std::time::Instant)>>,
    pub(crate) spotify_host_starting: Signal<bool>,
    pub(crate) spotify_start_task: Signal<Option<dioxus_core::Task>>,
    pub(crate) spotify_commanded: Signal<Option<(String, std::time::Instant)>>,
    pub(crate) spotify_device_chosen: Signal<bool>,
    pub external_active: Signal<bool>,
    pub(crate) external_lease_id: Signal<Option<String>>,
}

/// A buffered byte range of the current stream, for the seek-bar underlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferedRange {
    pub start: u64,
    pub end: u64,
    pub total: u64,
}

impl PlayerController {
    fn api(&self) -> Arc<dyn api::KopuzApi> {
        self.api.peek().clone()
    }

    fn claim_external_playback(&self) {
        if !*self.external_active.peek() || self.external_lease_id.peek().is_some() {
            return;
        }
        let api = self.api();
        let device = self.spotify_device_override.peek().clone();
        let active = self.external_active;
        let mut lease_id = self.external_lease_id;
        spawn(async move {
            match api
                .claim_external_playback(api::ExternalPlayback {
                    kind: "spotify".to_string(),
                    device,
                })
                .await
            {
                Ok(lease) => {
                    if *active.peek() && lease_id.peek().is_none() {
                        lease_id.set(Some(lease.lease_id));
                    } else if let Err(error) = api.release_external_playback(lease.lease_id).await {
                        tracing::warn!(%error, "late external playback release failed");
                    }
                }
                Err(error) => tracing::warn!(%error, "external playback claim failed"),
            }
        });
    }

    fn command(&self, command: api::PlayerCommand) {
        let api = self.api();
        spawn(async move {
            if let Err(error) = api.player_command(command).await {
                tracing::warn!(%error, "player command failed");
            }
        });
    }

    pub(crate) fn report_external_state(&self, completed: bool) {
        let Some(lease_id) = self.external_lease_id.peek().clone() else {
            return;
        };
        let report = api::ExternalPlaybackReport {
            lease_id: lease_id.clone(),
            track: self
                .current_track_snapshot
                .peek()
                .as_ref()
                .map(daemon::track_info_for_persistence),
            position_ms: self.current_song_progress.peek().saturating_mul(1000),
            playing: !completed && *self.is_playing.peek(),
            completed,
            device: self.spotify_device_override.peek().clone(),
        };
        let api = self.api();
        let mut current_lease_id = self.external_lease_id;
        spawn(async move {
            if let Err(error) = api.report_external_playback(report).await {
                tracing::warn!(%error, "external playback report failed");
                if error.code == api::ErrorCode::Conflict
                    && current_lease_id.peek().as_deref() == Some(lease_id.as_str())
                {
                    current_lease_id.set(None);
                }
            }
        });
    }

    fn tracks_request(
        tracks: Vec<Track>,
        mode: api::QueueMode,
        start_index: Option<usize>,
        shuffle: Option<bool>,
        insert_index: Option<usize>,
    ) -> api::SetQueueRequest {
        let context = if tracks.iter().all(|track| track.id.service().is_some()) {
            api::QueueContext::InlineTracks {
                tracks: tracks
                    .iter()
                    .map(daemon::track_info_for_persistence)
                    .collect(),
            }
        } else {
            api::QueueContext::Tracks {
                keys: tracks
                    .into_iter()
                    .map(|track| track.id.key().into_owned())
                    .collect(),
            }
        };
        api::SetQueueRequest {
            mode,
            context,
            start_index: start_index.map(|index| index as u32),
            shuffle,
            insert_index: insert_index.map(|index| index as u32),
        }
    }

    fn is_spotify_track(track: &Track) -> bool {
        track.id.service() == Some(config::MusicService::Spotify)
    }

    /// Retrieves the queue index for a given index, taking into account the shuffle state.
    pub fn get_queue_index(&self, idx: usize) -> Option<usize> {
        if *self.shuffle.peek() {
            self.shuffle_order.peek().get(idx).cloned()
        } else {
            Some(idx)
        }
    }

    pub fn get_current_track_index(&self) -> Option<usize> {
        self.get_queue_index(*self.current_queue_index.peek())
    }

    pub fn get_track_at(&self, idx: usize) -> Option<Track> {
        let idx = self.get_queue_index(idx)?;
        self.queue.peek().get(idx).cloned()
    }

    pub fn current_track(&self) -> Option<Track> {
        self.get_track_at(*self.current_queue_index.peek())
    }

    pub fn has_next_track(&self) -> bool {
        let queue_len = self.queue.peek().len();
        if queue_len == 0 {
            return false;
        }
        match *self.loop_mode.peek() {
            LoopMode::Track | LoopMode::Queue => true,
            LoopMode::None => *self.current_queue_index.peek() + 1 < queue_len,
        }
    }

    /// Play the track at a physical queue index (a track-list row click).
    /// While shuffle is on the permutation re-pins around it, exactly like the
    /// daemon's jump.
    pub fn play_track(&mut self, idx: usize) {
        let tracks = self.queue.peek().clone();
        self.play_queue_at(tracks, idx);
    }

    /// Replace the engine queue with a track-list result and start its physical row.
    pub fn play_queue_at(&mut self, tracks: Vec<Track>, idx: usize) {
        if idx >= tracks.len() {
            return;
        }
        let shuffle = *self.shuffle.peek();
        self.play_replacement(tracks, Some(idx), Some(shuffle));
    }

    /// Play the track at a play-order (logical) index, as the queue view uses.
    pub fn play_track_no_history(&mut self, idx: usize) {
        let Some(physical) = self.get_queue_index(idx) else {
            return;
        };
        let Some(track) = self.queue.peek().get(physical).cloned() else {
            return;
        };
        if Self::is_spotify_track(&track) {
            if !*self.external_active.peek() {
                self.command(api::PlayerCommand::Stop);
            }
            self.external_active.set(true);
            self.current_queue_index.set(idx);
            self.hydrate_current_track_metadata(idx, 0);
            self.start_spotify_track(&track);
            return;
        }
        if *self.external_active.peek() {
            self.play_physical(physical);
            return;
        }
        let api = self.api();
        spawn(async move {
            let _ = api
                .queue_edit(api::QueueEdit::Jump { index: idx as u32 })
                .await;
        });
    }

    /// Command the Spotify transport to start `track`; the caller has already
    /// positioned the queue signals.
    fn start_spotify_track(&mut self, track: &Track) {
        let reader::TrackId::Server { item_id, .. } = &track.id else {
            return;
        };
        self.spotify_progress_anchor
            .set(Some((0, std::time::Instant::now())));
        self.is_playing.set(true);
        self.spotify_play(item_id, track);
    }

    /// Play the track at a physical queue index. Spotify tracks route to the
    /// browser host; everything else is a session jump. Returning from
    /// external playback pushes the locally accumulated queue back into the
    /// daemon so nothing the Spotify session added is lost.
    fn play_physical(&mut self, physical_idx: usize) {
        let Some(track) = self.queue.peek().get(physical_idx).cloned() else {
            return;
        };
        if Self::is_spotify_track(&track) {
            self.play_spotify_physical(physical_idx, track);
            return;
        }
        let shuffle = *self.shuffle.peek();
        if *self.external_active.peek() {
            self.stop_external_playback();
            let tracks = self.queue.peek().clone();
            let api = self.api();
            let request = Self::tracks_request(
                tracks,
                api::QueueMode::Replace,
                Some(physical_idx),
                Some(shuffle),
                None,
            );
            spawn(async move {
                if let Err(error) = api.set_queue(request).await {
                    tracing::warn!(%error, "external queue handoff failed");
                }
            });
            return;
        }
        let logical_idx = if shuffle {
            self.shuffle_order
                .peek()
                .iter()
                .position(|index| *index == physical_idx)
                .unwrap_or(physical_idx)
        } else {
            physical_idx
        };
        let api = self.api();
        spawn(async move {
            if let Err(error) = api
                .queue_edit(api::QueueEdit::Jump {
                    index: logical_idx as u32,
                })
                .await
            {
                tracing::warn!(%error, "queue jump failed");
            }
        });
    }

    fn play_spotify_physical(&mut self, physical_idx: usize, track: Track) {
        if !*self.external_active.peek() {
            self.command(api::PlayerCommand::Stop);
        }
        let current = *self.current_queue_index.peek();
        self.history.with_mut(|history| {
            if history.last() != Some(&current) {
                history.push(current);
            }
        });
        let logical_idx = if *self.shuffle.peek() {
            self.repair_shuffle_order();
            self.shuffle_order
                .peek()
                .iter()
                .position(|index| *index == physical_idx)
                .unwrap_or(physical_idx)
        } else {
            physical_idx
        };
        self.current_queue_index.set(logical_idx);
        self.external_active.set(true);
        self.hydrate_current_track_metadata(logical_idx, 0);
        self.start_spotify_track(&track);
    }

    pub fn play_queue_linear(&mut self, tracks: Vec<Track>) {
        self.play_replacement(tracks, None, None);
    }

    /// Historical shuffle-play semantics: a random starting track, with the
    /// shuffle toggle left as the user set it.
    pub fn play_queue_shuffled(&mut self, tracks: Vec<Track>) {
        use rand::RngExt;
        if tracks.is_empty() {
            return;
        }
        let start = rand::rng().random_range(0..tracks.len());
        self.play_replacement(tracks, Some(start), None);
    }

    fn play_replacement(
        &mut self,
        tracks: Vec<Track>,
        start_index: Option<usize>,
        shuffle: Option<bool>,
    ) {
        if tracks.is_empty() {
            return;
        }
        if tracks.first().is_some_and(Self::is_spotify_track) {
            self.queue.set(tracks);
            self.history.write().clear();
            if shuffle == Some(true) {
                self.shuffle.set(true);
            }
            if *self.shuffle.peek() {
                self.rebuild_shuffle_order();
            }
            let start = start_index.unwrap_or(0);
            let target = self.queue.peek().get(start).cloned();
            if let Some(track) = target {
                self.play_spotify_physical(start, track);
            }
            return;
        }
        if *self.external_active.peek() {
            self.stop_external_playback();
        }
        let api = self.api();
        let request =
            Self::tracks_request(tracks, api::QueueMode::Replace, start_index, shuffle, None);
        spawn(async move {
            if let Err(error) = api.set_queue(request).await {
                tracing::warn!(%error, "queue replacement failed");
            }
        });
    }

    pub fn add_to_queue(&mut self, tracks: impl IntoIterator<Item = Track>) {
        let tracks: Vec<Track> = tracks.into_iter().collect();
        if tracks.is_empty() {
            return;
        }
        if *self.external_active.peek() {
            let first_new = self.queue.peek().len();
            let count = tracks.len();
            if *self.shuffle.peek() {
                self.repair_shuffle_order();
            }
            self.queue.with_mut(|queue| queue.extend(tracks));
            if *self.shuffle.peek() {
                self.shuffle_order
                    .with_mut(|order| order.extend(first_new..first_new + count));
            }
            return;
        }
        let api = self.api();
        let request = Self::tracks_request(tracks, api::QueueMode::Append, None, None, None);
        spawn(async move {
            let _ = api.set_queue(request).await;
        });
    }

    pub fn queue_play_next(&mut self, tracks: impl IntoIterator<Item = Track>) {
        let tracks: Vec<Track> = tracks.into_iter().collect();
        if tracks.is_empty() {
            return;
        }
        if *self.external_active.peek() {
            let insert_at = (*self.current_queue_index.peek() + 1).min(self.queue.peek().len());
            self.insert_queue_tracks_local(insert_at, tracks);
            return;
        }
        let api = self.api();
        let request = Self::tracks_request(tracks, api::QueueMode::PlayNext, None, None, None);
        spawn(async move {
            let _ = api.set_queue(request).await;
        });
    }

    pub fn play_next(&mut self) {
        if *self.external_active.peek() {
            self.external_step(1);
            return;
        }
        self.command(api::PlayerCommand::Next);
    }

    pub fn play_prev(&mut self) {
        if *self.external_active.peek() {
            self.external_step(-1);
            return;
        }
        self.command(api::PlayerCommand::Previous);
    }

    /// Local queue stepping for external playback, where the daemon is not
    /// driving the advance.
    fn external_step(&mut self, delta: i64) {
        let queue_len = if *self.shuffle.peek() {
            self.repair_shuffle_order();
            self.shuffle_order.peek().len()
        } else {
            self.queue.peek().len()
        };
        if queue_len == 0 {
            return;
        }
        let idx = *self.current_queue_index.peek() as i64;
        let loop_mode = *self.loop_mode.peek();
        let next = match loop_mode {
            LoopMode::Track => idx,
            _ => {
                let stepped = idx + delta;
                if stepped < 0 {
                    (queue_len as i64) - 1
                } else if stepped >= queue_len as i64 {
                    if loop_mode == LoopMode::None && delta > 0 {
                        self.spotify_transport_pause();
                        self.is_playing.set(false);
                        return;
                    }
                    0
                } else {
                    stepped
                }
            }
        } as usize;
        let Some(physical) = self.get_queue_index(next) else {
            return;
        };
        let Some(track) = self.queue.peek().get(physical).cloned() else {
            return;
        };
        self.current_queue_index.set(next);
        self.hydrate_current_track_metadata(next, 0);
        if Self::is_spotify_track(&track) {
            self.start_spotify_track(&track);
        } else {
            self.play_physical(physical);
        }
    }

    /// Insert tracks at a play-order position, as the queue view's drag-drop
    /// uses.
    pub fn insert_queue_tracks(&mut self, insert_at: usize, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        if *self.external_active.peek() {
            self.insert_queue_tracks_local(insert_at, tracks);
            return;
        }
        let api = self.api();
        let request =
            Self::tracks_request(tracks, api::QueueMode::Insert, None, None, Some(insert_at));
        spawn(async move {
            let _ = api.set_queue(request).await;
        });
    }

    fn insert_queue_tracks_local(&mut self, insert_at: usize, tracks: Vec<Track>) {
        let count = tracks.len();
        if *self.shuffle.peek() {
            self.repair_shuffle_order();
            let visual_insert = insert_at.min(self.shuffle_order.peek().len());
            let physical_insert = self
                .shuffle_order
                .peek()
                .get(visual_insert)
                .copied()
                .unwrap_or_else(|| self.queue.peek().len());
            self.queue.with_mut(|queue| {
                let insert_pos = physical_insert.min(queue.len());
                for (offset, track) in tracks.into_iter().enumerate() {
                    queue.insert(insert_pos + offset, track);
                }
            });
            self.shuffle_order.with_mut(|order| {
                for idx in order.iter_mut() {
                    if *idx >= physical_insert {
                        *idx += count;
                    }
                }
                for offset in 0..count {
                    order.insert(visual_insert + offset, physical_insert + offset);
                }
            });
            let current = *self.current_queue_index.peek();
            if visual_insert <= current {
                self.current_queue_index.set(current + count);
            }
            self.history.with_mut(|history| {
                for idx in history.iter_mut() {
                    if *idx >= visual_insert {
                        *idx += count;
                    }
                }
            });
        } else {
            let insert_at = insert_at.min(self.queue.peek().len());
            self.queue.with_mut(|queue| {
                for (offset, track) in tracks.into_iter().enumerate() {
                    queue.insert(insert_at + offset, track);
                }
            });
            let current = *self.current_queue_index.peek();
            if insert_at <= current {
                self.current_queue_index.set(current + count);
            }
            self.history.with_mut(|history| {
                for idx in history.iter_mut() {
                    if *idx >= insert_at {
                        *idx += count;
                    }
                }
            });
        }
    }

    pub fn pause(&mut self) {
        if *self.external_active.peek() {
            self.spotify_transport_pause();
            self.is_playing.set(false);
            return;
        }
        self.is_playing.set(false);
        self.command(api::PlayerCommand::Pause);
    }

    pub fn resume(&mut self) {
        if *self.external_active.peek() {
            self.spotify_transport_resume();
            self.is_playing.set(true);
            return;
        }
        self.is_playing.set(true);
        self.command(api::PlayerCommand::Play);
    }

    pub fn toggle(&mut self) {
        if *self.is_playing.peek() {
            self.pause();
        } else {
            self.resume();
        }
    }

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
        self.current_song_progress.set(time.as_secs());
        self.engine_anchor.set(Some((
            time.as_millis() as u64,
            std::time::Instant::now(),
            *self.is_playing.peek(),
        )));
        self.command(api::PlayerCommand::Seek {
            position_ms: time.as_millis() as u64,
        });
    }

    /// Volume changes route through the session; the local signal keeps the
    /// slider responsive.
    pub fn set_volume(&mut self, value: f32) {
        self.volume.set(value);
        self.command(api::PlayerCommand::SetVolume { volume: value });
    }

    /// Apply an equalizer preview to the engine without committing it to the
    /// config; a commit goes through the config signal and the session's
    /// config bridge instead.
    pub fn preview_equalizer(&self, equalizer: config::EqualizerSettings) {
        let api = self.api();
        spawn(async move {
            let value = match serde_json::to_value(equalizer) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(%error, "equalizer preview encoding failed");
                    return;
                }
            };
            if let Err(error) = api.preview_equalizer(value).await {
                tracing::warn!(%error, "equalizer preview failed");
            }
        });
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if *self.shuffle.peek() != on {
            self.toggle_shuffle();
        }
    }

    pub fn toggle_shuffle(&mut self) {
        let now_on = !*self.shuffle.peek();
        if *self.external_active.peek() {
            self.shuffle.set(now_on);
            if now_on {
                self.rebuild_shuffle_order();
            } else {
                let current = *self.current_queue_index.peek();
                let physical = self
                    .shuffle_order
                    .peek()
                    .get(current)
                    .copied()
                    .unwrap_or(current);
                self.current_queue_index.set(physical);
            }
            return;
        }
        self.shuffle.set(now_on);
        self.command(api::PlayerCommand::SetMode {
            shuffle: Some(now_on),
            loop_mode: None,
        });
    }

    pub fn set_loop_mode(&mut self, mode: LoopMode) {
        self.loop_mode.set(mode);
        self.command(api::PlayerCommand::SetMode {
            shuffle: None,
            loop_mode: Some(mode),
        });
    }

    pub fn toggle_loop(&mut self) {
        let next = self.loop_mode.peek().next();
        self.set_loop_mode(next);
    }

    pub fn play_radio(&mut self, station_id: &str, stream_id: &str) {
        if *self.external_active.peek() {
            self.stop_external_playback();
        }
        let api = self.api();
        let request = api::SetQueueRequest {
            mode: api::QueueMode::Replace,
            context: api::QueueContext::Radio {
                station_id: station_id.to_string(),
                stream_id: stream_id.to_string(),
            },
            start_index: Some(0),
            shuffle: None,
            insert_index: None,
        };
        spawn(async move {
            if let Err(error) = api.set_queue(request).await {
                tracing::warn!(%error, "radio start failed");
            }
        });
    }

    pub fn move_queue_item(&mut self, from: usize, to: usize) {
        if *self.external_active.peek() {
            let len = if *self.shuffle.peek() {
                self.shuffle_order.peek().len()
            } else {
                self.queue.peek().len()
            };
            if from >= len || to >= len || from == to {
                return;
            }
            if *self.shuffle.peek() {
                self.shuffle_order.with_mut(|order| {
                    let physical = order.remove(from);
                    order.insert(to, physical);
                });
            } else {
                self.queue.with_mut(|queue| {
                    let track = queue.remove(from);
                    queue.insert(to, track);
                });
            }
            let current =
                daemon::QueueModel::remap_queue_index(*self.current_queue_index.peek(), from, to);
            self.current_queue_index.set(current);
            self.history.with_mut(|history| {
                for index in history {
                    *index = daemon::QueueModel::remap_queue_index(*index, from, to);
                }
            });
            return;
        }
        let api = self.api();
        spawn(async move {
            let _ = api
                .queue_edit(api::QueueEdit::Move {
                    from: from as u32,
                    to: to as u32,
                })
                .await;
        });
    }

    pub fn swap_queue_item(&mut self, from: usize, to: usize) {
        self.move_queue_item(from, to);
    }

    /// Clear frontend-owned state after the daemon has switched sources and
    /// reset its engine queue.
    pub fn reset_for_backend_switch(&mut self) {
        self.stop_external_playback();
        self.playback_error.set(None);
        self.clear_current_track_metadata();
        self.queue.write().clear();
        self.history.write().clear();
        self.current_queue_index.set(0);
    }

    /// Zero for an external player: its position comes from the service, not us.
    pub fn output_latency_secs(&self) -> f64 {
        if *self.external_active.peek() {
            return 0.0;
        }
        *self.output_latency_ms.peek() as f64 / 1000.0
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
        if let Some(fading) = *self.fading_progress.peek() {
            return fading;
        }
        if let Some((ms, at, playing)) = *self.engine_anchor.peek() {
            let mut pos = ms as f64 / 1000.0;
            if playing {
                pos += at.elapsed().as_secs_f64();
            }
            let dur = *self.current_song_duration.peek();
            if dur > 0 && dur != u64::MAX {
                pos = pos.min(dur as f64);
            }
            return pos;
        }
        *self.current_song_progress.peek() as f64
    }

    pub(crate) fn cover_url_for_track(&self, track: &Track) -> String {
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

    /// Adopt a Spotify Connect track started elsewhere; see the old
    /// controller's notes on shuffle stability.
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

    pub(crate) fn rebuild_shuffle_order(&mut self) {
        use rand::seq::SliceRandom;
        let queue_len = self.queue.peek().len();
        let current_idx = *self.current_queue_index.peek();
        if queue_len == 0 {
            self.shuffle_order.set(Vec::new());
            self.current_queue_index.set(0);
            return;
        }
        let mut order: Vec<usize> = Vec::with_capacity(queue_len);
        order.push(current_idx);
        let mut rest: Vec<usize> = (0..queue_len).filter(|&i| i != current_idx).collect();
        rest.shuffle(&mut rand::rng());
        order.extend(rest);
        self.current_queue_index.set(0);
        self.shuffle_order.set(order);
    }

    pub(crate) fn shuffle_position_of(&mut self, physical_idx: usize) -> Option<usize> {
        self.repair_shuffle_order();
        self.shuffle_order
            .peek()
            .iter()
            .position(|&idx| idx == physical_idx)
    }

    fn repair_shuffle_order(&mut self) {
        use rand::seq::SliceRandom;
        let queue_len = self.queue.peek().len();
        let covered = self.shuffle_order.peek().len() == queue_len
            && self
                .shuffle_order
                .peek()
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == queue_len;
        if covered {
            return;
        }
        let order = self.shuffle_order.peek().clone();
        let mut repaired: Vec<usize> = Vec::with_capacity(queue_len);
        for idx in order {
            if idx < queue_len && !repaired.contains(&idx) {
                repaired.push(idx);
            }
        }
        let mut missing: Vec<usize> = (0..queue_len)
            .filter(|idx| !repaired.contains(idx))
            .collect();
        missing.shuffle(&mut rand::rng());
        repaired.extend(missing);
        self.shuffle_order.set(repaired);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn use_player_controller(
    api_handle: Arc<dyn api::KopuzApi>,
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
    _config_loaded_ok: Signal<bool>,
) -> PlayerController {
    let api = use_signal(move || api_handle);
    let loading = use_signal(|| false);
    let browse_loading = use_signal(|| false);
    let is_loading = use_memo(move || *loading.read() || *browse_loading.read());
    let history = use_signal(Vec::new);
    let shuffle = use_signal(|| false);
    let shuffle_order = use_signal(Vec::<usize>::new);
    let loop_mode = use_signal(|| LoopMode::None);
    let buffered_ranges = use_signal(Vec::<BufferedRange>::new);
    let playback_error = use_signal(|| None::<String>);
    let engine_anchor = use_signal(|| None::<(u64, std::time::Instant, bool)>);
    let fading_progress = use_signal(|| None::<f64>);
    let output_latency_ms = use_signal(|| 0u64);
    let spotify_scrobble_token = use_signal(|| 0u64);
    let spotify_token = use_signal(|| None::<String>);

    let spotify_host = use_signal(|| None::<::server::spotify::host::SpotifyHost>);
    let spotify_device = use_signal(|| None::<String>);
    let spotify_pending_uri = use_signal(|| None::<String>);
    let spotify_activated = use_signal(|| false);
    let spotify_device_override = use_signal(|| None::<String>);
    let spotify_progress_anchor = use_signal(|| None::<(u64, std::time::Instant)>);
    let spotify_host_starting = use_signal(|| false);
    let spotify_start_task = use_signal(|| None::<dioxus_core::Task>);
    let spotify_commanded = use_signal(|| None::<(String, std::time::Instant)>);
    let spotify_device_chosen = use_signal(|| false);
    let external_active = use_signal(|| false);
    let external_lease_id = use_signal(|| None::<String>);

    let ctrl = PlayerController {
        api,
        is_playing,
        is_loading,
        loading,
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
        current_song_cover_url,
        current_track_snapshot,
        volume,
        config,
        playback_error,
        browse_loading,
        engine_anchor,
        fading_progress,
        output_latency_ms,
        spotify_scrobble_token,
        spotify_token,
        spotify_host,
        spotify_device,
        spotify_pending_uri,
        spotify_activated,
        spotify_device_override,
        spotify_progress_anchor,
        spotify_host_starting,
        spotify_start_task,
        spotify_commanded,
        spotify_device_chosen,
        external_active,
        external_lease_id,
    };

    let lease_ctrl = ctrl;
    use_effect(move || {
        let active = *lease_ctrl.external_active.read();
        let _ = lease_ctrl.spotify_device_override.read();
        let current_lease_id = lease_ctrl.external_lease_id.read().clone();
        if active && current_lease_id.is_none() {
            lease_ctrl.claim_external_playback();
            return;
        }
        if active {
            return;
        }
        let Some(release_id) = current_lease_id else {
            return;
        };
        let api = lease_ctrl.api();
        let mut lease_id = lease_ctrl.external_lease_id;
        spawn(async move {
            if let Err(error) = api.release_external_playback(release_id.clone()).await {
                tracing::warn!(%error, "external playback release failed");
            }
            if lease_id.peek().as_deref() == Some(release_id.as_str()) {
                lease_id.set(None);
            }
        });
    });

    let report_ctrl = ctrl;
    use_effect(move || {
        let active = *report_ctrl.external_active.read();
        let _ = report_ctrl.external_lease_id.read();
        let _ = report_ctrl.current_track_snapshot.read();
        let _ = report_ctrl.current_song_progress.read();
        let _ = report_ctrl.is_playing.read();
        let _ = report_ctrl.spotify_device_override.read();
        if active {
            report_ctrl.report_external_state(false);
        }
    });

    let renew_ctrl = ctrl;
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if *renew_ctrl.external_active.peek() {
                if renew_ctrl.external_lease_id.peek().is_none() {
                    renew_ctrl.claim_external_playback();
                } else {
                    renew_ctrl.report_external_state(false);
                }
            }
        }
    });

    crate::session_projector::use_session_projector(ctrl);
    ctrl
}
