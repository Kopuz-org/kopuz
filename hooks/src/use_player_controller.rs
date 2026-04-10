use config::AppConfig;
use config::MusicService;
use dioxus::{logger::tracing, prelude::*};
use player::player::{NowPlayingMeta, Player};
use reader::{Library, Track};
use scrobble;
use utils;

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
    pub loop_mode: Signal<LoopMode>,
    pub current_queue_index: Signal<usize>,
    pub current_song_title: Signal<String>,
    pub current_song_artist: Signal<String>,
    pub current_song_album: Signal<String>,
    pub current_song_khz: Signal<u32>,
    pub current_song_bitrate: Signal<u8>,
    pub current_song_duration: Signal<u64>,
    pub current_song_progress: Signal<u64>,
    pub current_song_cover_url: Signal<String>,
    pub volume: Signal<f32>,
    pub library: Signal<Library>,
    pub config: Signal<AppConfig>,
    pub play_generation: Signal<usize>,
}

impl PlayerController {
    pub fn play_track(&mut self, idx: usize) {
        let current_idx = *self.current_queue_index.peek();
        self.history.with_mut(|h| {
            if h.last() != Some(&current_idx) {
                h.push(current_idx);
            }
        });
        self.play_track_no_history(idx);
    }

    pub fn play_track_no_history(&mut self, idx: usize) {
        self.play_generation.with_mut(|g| *g += 1);
        let current_gen = *self.play_generation.peek();

        let q = self.queue.peek();
        if idx < q.len() {
            let track = q[idx].clone();
            let path_str = track.path.to_string_lossy().to_string();
            let scheme = path_str
                .split(':')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_server_item = matches!(scheme.as_str(), "jellyfin" | "subsonic" | "custom");

            if is_server_item {
                let parts: Vec<&str> = path_str.split(':').collect();
                let id = parts.get(1).unwrap_or(&"").to_string();

                let conf = self.config.read();
                if let Some(server) = &conf.server {
                    let (stream_url, cover_url) = match server.service {
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
                            let (stream_url, cover_url) = if let (Some(password), Some(username)) =
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
                    };

                    if stream_url.is_empty() {
                        self.is_loading.set(false);
                        self.skip_in_progress.set(false);
                        return;
                    }

                    self.player.write().stop();
                    self.is_playing.set(false);

                    let mut player = self.player;
                    let mut is_playing = self.is_playing;
                    let mut is_loading = self.is_loading;
                    let mut skip_in_progress = self.skip_in_progress;
                    let play_generation = self.play_generation;
                    let volume = self.volume;
                    let cfg_signal = self.config;

                    self.current_song_title.set(track.title.clone());
                    self.current_song_artist.set(track.artist.clone());
                    self.current_song_album.set(track.album.clone());
                    self.current_song_duration.set(track.duration);
                    self.current_song_progress.set(0);
                    self.current_song_cover_url.set(cover_url.clone());
                    self.current_queue_index.set(idx);

                    self.is_loading.set(true);

                    spawn(async move {
                        let stream = utils::stream_buffer::StreamBuffer::new(stream_url);
                        let source_res = tokio::task::spawn_blocking(move || {
                            let (source, hint) = player::decoder::from_stream(stream);
                            Ok::<_, std::io::Error>((source, hint))
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

                                player.write().play(source, meta, hint);
                                player.write().set_volume(*volume.peek());
                                is_loading.set(false);
                                is_playing.set(true);
                                skip_in_progress.set(false);

                                let scrobble_track = track.clone();
                                let scrobble_gen = current_gen;
                                let scrobble_play_gen = play_generation;
                                let scrobble_cfg = cfg_signal;
                                let duration_secs = scrobble_track.duration;
                                let threshold_secs = std::cmp::min(240, (duration_secs / 2) as u64);

                                spawn(async move {
                                    let token_raw = scrobble_cfg.read().musicbrainz_token.clone();
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
                                            tracing::warn!(
                                                "Jellyfin: failed to submit playing_now: {}",
                                                e
                                            );
                                        }
                                    }

                                    tokio::time::sleep(std::time::Duration::from_secs(
                                        threshold_secs,
                                    ))
                                    .await;

                                    if *scrobble_play_gen.read() != scrobble_gen {
                                        return;
                                    }

                                    let token_raw = scrobble_cfg.read().musicbrainz_token.clone();
                                    if token_raw.is_empty() {
                                        return;
                                    }

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
                                            "Jellyfin scrobbled: {} - {}",
                                            scrobble_track.artist,
                                            scrobble_track.title
                                        ),
                                        Err(e) => tracing::warn!("Jellyfin scrobble failed: {}", e),
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
                                                .join(format!("rusic_cover_{}.jpg", random_id));

                                            if tokio::fs::write(&file_path, bytes).await.is_ok() {
                                                if *play_generation.read() == current_gen {
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
                                    }
                                });
                            }
                        } else {
                            is_loading.set(false);
                            skip_in_progress.set(false);
                        }
                    });
                }
            } else {
                self.current_queue_index.set(idx);
                if let Ok((source, hint)) = player::decoder::open_file(&track.path) {
                    {
                        let lib = self.library.peek();
                        let album = lib.albums.iter().find(|a| a.id == track.album_id);
                        let artwork = album.and_then(|a| {
                            a.cover_path
                                .as_ref()
                                .map(|p| p.to_string_lossy().into_owned())
                        });

                        let meta = NowPlayingMeta {
                            title: track.title.clone(),
                            artist: track.artist.clone(),
                            album: track.album.clone(),
                            duration: std::time::Duration::from_secs(track.duration),
                            artwork,
                        };

                        self.player.write().play(source, meta, hint);
                        self.player.write().set_volume(*self.volume.peek());

                        self.skip_in_progress.set(false);

                        self.current_song_title.set(track.title.clone());
                        self.current_song_artist.set(track.artist.clone());
                        self.current_song_album.set(track.album.clone());
                        self.current_song_khz.set(track.khz);
                        self.current_song_bitrate.set(track.bitrate);
                        self.current_song_duration.set(track.duration);
                        self.current_song_progress.set(0);

                        if let Some(album) = album {
                            if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref())
                            {
                                self.current_song_cover_url.set(url);
                            } else {
                                self.current_song_cover_url.set(String::new());
                            }
                        } else {
                            self.current_song_cover_url.set(String::new());
                        }

                        self.is_playing.set(true);

                        let cfg_signal = self.config;
                        let play_generation_signal = self.play_generation;
                        let gen_snapshot = current_gen;
                        let scrobble_track = track.clone();

                        let duration_secs = scrobble_track.duration;
                        let threshold_secs = std::cmp::min(240, (duration_secs / 2) as u64);

                        spawn(async move {
                            let token_raw = cfg_signal.read().musicbrainz_token.clone();
                            if !token_raw.is_empty() {
                                let auth_header_value = if token_raw.contains(' ') {
                                    token_raw
                                } else {
                                    format!("Token {}", token_raw)
                                };

                                let playing_now = scrobble::musicbrainz::make_playing_now(
                                    &scrobble_track.artist,
                                    &scrobble_track.title,
                                    Some(&scrobble_track.album),
                                );

                                if let Err(e) = scrobble::musicbrainz::submit_listens(
                                    &auth_header_value,
                                    vec![playing_now],
                                    "playing_now",
                                )
                                .await
                                {
                                    tracing::warn!("Failed to submit playing_now: {}", e);
                                }
                            }

                            tokio::time::sleep(std::time::Duration::from_secs(threshold_secs))
                                .await;
                            if *play_generation_signal.read() != gen_snapshot {
                                return;
                            }

                            let token_raw = cfg_signal.read().musicbrainz_token.clone();
                            if token_raw.is_empty() {
                                return;
                            }

                            let auth_header_value = if token_raw.contains(' ') {
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
                                &auth_header_value,
                                vec![listen],
                                "single",
                            )
                            .await
                            {
                                Ok(_) => tracing::info!(
                                    "Scrobbled: {} - {}",
                                    scrobble_track.artist,
                                    scrobble_track.title
                                ),
                                Err(e) => tracing::warn!("Scrobble failed: {}", e),
                            }
                        });
                    }
                }
            }
        }
    }

    pub fn play_next(&mut self) {
        let idx = *self.current_queue_index.peek();
        let queue_len = self.queue.peek().len();

        if queue_len == 0 {
            return;
        }

        let loop_mode = *self.loop_mode.peek();
        let shuffle = *self.shuffle.peek();

        match loop_mode {
            LoopMode::Track => {
                self.play_track(idx);
            }
            _ => {
                if shuffle && queue_len > 1 {
                    let mut rng = rand::thread_rng();
                    use rand::Rng;
                    let mut next_idx = rng.gen_range(0..queue_len);
                    while next_idx == idx {
                        next_idx = rng.gen_range(0..queue_len);
                    }
                    self.play_track(next_idx);
                } else if shuffle && queue_len == 1 {
                    self.play_track(0);
                } else if idx + 1 < queue_len {
                    self.play_track(idx + 1);
                } else if loop_mode == LoopMode::Queue {
                    self.play_track(0);
                } else {
                    self.is_playing.set(false);
                }
            }
        }
    }

    pub fn play_prev(&mut self) {
        let idx = *self.current_queue_index.peek();
        let queue_len = self.queue.peek().len();

        if queue_len == 0 {
            return;
        }

        if let Some(prev_idx) = self.history.with_mut(|h| h.pop()) {
            self.play_track_no_history(prev_idx);
            return;
        }

        if idx > 0 {
            self.play_track_no_history(idx - 1);
        } else if *self.loop_mode.peek() == LoopMode::Queue {
            self.play_track_no_history(queue_len - 1);
        }
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle.with_mut(|s| *s = !*s);
    }

    pub fn toggle_loop(&mut self) {
        self.loop_mode.with_mut(|l| *l = l.next());
    }

    pub fn pause(&mut self) {
        self.player.write().pause();
        self.is_playing.set(false);
    }

    pub fn resume(&mut self) {
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
    current_song_bitrate: Signal<u8>,
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
    current_song_cover_url: Signal<String>,
    volume: Signal<f32>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
) -> PlayerController {
    let play_generation = use_signal(|| 0);
    let is_loading = use_signal(|| false);
    let skip_in_progress = use_signal(|| false);
    let history = use_signal(|| Vec::new());
    let shuffle = use_signal(|| false);
    let loop_mode = use_signal(|| LoopMode::None);

    PlayerController {
        player,
        is_playing,
        is_loading,
        skip_in_progress,
        history,
        queue,
        shuffle,
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
        volume,
        library,
        config,
        play_generation,
    }
}
