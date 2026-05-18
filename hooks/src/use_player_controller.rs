use config::AppConfig;
use config::BackBehavior;
use config::MusicService;
use dioxus::{logger::tracing, prelude::*};
use player::player::{NowPlayingMeta, Player};
use reader::{Library, Track};
use scrobble;
use std::time::Duration;
use utils;

#[cfg(not(target_arch = "wasm32"))]
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
    pub library: Signal<Library>,
    pub config: Signal<AppConfig>,
    pub play_generation: Signal<usize>,
    pending_resume: Signal<Option<PendingResumeState>>,
    pub pending_crossfade_ui: Signal<Option<PendingCrossfadeUiState>>,
    pub radio_task: Signal<Option<dioxus_core::Task>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingResumeState {
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
        track.path.to_string_lossy().to_string()
    }

    fn current_track(&self, idx: usize) -> Option<Track> {
        let idx = if *self.shuffle.peek() {
            *self
                .shuffle_order
                .peek()
                .get(idx)
                .expect("shuffle order index out of bounds")
        } else {
            idx
        };
        self.queue.peek().get(idx).cloned()
    }

    fn cover_url_for_track(&self, track: &Track) -> String {
        let path_str = Self::track_key(track);
        let scheme = path_str
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        match scheme.as_str() {
            "jellyfin" => self
                .config
                .read()
                .server
                .as_ref()
                .and_then(|server| {
                    utils::jellyfin_image::jellyfin_image_url_from_path(
                        &path_str,
                        &server.url,
                        server.access_token.as_deref(),
                        800,
                        90,
                    )
                })
                .unwrap_or_default(),
            "subsonic" | "custom" => self
                .config
                .read()
                .server
                .as_ref()
                .and_then(|server| {
                    utils::subsonic_image::subsonic_image_url_from_path(
                        &path_str,
                        &server.url,
                        server.access_token.as_deref(),
                        800,
                        90,
                    )
                })
                .unwrap_or_default(),
            _ => self
                .library
                .read()
                .albums
                .iter()
                .find(|album| album.id == track.album_id)
                .and_then(|album| utils::format_artwork_url(album.cover_path.as_ref()))
                .map(|url| url.as_ref().to_string())
                .unwrap_or_default(),
        }
    }

    fn clear_current_track_metadata(&mut self) {
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

    fn hydrate_current_track_metadata(&mut self, idx: usize, progress_secs: u64) {
        if let Some(track) = self.current_track(idx) {
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

    fn clear_pending_resume(&mut self) {
        self.pending_resume.set(None);
    }

    fn clear_pending_crossfade_ui(&mut self) {
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

    fn schedule_pending_crossfade_ui(
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

    fn set_pending_resume_for_track(&mut self, track: &Track, progress_secs: u64) {
        self.pending_resume.set(Some(PendingResumeState {
            track_path: Self::track_key(track),
            progress_secs: progress_secs.min(track.duration),
        }));
    }

    fn apply_restore_seek(&mut self, seek_secs: u64) {
        self.player.write().seek(Duration::from_secs(seek_secs));
        self.current_song_progress.set(seek_secs);
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
    fn remap_queue_index(index: usize, from: usize, to: usize) -> usize {
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

    #[cfg(not(target_arch = "wasm32"))]
    pub fn should_crossfade(&self) -> bool {
        self.config.peek().crossfade_seconds > 0
            && *self.is_playing.peek()
            && self.player.peek().can_resume()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn should_crossfade(&self) -> bool {
        false
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
                } else if shuffle && queue_len == 1 {
                    true
                } else if idx + 1 < queue_len {
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

    fn play_track_no_history_with_transition(&mut self, idx: usize, allow_crossfade: bool) {
        self.play_generation.with_mut(|g| *g += 1);
        let current_gen = *self.play_generation.peek();
        self.cancel_radio_task();

        if let Some(track) = self.current_track(idx) {
            let path_str = track.path.to_string_lossy().to_string();
            let (restore_seek_secs, clear_pending_resume_on_success) =
                self.pending_resume_seek(&track);
            let use_crossfade = allow_crossfade
                && self.should_crossfade()
                && restore_seek_secs.map_or(true, |secs| secs == 0);
            let outgoing_duration_secs = *self.current_song_duration.peek();
            let outgoing_progress_secs =
                (*self.current_song_progress.peek()).min(outgoing_duration_secs);
            if !use_crossfade {
                self.clear_pending_crossfade_ui();
            }
            let crossfade_duration =
                Duration::from_secs(self.config.peek().crossfade_seconds as u64);
            let scheme = path_str
                .split(':')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_radio_item = scheme.as_str() == "radio";
            let is_server_item = matches!(scheme.as_str(), "jellyfin" | "subsonic" | "custom");

            if is_server_item || is_radio_item {
                let parts: Vec<&str> = path_str.split(':').collect();
                let id = parts.get(1).unwrap_or(&"").to_string();
                let stream_id = parts.get(2).unwrap_or(&"").to_string();

                // Check offline cache first
                #[cfg(not(target_arch = "wasm32"))]
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
                    if let Some(local_path) = offline_path {
                        if let Ok((source, hint)) = decoder::open_file(&local_path) {
                            if !use_crossfade {
                                self.current_queue_index.set(idx);
                                self.player.write().stop_for_transition();
                                self.is_playing.set(false);
                            }

                            let cover_url = self.cover_url_for_track(&track);
                            if !use_crossfade {
                                self.hydrate_current_track_metadata(idx, 0);
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
                                        eprintln!("Offline playback error: {e}");
                                        is_loading.set(false);
                                        skip_in_progress.set(false);
                                        return;
                                    }
                                    player.write().set_volume(*volume.peek());
                                    if let Some(seek_secs) = restore_seek_secs {
                                        if seek_secs > 0 {
                                            player.write().seek(Duration::from_secs(seek_secs));
                                        }
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
                                        current_song_progress.set(0);
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
                }

                if let Some((stream_url, cover_url)) = {
                    if is_radio_item {
                        let stream_url = radio::stations::stream_url(&id, &stream_id);
                        Some((stream_url.to_string(), String::new()))
                    } else {
                        let conf = self.config.read();
                        conf.server.as_ref().map(|server| match server.service {
                            MusicService::Jellyfin => {
                                let mut stream_url =
                                    format!("{}/Audio/{}/stream?static=true", server.url, id);
                                if let Some(token) = &server.access_token {
                                    stream_url.push_str(&format!("&api_key={}", token));
                                }

                                let cover_url = {
                                    let path_str = track.path.to_string_lossy();
                                    utils::jellyfin_image::jellyfin_image_url_from_path(
                                        &path_str,
                                        &server.url,
                                        server.access_token.as_deref(),
                                        800,
                                        90,
                                    )
                                    .unwrap_or_default()
                                };

                                (stream_url, cover_url)
                            }
                            MusicService::Subsonic | MusicService::Custom => {
                                let (stream_url, cover_url) =
                                    if let (Some(password), Some(username)) =
                                        (&server.access_token, &server.user_id)
                                    {
                                        let remote = ::server::subsonic::SubsonicClient::new(
                                            &server.url,
                                            username,
                                            password,
                                        );
                                        let stream_url = remote.stream_url(&id).unwrap_or_default();
                                        let cover_url =
                                            utils::subsonic_image::subsonic_image_url_from_path(
                                                &path_str,
                                                &server.url,
                                                server.access_token.as_deref(),
                                                800,
                                                90,
                                            )
                                            .or_else(|| remote.cover_art_url(&id, Some(800)).ok())
                                            .unwrap_or_default();
                                        (stream_url, cover_url)
                                    } else {
                                        (String::new(), String::new())
                                    };

                                (stream_url, cover_url)
                            }
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
                    let play_generation = self.play_generation;
                    let volume = self.volume;
                    let mut current_song_progress = self.current_song_progress;
                    let mut pending_crossfade_ui = self.pending_crossfade_ui;
                    let mut pending_resume = self.pending_resume;
                    let cfg_signal = self.config;
                    let mut radio_task = self.radio_task;
                    let mut current_song_title = self.current_song_title;
                    let mut current_song_artist = self.current_song_artist;
                    let mut current_song_album = self.current_song_album;
                    let mut current_song_cover_url = self.current_song_cover_url;

                    if !use_crossfade {
                        self.hydrate_current_track_metadata(idx, 0);
                        self.current_song_cover_url.set(cover_url.clone());
                    }

                    self.is_loading.set(true);

                    let is_radio = track.path.to_string_lossy().starts_with("radio:");

                    #[cfg(not(target_arch = "wasm32"))]
                    spawn(async move {
                        let stream = utils::stream_buffer::StreamBuffer::new(stream_url, is_radio);
                        let source_res = tokio::task::spawn_blocking(move || {
                            if is_radio {
                                let (source, hint) = decoder::from_stream_with_hint(stream, "ogg");
                                Ok::<_, std::io::Error>((source, hint))
                            } else {
                                stream.wait_for_total_size();
                                let len = stream.known_total_size();
                                let (source, hint) = decoder::from_stream_with_len(stream, len);
                                Ok::<_, std::io::Error>((source, hint))
                            }
                        })
                        .await;

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
                                    eprintln!("Playback error: {e}");
                                    is_loading.set(false);
                                    skip_in_progress.set(false);
                                    return;
                                }
                                player.write().set_volume(*volume.peek());
                                if let Some(seek_secs) = restore_seek_secs {
                                    if seek_secs > 0 {
                                        player.write().seek(Duration::from_secs(seek_secs));
                                        current_song_progress.set(seek_secs);
                                    }
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
                                    current_song_progress.set(0);
                                }
                                if clear_pending_resume_on_success {
                                    pending_resume.set(None);
                                }
                                is_loading.set(false);
                                is_playing.set(true);
                                skip_in_progress.set(false);

                                let is_radio_item =
                                    track.path.to_string_lossy().starts_with("radio:");
                                let path_lossy = track.path.to_string_lossy().to_string();
                                let parts: Vec<&str> = path_lossy.split(':').collect();
                                let (station_id, stream_id) = if is_radio_item {
                                    (
                                        parts.get(1).unwrap_or(&"").to_string(),
                                        parts.get(2).unwrap_or(&"").to_string(),
                                    )
                                } else {
                                    (String::new(), String::new())
                                };

                                if let Some(task) = radio_task.take() {
                                    task.cancel();
                                }

                                if is_radio_item {
                                    {
                                        let provider: Box<dyn radio::RadioMetadataProvider> =
                                            match station_id.as_str() {
                                                "listen_moe" => {
                                                    Box::new(radio::listen_moe::ListenMoeProvider)
                                                }
                                                "j1" => Box::new(radio::j1::J1Provider),
                                                "doujinstyle" => Box::new(
                                                    radio::doujinstyle::DoujinstyleProvider,
                                                ),
                                                "vocaloid" => {
                                                    Box::new(radio::vocaloid::VocaloidProvider)
                                                }
                                                _ => {
                                                    tracing::warn!(
                                                        "[radio] No metadata provider for station: {}",
                                                        station_id
                                                    );
                                                    is_loading.set(false);
                                                    is_playing.set(true);
                                                    skip_in_progress.set(false);
                                                    return;
                                                }
                                            };

                                        let task = spawn(async move {
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
                                    }
                                }
                                // Don't scrobble if the track is a radio item
                                if !is_radio_item {
                                    let scrobble_track = track.clone();
                                    let scrobble_gen = current_gen;
                                    let scrobble_play_gen = play_generation;
                                    let scrobble_cfg = cfg_signal;
                                    let scrobble_id = id.clone();
                                    let duration_secs = scrobble_track.duration;
                                    let threshold_secs =
                                        std::cmp::min(240, (duration_secs / 2) as u64);

                                    spawn(async move {
                                        // track must be longer than 30 seconds
                                        if duration_secs < 30 {
                                            return;
                                        }

                                        {
                                            let subsonic_creds = {
                                                let conf = scrobble_cfg.read();
                                                conf.server.as_ref().and_then(|s| {
                                                    if matches!(
                                                        s.service,
                                                        MusicService::Subsonic
                                                            | MusicService::Custom
                                                    ) {
                                                        if let (Some(pw), Some(un)) =
                                                            (&s.access_token, &s.user_id)
                                                        {
                                                            Some((
                                                                s.url.clone(),
                                                                un.clone(),
                                                                pw.clone(),
                                                            ))
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                })
                                            };
                                            if let Some((url, username, password)) = subsonic_creds
                                            {
                                                let client =
                                                    ::server::subsonic::SubsonicClient::new(
                                                        &url, &username, &password,
                                                    );
                                                if let Err(e) =
                                                    client.scrobble_now_playing(&scrobble_id).await
                                                {
                                                    tracing::warn!(
                                                        "Subsonic now-playing failed: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }

                                        // Last.fm now-playing
                                        let lastfm_api_key =
                                            scrobble_cfg.read().lastfm_api_key.clone();
                                        let lastfm_api_secret =
                                            scrobble_cfg.read().lastfm_api_secret.clone();
                                        let lastfm_session_key =
                                            scrobble_cfg.read().lastfm_session_key.clone();
                                        let has_lastfm = !lastfm_api_key.is_empty()
                                            && !lastfm_api_secret.is_empty();

                                        if has_lastfm {
                                            let playing_now = scrobble::lastfm::make_playing_now(
                                                &scrobble_track.artist,
                                                &scrobble_track.title,
                                                Some(&scrobble_track.album),
                                            );
                                            if let Err(e) = scrobble::lastfm::submit_now_playing(
                                                &lastfm_api_key,
                                                &lastfm_api_secret,
                                                &lastfm_session_key,
                                                &playing_now,
                                            )
                                            .await
                                            {
                                                tracing::warn!("Last.fm now playing failed: {}", e);
                                            }
                                        }

                                        // MusicBrainz playing_now
                                        let token_raw =
                                            scrobble_cfg.read().musicbrainz_token.clone();
                                        if !token_raw.is_empty() {
                                            let auth = if token_raw.contains(' ') {
                                                token_raw.clone()
                                            } else {
                                                format!("Token {}", token_raw)
                                            };
                                            let playing_now =
                                                scrobble::musicbrainz::make_playing_now(
                                                    &scrobble_track.artist,
                                                    &scrobble_track.title,
                                                    Some(&scrobble_track.album),
                                                );
                                            if let Err(e) = scrobble::musicbrainz::submit_listens(
                                                &auth,
                                                vec![playing_now],
                                                "playing_now",
                                            )
                                            .await
                                            {
                                                tracing::warn!(
                                                    "MusicBrainz playing_now failed: {}",
                                                    e
                                                );
                                            }
                                        }

                                        // threshold sleep
                                        tokio::time::sleep(Duration::from_secs(threshold_secs))
                                            .await;

                                        if *scrobble_play_gen.read() != scrobble_gen {
                                            return;
                                        }

                                        // post-threshold: actual scrobbles

                                        // Subsonic scrobble
                                        {
                                            let subsonic_creds = {
                                                let conf = scrobble_cfg.read();
                                                conf.server.as_ref().and_then(|s| {
                                                    if matches!(
                                                        s.service,
                                                        MusicService::Subsonic
                                                            | MusicService::Custom
                                                    ) {
                                                        if let (Some(pw), Some(un)) =
                                                            (&s.access_token, &s.user_id)
                                                        {
                                                            Some((
                                                                s.url.clone(),
                                                                un.clone(),
                                                                pw.clone(),
                                                            ))
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                })
                                            };
                                            if let Some((url, username, password)) = subsonic_creds
                                            {
                                                let client =
                                                    ::server::subsonic::SubsonicClient::new(
                                                        &url, &username, &password,
                                                    );
                                                match client.scrobble(&scrobble_id).await {
                                                    Ok(_) => tracing::info!(
                                                        "Subsonic scrobbled: {} - {}",
                                                        scrobble_track.artist,
                                                        scrobble_track.title
                                                    ),
                                                    Err(e) => tracing::warn!(
                                                        "Subsonic scrobble failed: {}",
                                                        e
                                                    ),
                                                }
                                            }
                                        }

                                        // Last.fm scrobble
                                        if has_lastfm {
                                            let scrobble = scrobble::lastfm::make_scrobble(
                                                &scrobble_track.artist,
                                                &scrobble_track.title,
                                                Some(&scrobble_track.album),
                                            );
                                            match scrobble::lastfm::submit_scrobble(
                                                &lastfm_api_key,
                                                &lastfm_api_secret,
                                                &lastfm_session_key,
                                                &scrobble,
                                            )
                                            .await
                                            {
                                                Ok(_) => tracing::info!(
                                                    "Last.fm scrobbled: {} - {}",
                                                    scrobble_track.artist,
                                                    scrobble_track.title
                                                ),
                                                Err(e) => {
                                                    tracing::warn!("Last.fm scrobble failed: {}", e)
                                                }
                                            }
                                        }

                                        // MusicBrainz single listen
                                        let token_raw =
                                            scrobble_cfg.read().musicbrainz_token.clone();
                                        if !token_raw.is_empty() {
                                            let auth = if token_raw.contains(' ') {
                                                token_raw
                                            } else {
                                                format!("Token {}", token_raw)
                                            };
                                            let listen = scrobble::musicbrainz::make_listen(
                                                &scrobble_track.artist,
                                                &scrobble_track.title,
                                                Some(&scrobble_track.album),
                                            );
                                            match scrobble::musicbrainz::submit_listens(
                                                &auth,
                                                vec![listen],
                                                "single",
                                            )
                                            .await
                                            {
                                                Ok(_) => tracing::info!(
                                                    "MusicBrainz scrobbled: {} - {}",
                                                    scrobble_track.artist,
                                                    scrobble_track.title
                                                ),
                                                Err(e) => {
                                                    tracing::warn!(
                                                        "MusicBrainz scrobble failed: {}",
                                                        e
                                                    )
                                                }
                                            }
                                        }
                                    });

                                    let cover_url = cover_url.clone();
                                    let track = track.clone();
                                    let mut player = player;
                                    let play_generation = play_generation;

                                    spawn(async move {
                                        if let Ok(response) = reqwest::get(&cover_url).await {
                                            if let Ok(bytes) = response.bytes().await {
                                                let temp_dir = std::env::temp_dir();
                                                let random_id: u64 = rand::random();
                                                let file_path = temp_dir
                                                    .join(format!("kopuz_cover_{}.jpg", random_id));

                                                if tokio::fs::write(&file_path, bytes).await.is_ok()
                                                {
                                                    if *play_generation.read() == current_gen {
                                                        let path_str =
                                                            file_path.to_string_lossy().to_string();
                                                        let new_meta = NowPlayingMeta {
                                                            title: track.title,
                                                            artist: track.artist,
                                                            album: track.album,
                                                            duration:
                                                                std::time::Duration::from_secs(
                                                                    track.duration,
                                                                ),
                                                            artwork: Some(path_str),
                                                        };
                                                        player.write().update_metadata(new_meta);
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                        } else {
                            is_loading.set(false);
                            skip_in_progress.set(false);
                        }
                    });

                    #[cfg(target_arch = "wasm32")]
                    spawn(async move {
                        if *play_generation.read() == current_gen {
                            let meta = NowPlayingMeta {
                                title: track.title.clone(),
                                artist: track.artist.clone(),
                                album: track.album.clone(),
                                duration: std::time::Duration::from_secs(track.duration),
                                artwork: Some(cover_url.clone()),
                            };

                            let started = {
                                let mut player = player.write();
                                player.play_url(stream_url, meta);
                                player.set_volume(*volume.peek());
                                if let Some(seek_secs) = restore_seek_secs {
                                    if seek_secs > 0 && player.can_resume() {
                                        player.seek(Duration::from_secs(seek_secs));
                                        current_song_progress.set(seek_secs);
                                    }
                                }
                                player.can_resume()
                            };
                            if started && clear_pending_resume_on_success {
                                pending_resume.set(None);
                            }
                            is_loading.set(false);
                            is_playing.set(started);
                            skip_in_progress.set(false);

                            if started {
                                if !is_radio_item {
                                    let scrobble_track = track.clone();
                                    let scrobble_gen = current_gen;
                                    let scrobble_play_gen = play_generation;
                                    let scrobble_cfg = cfg_signal;
                                    let scrobble_id = id.clone();
                                    let duration_secs = scrobble_track.duration;
                                    let threshold_secs =
                                        std::cmp::min(240, (duration_secs / 2) as u64);

                                    spawn(async move {
                                        if duration_secs < 30 {
                                            return;
                                        }

                                        {
                                            let subsonic_creds = {
                                                let conf = scrobble_cfg.read();
                                                conf.server.as_ref().and_then(|s| {
                                                    if matches!(
                                                        s.service,
                                                        MusicService::Subsonic
                                                            | MusicService::Custom
                                                    ) {
                                                        if let (Some(pw), Some(un)) =
                                                            (&s.access_token, &s.user_id)
                                                        {
                                                            Some((
                                                                s.url.clone(),
                                                                un.clone(),
                                                                pw.clone(),
                                                            ))
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                })
                                            };
                                            if let Some((url, username, password)) = subsonic_creds
                                            {
                                                let client =
                                                    ::server::subsonic::SubsonicClient::new(
                                                        &url, &username, &password,
                                                    );
                                                if let Err(e) =
                                                    client.scrobble_now_playing(&scrobble_id).await
                                                {
                                                    tracing::warn!(
                                                        "Subsonic now-playing failed: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }

                                        let lastfm_api_key =
                                            scrobble_cfg.read().lastfm_api_key.clone();
                                        let lastfm_api_secret =
                                            scrobble_cfg.read().lastfm_api_secret.clone();
                                        let lastfm_session_key =
                                            scrobble_cfg.read().lastfm_session_key.clone();
                                        let has_lastfm = !lastfm_api_key.is_empty()
                                            && !lastfm_api_secret.is_empty();

                                        if has_lastfm {
                                            let playing_now = scrobble::lastfm::make_playing_now(
                                                &scrobble_track.artist,
                                                &scrobble_track.title,
                                                Some(&scrobble_track.album),
                                            );
                                            if let Err(e) = scrobble::lastfm::submit_now_playing(
                                                &lastfm_api_key,
                                                &lastfm_api_secret,
                                                &lastfm_session_key,
                                                &playing_now,
                                            )
                                            .await
                                            {
                                                tracing::warn!("Last.fm now playing failed: {}", e);
                                            }
                                        }

                                        let token_raw =
                                            scrobble_cfg.read().musicbrainz_token.clone();
                                        if !token_raw.is_empty() {
                                            let auth = if token_raw.contains(' ') {
                                                token_raw.clone()
                                            } else {
                                                format!("Token {}", token_raw)
                                            };
                                            let playing_now =
                                                scrobble::musicbrainz::make_playing_now(
                                                    &scrobble_track.artist,
                                                    &scrobble_track.title,
                                                    Some(&scrobble_track.album),
                                                );
                                            if let Err(e) = scrobble::musicbrainz::submit_listens(
                                                &auth,
                                                vec![playing_now],
                                                "playing_now",
                                            )
                                            .await
                                            {
                                                tracing::warn!(
                                                    "MusicBrainz playing_now failed: {}",
                                                    e
                                                );
                                            }
                                        }

                                        utils::sleep(std::time::Duration::from_secs(
                                            threshold_secs,
                                        ))
                                        .await;

                                        if *scrobble_play_gen.read() != scrobble_gen {
                                            return;
                                        }

                                        {
                                            let subsonic_creds = {
                                                let conf = scrobble_cfg.read();
                                                conf.server.as_ref().and_then(|s| {
                                                    if matches!(
                                                        s.service,
                                                        MusicService::Subsonic
                                                            | MusicService::Custom
                                                    ) {
                                                        if let (Some(pw), Some(un)) =
                                                            (&s.access_token, &s.user_id)
                                                        {
                                                            Some((
                                                                s.url.clone(),
                                                                un.clone(),
                                                                pw.clone(),
                                                            ))
                                                        } else {
                                                            None
                                                        }
                                                    } else {
                                                        None
                                                    }
                                                })
                                            };
                                            if let Some((url, username, password)) = subsonic_creds
                                            {
                                                let client =
                                                    ::server::subsonic::SubsonicClient::new(
                                                        &url, &username, &password,
                                                    );
                                                match client.scrobble(&scrobble_id).await {
                                                    Ok(_) => tracing::info!(
                                                        "Subsonic scrobbled: {} - {}",
                                                        scrobble_track.artist,
                                                        scrobble_track.title
                                                    ),
                                                    Err(e) => tracing::warn!(
                                                        "Subsonic scrobble failed: {}",
                                                        e
                                                    ),
                                                }
                                            }
                                        }

                                        if has_lastfm {
                                            let scrobble = scrobble::lastfm::make_scrobble(
                                                &scrobble_track.artist,
                                                &scrobble_track.title,
                                                Some(&scrobble_track.album),
                                            );
                                            match scrobble::lastfm::submit_scrobble(
                                                &lastfm_api_key,
                                                &lastfm_api_secret,
                                                &lastfm_session_key,
                                                &scrobble,
                                            )
                                            .await
                                            {
                                                Ok(_) => tracing::info!(
                                                    "Last.fm scrobbled: {} - {}",
                                                    scrobble_track.artist,
                                                    scrobble_track.title
                                                ),
                                                Err(e) => {
                                                    tracing::warn!("Last.fm scrobble failed: {}", e)
                                                }
                                            }
                                        }

                                        let token_raw =
                                            scrobble_cfg.read().musicbrainz_token.clone();
                                        if !token_raw.is_empty() {
                                            let auth = if token_raw.contains(' ') {
                                                token_raw
                                            } else {
                                                format!("Token {}", token_raw)
                                            };
                                            let listen = scrobble::musicbrainz::make_listen(
                                                &scrobble_track.artist,
                                                &scrobble_track.title,
                                                Some(&scrobble_track.album),
                                            );
                                            match scrobble::musicbrainz::submit_listens(
                                                &auth,
                                                vec![listen],
                                                "single",
                                            )
                                            .await
                                            {
                                                Ok(_) => tracing::info!(
                                                    "MusicBrainz scrobbled: {} - {}",
                                                    scrobble_track.artist,
                                                    scrobble_track.title
                                                ),
                                                Err(e) => {
                                                    tracing::warn!(
                                                        "MusicBrainz scrobble failed: {}",
                                                        e
                                                    )
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                        } else {
                            is_loading.set(false);
                            skip_in_progress.set(false);
                        }
                    });
                }
            } else {
                #[cfg(not(target_arch = "wasm32"))]
                if !use_crossfade {
                    self.current_queue_index.set(idx);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = idx;
                    return;
                } // local files not supported on web
                #[cfg(not(target_arch = "wasm32"))]
                if let Ok((source, hint)) = decoder::open_file(&track.path) {
                    {
                        let artwork = {
                            let lib = self.library.peek();
                            lib.albums
                                .iter()
                                .find(|a| a.id == track.album_id)
                                .and_then(|a| {
                                    a.cover_path
                                        .as_ref()
                                        .map(|p| p.to_string_lossy().into_owned())
                                })
                        };

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
                            eprintln!("Playback error: {e}");
                            self.skip_in_progress.set(false);
                            return;
                        }
                        self.player.write().set_volume(*self.volume.peek());

                        self.skip_in_progress.set(false);

                        if !use_crossfade {
                            self.hydrate_current_track_metadata(idx, 0);
                        } else {
                            self.schedule_pending_crossfade_ui(
                                idx,
                                outgoing_duration_secs,
                                outgoing_progress_secs,
                            );
                        }

                        self.is_playing.set(true);

                        if let Some(seek_secs) = restore_seek_secs {
                            if seek_secs > 0 {
                                self.apply_restore_seek(seek_secs);
                            }
                        }
                        if clear_pending_resume_on_success {
                            self.clear_pending_resume();
                        }
                        if !is_radio_item {
                            let cfg_signal = self.config;
                            let play_generation_signal = self.play_generation;
                            let gen_snapshot = current_gen;
                            let scrobble_track = track.clone();

                            let duration_secs = scrobble_track.duration;
                            let threshold_secs = std::cmp::min(240, (duration_secs / 2) as u64);

                            spawn(async move {
                                // track must be longer than 30 seconds
                                if duration_secs < 30 {
                                    return;
                                }

                                // Last.fm now-playing
                                let lastfm_api_key = cfg_signal.read().lastfm_api_key.clone();
                                let lastfm_api_secret = cfg_signal.read().lastfm_api_secret.clone();
                                let lastfm_session_key =
                                    cfg_signal.read().lastfm_session_key.clone();
                                let has_lastfm =
                                    !lastfm_api_key.is_empty() && !lastfm_api_secret.is_empty();

                                if has_lastfm {
                                    let playing_now = scrobble::lastfm::make_playing_now(
                                        &scrobble_track.artist,
                                        &scrobble_track.title,
                                        Some(&scrobble_track.album),
                                    );
                                    if let Err(e) = scrobble::lastfm::submit_now_playing(
                                        &lastfm_api_key,
                                        &lastfm_api_secret,
                                        &lastfm_session_key,
                                        &playing_now,
                                    )
                                    .await
                                    {
                                        tracing::warn!("Last.fm now playing failed: {}", e);
                                    }
                                }

                                // MusicBrainz playing_now
                                let token_raw = cfg_signal.read().musicbrainz_token.clone();
                                if !token_raw.is_empty() {
                                    let auth = if token_raw.contains(' ') {
                                        token_raw.clone()
                                    } else {
                                        format!("Token {}", token_raw)
                                    };
                                    let playing_now = scrobble::musicbrainz::make_playing_now(
                                        &scrobble_track.artist,
                                        &scrobble_track.title,
                                        Some(&scrobble_track.album),
                                    );
                                    if let Err(e) = scrobble::musicbrainz::submit_listens(
                                        &auth,
                                        vec![playing_now],
                                        "playing_now",
                                    )
                                    .await
                                    {
                                        tracing::warn!("MusicBrainz playing_now failed: {}", e);
                                    }
                                }

                                tokio::time::sleep(std::time::Duration::from_secs(threshold_secs))
                                    .await;

                                if *play_generation_signal.read() != gen_snapshot {
                                    return;
                                }

                                if has_lastfm {
                                    let scrobble = scrobble::lastfm::make_scrobble(
                                        &scrobble_track.artist,
                                        &scrobble_track.title,
                                        Some(&scrobble_track.album),
                                    );
                                    match scrobble::lastfm::submit_scrobble(
                                        &lastfm_api_key,
                                        &lastfm_api_secret,
                                        &lastfm_session_key,
                                        &scrobble,
                                    )
                                    .await
                                    {
                                        Ok(_) => tracing::info!(
                                            "Last.fm scrobbled: {} - {}",
                                            scrobble_track.artist,
                                            scrobble_track.title
                                        ),
                                        Err(e) => {
                                            tracing::warn!("Last.fm scrobble failed: {}", e)
                                        }
                                    }
                                }

                                let token_raw = cfg_signal.read().musicbrainz_token.clone();
                                if !token_raw.is_empty() {
                                    let auth = if token_raw.contains(' ') {
                                        token_raw
                                    } else {
                                        format!("Token {}", token_raw)
                                    };
                                    let listen = scrobble::musicbrainz::make_listen(
                                        &scrobble_track.artist,
                                        &scrobble_track.title,
                                        Some(&scrobble_track.album),
                                    );
                                    match scrobble::musicbrainz::submit_listens(
                                        &auth,
                                        vec![listen],
                                        "single",
                                    )
                                    .await
                                    {
                                        Ok(_) => tracing::info!(
                                            "MusicBrainz scrobbled: {} - {}",
                                            scrobble_track.artist,
                                            scrobble_track.title
                                        ),
                                        Err(e) => {
                                            tracing::warn!("MusicBrainz scrobble failed: {}", e)
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }
    }

    pub fn play_next(&mut self) {
        self.play_next_with_transition(false);
    }

    pub fn play_next_with_crossfade(&mut self) {
        self.play_next_with_transition(true);
    }

    fn play_next_with_transition(&mut self, allow_crossfade: bool) {
        if *self.is_loading.peek() {
            self.skip_in_progress.set(false);
        }

        let idx = *self.current_queue_index.peek();
        let queue_len = self.queue.peek().len();

        if queue_len == 0 {
            self.skip_in_progress.set(false);
            return;
        }

        let loop_mode = *self.loop_mode.peek();

        match loop_mode {
            LoopMode::Track => {
                self.play_track_with_history(idx, allow_crossfade);
            }
            _ => {
                if idx + 1 >= queue_len && loop_mode == LoopMode::None {
                    self.skip_in_progress.set(false);
                    self.player.write().pause();
                    self.is_playing.set(false);
                    return;
                }
                let next_idx = if idx + 1 < queue_len { idx + 1 } else { 0 };
                self.play_track_with_history(next_idx, allow_crossfade);
            }
        }
    }

    fn play_track_with_history(&mut self, track_idx: usize, allow_crossfade: bool) {
        let current_idx = *self.current_queue_index.peek();
        self.history.with_mut(|h| {
            if h.last() != Some(&current_idx) {
                h.push(current_idx);
            }
        });
        self.play_track_no_history_with_transition(track_idx, allow_crossfade);
    }

    pub fn play_prev(&mut self) {
        let progress = *self.current_song_progress.peek();
        let back_behavior = self.config.peek().back_behavior;

        if back_behavior == BackBehavior::RewindThenPrev && progress > 3 {
            self.player.write().seek(std::time::Duration::ZERO);
            self.current_song_progress.set(0);
            return;
        }

        let idx = *self.current_queue_index.peek();
        let queue_len = self.queue.peek().len();

        if queue_len == 0 {
            return;
        }

        if let Some(prev_idx) = self.history.with_mut(|h| h.pop()) {
            self.play_track_no_history_without_crossfade(prev_idx);
            return;
        }

        if idx > 0 {
            self.play_track_no_history_without_crossfade(idx - 1);
        } else if *self.loop_mode.peek() == LoopMode::Queue {
            self.play_track_no_history_without_crossfade(queue_len - 1);
        }
    }

    fn rebuild_shuffle_order(&mut self) {
        use rand::seq::SliceRandom;
        let queue_len = self.queue.peek().len();
        let current_idx = *self.current_queue_index.peek();

        if queue_len == 0 {
            self.shuffle_order.set(Vec::new());
            self.current_queue_index.set(0);
            return;
        }

        // Tracks that come after the current position (play these first).
        let mut ahead: Vec<usize> = (current_idx..queue_len).collect();
        ahead.shuffle(&mut rand::thread_rng());
        // move current played track to the front
        let pos = ahead
            .iter()
            .position(|&i| i == current_idx)
            .expect("cannot find current index in shuffle order");
        ahead.swap(pos, 0);

        // Tracks that wrap around from the beginning (play after the ahead group).
        let mut wrapped: Vec<usize> = (0..current_idx).collect();
        wrapped.shuffle(&mut rand::thread_rng());

        ahead.extend(wrapped);
        // reset current queue index to match the currently played track (now moved at pos 0)
        // will be used as a pointer to the retrieve the current track in the shuffled order
        self.current_queue_index.set(0);
        self.shuffle_order.set(ahead);
    }

    pub fn play_queue_shuffled(&mut self, tracks: Vec<Track>) {
        use rand::Rng;
        let queue_len = tracks.len();
        if queue_len == 0 {
            return;
        }

        self.queue.set(tracks);
        let start = rand::thread_rng().gen_range(0..queue_len);
        self.play_track(start);
    }

    pub fn play_queue_linear(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        self.queue.set(tracks);
        self.play_track(0);
        self.play_track_no_history(0);
    }

    pub fn add_to_queue(&mut self, tracks: impl IntoIterator<Item = Track>) {
        let tracks: Vec<Track> = tracks.into_iter().collect();
        let count = tracks.len();
        if count == 0 {
            return;
        }

        self.queue.with_mut(|q| q.extend(tracks));

        if *self.shuffle.peek() {
            let q_len = self.queue.peek().len();
            let start_idx = q_len - count;
            self.shuffle_order.with_mut(|so| {
                (start_idx..q_len).for_each(|idx| so.push(idx));
            });
        }
    }

    pub fn toggle_shuffle(&mut self) {
        let now_on = !*self.shuffle.peek();
        self.shuffle.set(now_on);
        if now_on {
            self.rebuild_shuffle_order();
        } else {
            // reset current queue index to match track index when turning off shuffle mode
            let current_idx = *self.current_queue_index.peek();
            self.current_queue_index.set(
                *self
                    .shuffle_order
                    .peek()
                    .get(current_idx)
                    .unwrap_or(&current_idx),
            );
        }
    }

    pub fn set_loop_mode(&mut self, mode: LoopMode) {
        self.loop_mode.set(mode);
    }
    fn cancel_radio_task(&mut self) {
        if let Some(task) = self.radio_task.take() {
            task.cancel();
        }
    }
    pub fn play_radio(&mut self, station_id: &str, stream_id: &str) {
        let path = format!("radio:{}:{}", station_id, stream_id);
        let track = Track {
            path: std::path::PathBuf::from(path),
            album_id: "".to_string(),
            title: stream_id.to_string(),
            artist: station_id.to_string(),
            album: "Live Radio".to_string(),
            duration: u64::MAX,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            playlist_item_id: None,
            artists: vec![],
        };

        let mut q = self.queue.write();
        q.clear();
        q.push(track);
        drop(q);

        self.shuffle_order.set(vec![0]);
        self.current_queue_index.set(0);
        self.history.write().clear();
        self.play_track_no_history_with_transition(0, false);
    }

    pub fn toggle_loop(&mut self) {
        self.loop_mode.with_mut(|l| *l = l.next());
    }

    pub fn pause(&mut self) {
        let idx = *self.current_queue_index.peek();
        let is_radio = self
            .current_track(idx)
            .map_or(false, |t| t.path.to_string_lossy().starts_with("radio:"));

        if is_radio {
            self.player.write().stop_for_transition();
        } else {
            self.player.write().pause();
        }
        self.is_playing.set(false);
    }

    pub fn resume(&mut self) {
        let idx = *self.current_queue_index.peek();
        let is_radio = self
            .current_track(idx)
            .map_or(false, |t| t.path.to_string_lossy().starts_with("radio:"));

        if is_radio || !self.player.peek().can_resume() {
            if let Some(track) = self.current_track(idx) {
                if !is_radio {
                    let progress_secs = (*self.current_song_progress.peek()).min(track.duration);
                    self.set_pending_resume_for_track(&track, progress_secs);
                }
                self.play_track_no_history(idx);
            }
            return;
        }

        self.player.write().play_resume();
        self.is_playing.set(true);
    }

    pub fn toggle(&mut self) {
        if *self.is_playing.peek() {
            self.pause();
        } else {
            self.resume();
        }
    }

    pub fn move_queue_item(&mut self, from: usize, to: usize) {
        self.move_physical_queue_item(from, to);
    }

    fn move_physical_queue_item(&mut self, from: usize, to: usize) {
        let len = self.queue.peek().len();
        if from >= len || to >= len || from == to {
            return;
        }

        if *self.shuffle.peek() {
            self.shuffle_order.with_mut(|so| so.swap(from, to));
            return;
        }

        self.queue.with_mut(|queue| {
            let track = queue.remove(from);
            queue.insert(to, track);
        });

        let current_idx = *self.current_queue_index.peek();
        self.current_queue_index
            .set(Self::remap_queue_index(current_idx, from, to));

        self.history.with_mut(|history| {
            for idx in history.iter_mut() {
                *idx = Self::remap_queue_index(*idx, from, to);
            }
        });
    }

    pub fn restore_queue_state(
        &mut self,
        queue: Vec<Track>,
        current_queue_index: usize,
        progress_secs: u64,
        shuffle_order: Vec<usize>,
        shuffle_enabled: bool,
    ) {
        self.clear_pending_crossfade_ui();
        self.player.write().stop();
        self.is_playing.set(false);
        self.is_loading.set(false);
        self.skip_in_progress.set(false);
        self.history.set(Vec::new());
        self.queue.set(queue);
        self.shuffle.set(shuffle_enabled);
        self.shuffle_order.set(shuffle_order);

        let queue_len = self.queue.peek().len();
        if queue_len == 0 {
            self.current_queue_index.set(0);
            self.pending_resume.set(None);
            self.clear_current_track_metadata();
            return;
        }

        let idx = current_queue_index.min(queue_len - 1);
        let track = self
            .current_track(idx)
            .expect("queue index should be valid");
        let progress_secs = progress_secs.min(track.duration);

        self.hydrate_current_track_metadata(idx, progress_secs);
        self.set_pending_resume_for_track(&track, progress_secs);
    }
}

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
    library: Signal<Library>,
    config: Signal<AppConfig>,
) -> PlayerController {
    let play_generation = use_signal(|| 0);
    let is_loading = use_signal(|| false);
    let skip_in_progress = use_signal(|| false);
    let history = use_signal(|| Vec::new());
    let shuffle = use_signal(|| false);
    let shuffle_order = use_signal(|| Vec::<usize>::new());
    let loop_mode = use_signal(|| LoopMode::None);
    let pending_resume = use_signal(|| None::<PendingResumeState>);
    let pending_crossfade_ui = use_signal(|| None::<PendingCrossfadeUiState>);
    let radio_task = use_signal(|| None::<dioxus_core::Task>);

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
        library,
        config,
        play_generation,
        pending_resume,
        pending_crossfade_ui,
        radio_task,
    }
}
