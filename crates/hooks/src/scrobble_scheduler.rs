use config::AppConfig;
use dioxus::logger::tracing::Instrument;
use dioxus::prelude::*;
use reader::Track;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct ScrobbleOptions {
    pub include_librefm: bool,
    pub include_musicbrainz_ids: bool,
}

impl ScrobbleOptions {
    pub const REMOTE_NATIVE: Self = Self {
        include_librefm: true,
        include_musicbrainz_ids: false,
    };

    pub const REMOTE_WEB: Self = Self {
        include_librefm: false,
        include_musicbrainz_ids: false,
    };

    pub const LOCAL: Self = Self {
        include_librefm: false,
        include_musicbrainz_ids: true,
    };
}

pub fn schedule(
    track: Track,
    item_id: Option<String>,
    config: Signal<AppConfig>,
    play_generation: Signal<usize>,
    generation: usize,
    active_source: Option<Signal<::server::source::ActiveSource>>,
    options: ScrobbleOptions,
) {
    let duration_secs = track.duration;
    let threshold_secs = std::cmp::min(240, duration_secs / 2);
    let span = tracing::info_span!(
        "scrobble.submit",
        track = item_id.as_deref().unwrap_or(track.id.uid().as_str())
    );

    spawn(
        async move {
            if duration_secs < 30 {
                return;
            }

            let mbids = musicbrainz_ids(&track, options.include_musicbrainz_ids);

            if let (Some(source), Some(id)) = (active_source, item_id.as_deref()) {
                let source = source.peek().clone();
                if let Err(error) = source.scrobble_now_playing(id).await {
                    tracing::warn!("now-playing scrobble failed: {}", error);
                }
            }

            let lastfm_api_key = config.read().lastfm_api_key.clone();
            let lastfm_api_secret = config.read().lastfm_api_secret.clone();
            let lastfm_session_key = config.read().lastfm_session_key.clone();
            let has_lastfm = !lastfm_api_key.is_empty() && !lastfm_api_secret.is_empty();

            if has_lastfm {
                let playing_now = scrobble::lastfm::make_playing_now(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                );
                if let Err(error) = scrobble::lastfm::submit_now_playing(
                    &lastfm_api_key,
                    &lastfm_api_secret,
                    &lastfm_session_key,
                    &playing_now,
                )
                .await
                {
                    tracing::warn!("Last.fm now playing failed: {}", error);
                }
            }

            let librefm_session_key = config.read().librefm_session_key.clone();
            let has_librefm = options.include_librefm && !librefm_session_key.is_empty();

            if has_librefm {
                let playing_now = scrobble::librefm::make_playing_now(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                );
                if let Err(error) = scrobble::librefm::submit_now_playing(
                    scrobble::librefm::API_KEY,
                    scrobble::librefm::API_SECRET,
                    &librefm_session_key,
                    &playing_now,
                )
                .await
                {
                    tracing::warn!("Libre.fm now playing failed: {}", error);
                }
            }

            let token_raw = config.read().musicbrainz_token.clone();
            if !token_raw.is_empty() {
                let auth = musicbrainz_auth(&token_raw);
                let playing_now = scrobble::musicbrainz::make_playing_now(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                    mbids.clone(),
                );
                if let Err(error) =
                    scrobble::musicbrainz::submit_listens(&auth, vec![playing_now], "playing_now")
                        .await
                {
                    tracing::warn!("MusicBrainz playing_now failed: {}", error);
                }
            }

            sleep_threshold(Duration::from_secs(threshold_secs)).await;

            if *play_generation.read() != generation {
                return;
            }

            if let (Some(source), Some(id)) = (active_source, item_id.as_deref()) {
                let source = source.peek().clone();
                match source.scrobble(id).await {
                    Ok(_) => tracing::info!("scrobbled: {} - {}", track.artist, track.title),
                    Err(error) => tracing::warn!("scrobble failed: {}", error),
                }
            }

            if has_lastfm {
                let scrobble = scrobble::lastfm::make_scrobble(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                );
                match scrobble::lastfm::submit_scrobble(
                    &lastfm_api_key,
                    &lastfm_api_secret,
                    &lastfm_session_key,
                    &scrobble,
                )
                .await
                {
                    Ok(_) => {
                        tracing::info!("Last.fm scrobbled: {} - {}", track.artist, track.title)
                    }
                    Err(error) => tracing::warn!("Last.fm scrobble failed: {}", error),
                }
            }

            if has_librefm {
                let scrobble = scrobble::librefm::make_scrobble(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                );
                match scrobble::librefm::submit_scrobble(
                    scrobble::librefm::API_KEY,
                    scrobble::librefm::API_SECRET,
                    &librefm_session_key,
                    &scrobble,
                )
                .await
                {
                    Ok(_) => {
                        tracing::info!("Libre.fm scrobbled: {} - {}", track.artist, track.title)
                    }
                    Err(error) => tracing::warn!("Libre.fm scrobble failed: {}", error),
                }
            }

            let token_raw = config.read().musicbrainz_token.clone();
            if !token_raw.is_empty() {
                let auth = musicbrainz_auth(&token_raw);
                let listen = scrobble::musicbrainz::make_listen(
                    &track.artist,
                    &track.title,
                    Some(&track.album),
                    mbids,
                );
                match scrobble::musicbrainz::submit_listens(&auth, vec![listen], "single").await {
                    Ok(_) => {
                        tracing::info!("MusicBrainz scrobbled: {} - {}", track.artist, track.title)
                    }
                    Err(error) => tracing::warn!("MusicBrainz scrobble failed: {}", error),
                }
            }
        }
        .instrument(span),
    );
}

fn musicbrainz_auth(token: &str) -> String {
    if token.contains(' ') {
        token.to_string()
    } else {
        format!("Token {token}")
    }
}

fn musicbrainz_ids(track: &Track, enabled: bool) -> Option<HashMap<&str, &str>> {
    if !enabled {
        return None;
    }

    let mut map = HashMap::new();
    if let Some(mbid) = &track.musicbrainz_release_id {
        map.insert("release_mbid", mbid.as_str());
    }
    if let Some(mbid) = &track.musicbrainz_recording_id {
        map.insert("recording_mbid", mbid.as_str());
    }
    if let Some(mbid) = &track.musicbrainz_track_id {
        map.insert("track_mbid", mbid.as_str());
    }
    Some(map)
}

async fn sleep_threshold(duration: Duration) {
    tokio::time::sleep(duration).await;
}
