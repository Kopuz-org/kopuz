//! The player controller, as a mirror over the daemon session.
//!
//! Every field is a projection of the daemon's `PlayerState` stream and every
//! transport method forwards a command. There is no second queue here and no
//! special case for a track the engine cannot decode: the daemon hands that
//! one to whatever can play it and reports back the same way.

use std::sync::Arc;
use std::time::Duration;

use api::KopuzApi;
use config::AppConfig;
use dioxus::prelude::*;
use reader::Track;

pub use api::LoopMode;

#[derive(Clone, Copy)]
pub struct PlayerController {
    pub(crate) api: Signal<Arc<dyn KopuzApi>>,
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
    /// The picture for what is playing, as a reference the daemon resolves.
    /// Every surface that paints the cover reads it through hooks::artwork.
    pub current_artwork: Signal<Option<api::ArtworkRef>>,
    /// The device an integration is playing on, when one owns playback.
    pub external_device: Signal<Option<String>>,
    pub(crate) output_latency_ms: Signal<u64>,
}

/// What to say while a mix is being built, and if it fails.
///
/// The strings come from the caller because the daemon has no locale and this
/// crate has no string table -- the surface that offers the action knows both.
#[derive(Clone, Debug)]
pub struct RadioNotices {
    pub starting: String,
    pub failed: String,
}

/// A buffered byte range of the current stream, for the seek-bar underlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferedRange {
    pub start: u64,
    pub end: u64,
    pub total: u64,
}

impl PlayerController {
    fn handle(&self) -> Arc<dyn KopuzApi> {
        self.api.peek().clone()
    }

    fn command(&self, command: api::PlayerCommand) {
        let handle = self.handle();
        spawn(async move {
            if let Err(error) = handle.player_command(command).await {
                tracing::warn!(%error, "session command failed");
            }
        });
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
        self.play_physical(idx);
    }

    /// Play the track at a play-order (logical) index, as the queue view uses.
    pub fn play_track_no_history(&mut self, idx: usize) {
        let handle = self.handle();
        spawn(async move {
            let _ = handle
                .queue_edit(api::QueueEdit::Jump { index: idx as u32 })
                .await;
        });
    }

    /// Play the track at a physical queue index: a jump the daemon re-pins the
    /// shuffle order around.
    fn play_physical(&mut self, physical_idx: usize) {
        let handle = self.handle();
        spawn(async move {
            let edit = api::QueueEdit::JumpPhysical {
                index: physical_idx as u32,
            };
            if let Err(error) = handle.queue_edit(edit).await {
                tracing::warn!(%error, "queue jump failed");
            }
        });
    }

    /// The keys of a track list, for the calls that name rows rather than
    /// carrying them. Every track the UI holds came from the daemon, so its
    /// key resolves there -- including a browse row it registered.
    fn keys_of(tracks: &[Track]) -> Vec<String> {
        tracks
            .iter()
            .map(|track| track.id.key().into_owned())
            .filter(|key| !key.is_empty())
            .collect()
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
        let keys = Self::keys_of(&tracks);
        let handle = self.handle();
        spawn(async move {
            let request = api::SetQueueRequest {
                mode: api::QueueMode::Replace,
                context: api::QueueContext::Tracks { keys },
                start_index: start_index.map(|index| index as u32),
                shuffle,
            };
            if let Err(error) = handle.set_queue(request).await {
                tracing::warn!(%error, "replacing the queue failed");
            }
        });
    }

    /// Queue rows by key. Library tracks and the catalog rows the daemon
    /// registered when it served them resolve the same way, so a browse tile
    /// plays without the client shipping a track list back.
    pub fn set_queue_keys(
        &mut self,
        keys: Vec<String>,
        mode: api::QueueMode,
        start_index: Option<u32>,
    ) {
        if keys.is_empty() {
            return;
        }
        let handle = self.handle();
        spawn(async move {
            let request = api::SetQueueRequest {
                mode,
                context: api::QueueContext::Tracks { keys },
                start_index,
                shuffle: None,
            };
            if let Err(error) = handle.set_queue(request).await {
                tracing::warn!(%error, "queueing by key failed");
            }
        });
    }

    pub fn add_to_queue(&mut self, tracks: impl IntoIterator<Item = Track>) {
        let tracks: Vec<Track> = tracks.into_iter().collect();
        self.set_queue_keys(Self::keys_of(&tracks), api::QueueMode::Append, None);
    }

    pub fn queue_play_next(&mut self, tracks: impl IntoIterator<Item = Track>) {
        let tracks: Vec<Track> = tracks.into_iter().collect();
        self.set_queue_keys(Self::keys_of(&tracks), api::QueueMode::PlayNext, None);
    }

    pub fn play_next(&mut self) {
        self.command(api::PlayerCommand::Next);
    }

    pub fn play_prev(&mut self) {
        self.command(api::PlayerCommand::Previous);
    }

    /// Insert tracks at a play-order position, as the queue view's drag-drop
    /// uses.
    pub fn insert_queue_tracks(&mut self, insert_at: usize, tracks: Vec<Track>) {
        let keys = Self::keys_of(&tracks);
        if keys.is_empty() {
            return;
        }
        let handle = self.handle();
        spawn(async move {
            let edit = api::QueueEdit::Insert {
                index: insert_at as u32,
                keys,
            };
            if let Err(error) = handle.queue_edit(edit).await {
                tracing::warn!(%error, "inserting into the queue failed");
            }
        });
    }

    pub fn pause(&mut self) {
        self.is_playing.set(false);
        self.command(api::PlayerCommand::Pause);
    }

    pub fn resume(&mut self) {
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
        let handle = self.handle();
        spawn(async move {
            if let Err(error) = handle.preview_equalizer(equalizer).await {
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

    /// Play the source's mix seeded by one track. The daemon fetches it and
    /// pins the seed at the front, so the track that was clicked plays first.
    pub fn play_track_radio(&mut self, key: String, notices: RadioNotices) {
        self.play_seeded_radio(api::QueueContext::TrackRadio { key }, notices);
    }

    pub fn play_playlist_radio(&mut self, id: String, notices: RadioNotices) {
        self.play_seeded_radio(api::QueueContext::PlaylistRadio { id }, notices);
    }

    /// Building a mix is a remote round trip that can take tens of seconds, so
    /// a notice goes up first: without it a click looks like it did nothing.
    /// A failure leaves the queue alone -- the music keeps playing rather than
    /// stopping on a network hiccup.
    fn play_seeded_radio(&mut self, context: api::QueueContext, notices: RadioNotices) {
        let handle = self.handle();
        spawn(async move {
            crate::toast::toast(&notices.starting);
            let request = api::SetQueueRequest {
                mode: api::QueueMode::Replace,
                context,
                start_index: Some(0),
                shuffle: None,
            };
            if let Err(error) = handle.set_queue(request).await {
                tracing::warn!(%error, "radio start failed");
                crate::toast::toast_error(&notices.failed);
            }
        });
    }

    pub fn play_radio(&mut self, station_id: &str, stream_id: &str) {
        let handle = self.handle();
        let request = api::SetQueueRequest {
            mode: api::QueueMode::Replace,
            context: api::QueueContext::Radio {
                station_id: station_id.to_string(),
                stream_id: stream_id.to_string(),
            },
            start_index: Some(0),
            shuffle: None,
        };
        spawn(async move {
            if let Err(error) = handle.set_queue(request).await {
                tracing::warn!(%error, "radio start failed");
            }
        });
    }

    pub fn move_queue_item(&mut self, from: usize, to: usize) {
        let handle = self.handle();
        spawn(async move {
            let _ = handle
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

    /// Hard reset when the active server changes: stop everything and clear
    /// the queue so a queued remote track cannot replay through the wrong
    /// backend.
    pub fn reset_for_backend_switch(&mut self) {
        self.playback_error.set(None);
        self.clear_current_track_metadata();
        self.queue.write().clear();
        self.history.write().clear();
        self.current_queue_index.set(0);
        let handle = self.handle();
        spawn(async move {
            let _ = handle.player_command(api::PlayerCommand::Stop).await;
            let request = api::SetQueueRequest {
                mode: api::QueueMode::Replace,
                context: api::QueueContext::Tracks { keys: Vec::new() },
                start_index: None,
                shuffle: None,
            };
            let _ = handle.set_queue(request).await;
        });
    }

    pub fn output_latency_secs(&self) -> f64 {
        *self.output_latency_ms.peek() as f64 / 1000.0
    }

    pub fn displayed_progress_secs_f64(&self) -> f64 {
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
}

#[allow(clippy::too_many_arguments)]
pub fn use_player_controller(
    api_handle: Arc<dyn KopuzApi>,
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
    let current_artwork = use_signal(|| None::<api::ArtworkRef>);
    let external_device = use_signal(|| None::<String>);

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
        current_artwork,
        external_device,
        output_latency_ms,
    };

    crate::session_projector::use_session_projector(ctrl);
    ctrl
}
