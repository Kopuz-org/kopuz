//! Spotify section embedded into the existing search page.
//!
//! When the user is logged in to Spotify, the same `search_query` that drives
//! local/server search is also issued against the Spotify Web API. Results
//! render as a list of tracks; clicking a row tells the active backend to
//! `play_uri` on the user's selected Connect device (or the SDK device, in
//! Web Playback mode).

use config::AppConfig;
use dioxus::prelude::*;
use hooks::PlayerController;
use std::path::PathBuf;

use crate::spotify_settings::SpotifyBackendCache;
use spotify::provider::{SearchResult, StreamingProvider, TrackSummary};

pub(crate) fn spotify_summary_to_track(summary: &TrackSummary) -> reader::models::Track {
    reader::models::Track {
        path: PathBuf::from(summary.uri.clone()),
        album_id: format!("spotify:{}", summary.id),
        title: summary.title.clone(),
        artist: summary.artists.join(", "),
        album: summary.album.clone(),
        duration: summary.duration_ms / 1000,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: summary.artwork_url.clone(),
        artists: summary.artists.clone(),
    }
}

pub(crate) fn spotify_result_to_track(result: &SearchResult) -> reader::models::Track {
    reader::models::Track {
        path: PathBuf::from(result.uri.clone()),
        album_id: format!("spotify:{}", result.id),
        title: result.title.clone(),
        artist: result.subtitle.clone(),
        album: "Spotify".to_string(),
        duration: result.duration_ms / 1000,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: result.artwork_url.clone(),
        artists: result
            .subtitle
            .split(',')
            .map(str::trim)
            .filter(|artist| !artist.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

#[component]
pub fn SpotifySearch(
    library: Signal<reader::Library>,
    playlist_store: Signal<reader::PlaylistStore>,
    config: Signal<AppConfig>,
    cache: SpotifyBackendCache,
    search_query: Signal<String>,
) -> Element {
    let mut results = use_signal(Vec::<SearchResult>::new);
    let mut loading = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    let mut import_status = use_signal(|| Option::<String>::None);
    let mut importing = use_signal(|| false);
    let mut last_query = use_signal(String::new);
    let mut ctrl = use_context::<PlayerController>();

    let import_spotify_playlists = {
        let cache = cache.clone();
        move |_| {
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            importing.set(true);
            import_status.set(None);
            spawn(async move {
                let Some(backend) = cache.get_or_build(&cfg).await else {
                    import_status.set(Some("Spotify is not configured.".into()));
                    importing.set(false);
                    return;
                };

                let mut cached_tracks = Vec::<reader::models::Track>::new();
                let mut imported_playlists = Vec::<reader::models::Playlist>::new();

                match backend.saved_tracks().await {
                    Ok(saved) if !saved.is_empty() => {
                        let tracks: Vec<_> = saved.iter().map(spotify_summary_to_track).collect();
                        cached_tracks.extend(tracks.clone());
                        imported_playlists.push(reader::models::Playlist {
                            id: "spotify:saved-tracks".to_string(),
                            name: "Spotify Liked Songs".to_string(),
                            tracks: tracks.into_iter().map(|track| track.path).collect(),
                            cover_path: None,
                        });
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(target: "spotify::ui", "loading Spotify saved tracks failed: {e}")
                    }
                }

                match backend.user_playlists().await {
                    Ok(playlists) => {
                        for playlist in playlists {
                            match backend.playlist_tracks(&playlist.id).await {
                                Ok(items) => {
                                    let tracks: Vec<_> =
                                        items.iter().map(spotify_summary_to_track).collect();
                                    cached_tracks.extend(tracks.clone());
                                    imported_playlists.push(reader::models::Playlist {
                                        id: format!("spotify:{}", playlist.id),
                                        name: format!("Spotify - {}", playlist.name),
                                        tracks: tracks
                                            .into_iter()
                                            .map(|track| track.path)
                                            .collect(),
                                        cover_path: None,
                                    });
                                }
                                Err(e) => {
                                    tracing::warn!(target: "spotify::ui", "loading Spotify playlist {} failed: {e}", playlist.id)
                                }
                            }
                        }
                    }
                    Err(e) => {
                        import_status.set(Some(format!("Could not load Spotify playlists: {e}")));
                        importing.set(false);
                        return;
                    }
                }

                let imported_count = imported_playlists.len();
                let cached_count = cached_tracks.len();
                library.with_mut(|lib| {
                    for track in cached_tracks {
                        lib.add_track(track);
                    }
                });
                playlist_store.with_mut(|store| {
                    for playlist in imported_playlists {
                        if let Some(existing) =
                            store.playlists.iter_mut().find(|p| p.id == playlist.id)
                        {
                            *existing = playlist;
                        } else {
                            store.playlists.push(playlist);
                        }
                    }
                });
                import_status.set(Some(format!(
                    "Imported {imported_count} Spotify playlists and cached {cached_count} tracks."
                )));
                importing.set(false);
            });
        }
    };

    // Re-query whenever search_query changes meaningfully.
    {
        let cache = cache.clone();
        use_effect(move || {
            let q = search_query.read().trim().to_string();
            if q == last_query.read().as_str() {
                return;
            }
            last_query.set(q.clone());
            if q.is_empty() {
                results.set(Vec::new());
                error.set(None);
                return;
            }
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            loading.set(true);
            spawn(async move {
                let Some(backend) = cache.get_or_build(&cfg).await else {
                    loading.set(false);
                    return;
                };
                match backend.search(&q).await {
                    Ok(r) => {
                        results.set(r);
                        error.set(None);
                    }
                    Err(e) => {
                        error.set(Some(e.to_string()));
                        results.set(Vec::new());
                    }
                }
                loading.set(false);
            });
        });
    }

    if !config.read().spotify.enabled {
        return rsx! {};
    }

    rsx! {
        section { class: "mt-6",
            div { class: "flex items-center justify-between mb-2",
                h2 { class: "text-lg font-semibold text-white/80", "Spotify" }
                button {
                    class: "px-3 py-1 rounded bg-white/10 hover:bg-white/20 text-white text-xs disabled:opacity-50",
                    disabled: importing(),
                    onclick: import_spotify_playlists,
                    if importing() { "Importing..." } else { "Import playlists" }
                }
            }
            if let Some(msg) = import_status() {
                div { class: "text-white/50 text-sm mb-2", "{msg}" }
            }
            if loading() {
                div { class: "text-white/40 text-sm", "Searching Spotify..." }
            }
            if let Some(e) = error() {
                div { class: "text-red-300 text-sm", "{e}" }
            }
            if !results().is_empty() {
                div { class: "flex flex-col gap-1",
                    for r in results().iter().cloned() {
                        {
                            let title = r.title.clone();
                            let subtitle = r.subtitle.clone();
                            let artwork = r.artwork_url.clone();
                            let track = spotify_result_to_track(&r);
                            rsx!(
                                button {
                                    key: "{r.id}",
                                    class: "flex items-center gap-3 px-2 py-1 rounded hover:bg-white/5 text-left",
                                    onclick: move |_| {
                                        ctrl.play_queue_linear(vec![track.clone()]);
                                    },
                                    if let Some(url) = artwork {
                                        img { src: "{url}", class: "w-10 h-10 rounded object-cover" }
                                    }
                                    div { class: "flex flex-col",
                                        span { class: "text-white", "{title}" }
                                        span { class: "text-white/50 text-xs", "{subtitle}" }
                                    }
                                }
                            )
                        }
                    }
                }
            }
        }
    }
}
