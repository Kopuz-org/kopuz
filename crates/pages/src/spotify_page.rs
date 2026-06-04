use components::search_bar::SearchBar;
use config::{AppConfig, SpotifyBackendKind};
use dioxus::prelude::*;
use hooks::PlayerController;
use reader::{Library, PlaylistStore};
use spotify::provider::{PlaybackDevice, PlaylistSummary, StreamingProvider, TrackSummary};

use crate::spotify_search::{SpotifySearch, spotify_summary_to_track};
use crate::spotify_settings::SpotifyBackendCache;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SpotifyView {
    Home,
    Search,
    Library,
    Playlists,
    Favorites,
}

#[component]
pub fn SpotifyPage(
    library: Signal<Library>,
    playlist_store: Signal<PlaylistStore>,
    config: Signal<AppConfig>,
    view: SpotifyView,
) -> Element {
    let cache = use_hook(SpotifyBackendCache::default);
    let enabled = config.read().spotify.enabled;
    let has_client_id = !config.read().spotify.client_id.trim().is_empty();

    let title = match view {
        SpotifyView::Home => "Spotify",
        SpotifyView::Search => "Spotify Search",
        SpotifyView::Library => "Spotify Library",
        SpotifyView::Playlists => "Spotify Playlists",
        SpotifyView::Favorites => "Spotify Liked Songs",
    };

    rsx! {
        div { class: "h-full w-full overflow-y-auto px-6 py-5 pb-32",
            div { class: "mb-6",
                div { class: "flex items-center gap-3 mb-2",
                    i { class: "fa-brands fa-spotify text-green-400 text-2xl" }
                    h1 { class: "text-3xl font-bold text-white", "{title}" }
                }
                p { class: "text-white/50 text-sm max-w-2xl",
                    "Spotify is controlled from Kopuz through the main queue and player controls. Choose the Spotify playback device Kopuz should target."
                }
            }

            if !enabled || !has_client_id {
                div { class: "bg-yellow-900/30 border border-yellow-700/50 text-yellow-100 p-4 rounded text-sm mb-4",
                    "Spotify is not configured. Enable Spotify, set a Client ID in Settings, then log in."
                }
            } else {
                SpotifyDeviceSelector { config, cache: cache.clone() }
            }

            match view {
                SpotifyView::Home => rsx! { SpotifyHome { library, playlist_store, config, cache } },
                SpotifyView::Search => rsx! { SpotifySearchView { library, playlist_store, config, cache } },
                SpotifyView::Library => rsx! { SpotifyTracksView { config, cache, title: "Saved tracks".to_string(), saved_only: true } },
                SpotifyView::Favorites => rsx! { SpotifyTracksView { config, cache, title: "Liked songs".to_string(), saved_only: true } },
                SpotifyView::Playlists => rsx! { SpotifyPlaylistsView { config, cache } },
            }
        }
    }
}

#[component]
fn SpotifyDeviceSelector(config: Signal<AppConfig>, cache: SpotifyBackendCache) -> Element {
    let mut devices = use_signal(Vec::<PlaybackDevice>::new);
    let mut loading = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    let selected_device_id = config.read().spotify.default_device_id.clone();
    let backend_kind = config.read().spotify.backend;

    {
        let cache = cache.clone();
        use_effect(move || {
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            loading.set(true);
            error.set(None);
            spawn(async move {
                let Some(backend) = cache.get_or_build(&cfg).await else {
                    devices.set(Vec::new());
                    loading.set(false);
                    return;
                };
                match backend.devices().await {
                    Ok(items) => devices.set(items),
                    Err(e) => error.set(Some(e.to_string())),
                }
                loading.set(false);
            });
        });
    }

    let selected_name = devices()
        .into_iter()
        .find(|device| device.id == selected_device_id)
        .map(|device| device.name)
        .unwrap_or_else(|| "No device selected".to_string());
    let cache_for_mode_button = cache.clone();
    let cache_for_refresh = cache.clone();

    rsx! {
        section { class: "rounded-xl border border-white/10 bg-white/[0.03] p-4 mb-6",
            div { class: "flex flex-col md:flex-row md:items-center md:justify-between gap-3",
                div {
                    h2 { class: "text-white font-semibold", "Playback device" }
                    p { class: "text-white/50 text-sm",
                        "Selected: {selected_name}. Open Spotify on the device you want, then refresh/select it here."
                    }
                }
                div { class: "flex items-center gap-2",
                    if backend_kind != SpotifyBackendKind::Connect {
                        button {
                            class: "px-3 py-1.5 rounded bg-white/10 hover:bg-white/20 text-white text-sm",
                            onclick: move |_| {
                                config.write().spotify.backend = SpotifyBackendKind::Connect;
                                let cache = cache_for_mode_button.clone();
                                spawn(async move { cache.invalidate().await; });
                            },
                            "Use selected Spotify device"
                        }
                    }
                    button {
                        class: "px-3 py-1.5 rounded bg-white/10 hover:bg-white/20 text-white text-sm disabled:opacity-50",
                        disabled: loading(),
                        onclick: move |_| {
                            let cfg = config.read().spotify.clone();
                            let cache = cache_for_refresh.clone();
                            loading.set(true);
                            error.set(None);
                            spawn(async move {
                                if let Some(backend) = cache.get_or_build(&cfg).await {
                                    match backend.devices().await {
                                        Ok(items) => devices.set(items),
                                        Err(e) => error.set(Some(e.to_string())),
                                    }
                                }
                                loading.set(false);
                            });
                        },
                        if loading() { "Refreshing..." } else { "Refresh devices" }
                    }
                }
            }
            if let Some(e) = error() {
                div { class: "text-red-300 text-sm", "{e}" }
            }
            div { class: "flex flex-wrap gap-2 mt-4",
                for device in devices().into_iter() {
                    {
                        let device_id = device.id.clone();
                        let name = device.name.clone();
                        let kind = device.kind.clone();
                        let active = device.is_active;
                        let restricted = device.is_restricted;
                        let is_selected = device.id == selected_device_id;
                        let cache_for_click = cache.clone();
                        rsx! {
                            button {
                                key: "{device_id}",
                                class: if is_selected {
                                    "px-3 py-2 rounded-lg border border-green-500/60 bg-green-500/15 text-left"
                                } else {
                                    "px-3 py-2 rounded-lg border border-white/10 bg-white/[0.03] hover:bg-white/[0.07] text-left disabled:opacity-40"
                                },
                                disabled: restricted || device_id.is_empty(),
                                onclick: move |_| {
                                    let mut cfg = config.write();
                                    cfg.spotify.backend = SpotifyBackendKind::Connect;
                                    cfg.spotify.default_device_id = device_id.clone();
                                    let cache = cache_for_click.clone();
                                    spawn(async move { cache.invalidate().await; });
                                },
                                div { class: "text-white text-sm font-medium", "{name}" }
                                div { class: "text-white/40 text-xs",
                                    "{kind}"
                                    if active { " · active" }
                                    if restricted { " · restricted" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SpotifySearchView(
    library: Signal<Library>,
    playlist_store: Signal<PlaylistStore>,
    config: Signal<AppConfig>,
    cache: SpotifyBackendCache,
) -> Element {
    let search_query = use_signal(String::new);
    rsx! {
        SearchBar { search_query }
        SpotifySearch { library, playlist_store, config, cache, search_query }
    }
}

#[component]
fn SpotifyHome(
    library: Signal<Library>,
    playlist_store: Signal<PlaylistStore>,
    config: Signal<AppConfig>,
    cache: SpotifyBackendCache,
) -> Element {
    rsx! {
        div { class: "space-y-6 mb-8",
            div { class: "max-w-3xl",
                h2 { class: "text-white font-semibold mb-2", "Find music" }
                p { class: "text-white/50 text-sm mb-4", "Search Spotify and play results through Kopuz's queue." }
                SpotifySearchView { library, playlist_store, config, cache: cache.clone() }
            }
            div { class: "max-w-3xl",
                h2 { class: "text-white font-semibold mb-2", "Liked songs" }
                p { class: "text-white/50 text-sm mb-4", "Preview and play your Spotify saved tracks." }
                SpotifyTracksView { config, cache, title: "Liked songs".to_string(), saved_only: true }
            }
        }
    }
}

#[component]
fn SpotifyTracksView(
    config: Signal<AppConfig>,
    cache: SpotifyBackendCache,
    title: String,
    saved_only: bool,
) -> Element {
    let mut tracks = use_signal(Vec::<TrackSummary>::new);
    let mut loading = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);

    {
        let cache = cache.clone();
        use_effect(move || {
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            loading.set(true);
            error.set(None);
            spawn(async move {
                let Some(backend) = cache.get_or_build(&cfg).await else {
                    tracks.set(Vec::new());
                    loading.set(false);
                    return;
                };
                let result = if saved_only {
                    backend.saved_tracks().await
                } else {
                    backend.saved_tracks().await
                };
                match result {
                    Ok(items) => tracks.set(items),
                    Err(e) => error.set(Some(e.to_string())),
                }
                loading.set(false);
            });
        });
    }

    rsx! {
        section { class: "rounded-xl border border-white/10 bg-white/[0.03] p-4",
            h2 { class: "text-white font-semibold mb-3", "{title}" }
            if loading() {
                div { class: "text-white/40 text-sm", "Loading Spotify tracks..." }
            }
            if let Some(e) = error() {
                div { class: "text-red-300 text-sm", "{e}" }
            }
            SpotifyTrackList { tracks: tracks(), empty_text: "No Spotify tracks found.".to_string() }
        }
    }
}

#[component]
fn SpotifyPlaylistsView(config: Signal<AppConfig>, cache: SpotifyBackendCache) -> Element {
    let mut playlists = use_signal(Vec::<PlaylistSummary>::new);
    let mut selected = use_signal(|| Option::<PlaylistSummary>::None);
    let mut tracks = use_signal(Vec::<TrackSummary>::new);
    let mut loading = use_signal(|| false);
    let mut track_loading = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);

    {
        let cache = cache.clone();
        use_effect(move || {
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            loading.set(true);
            error.set(None);
            spawn(async move {
                let Some(backend) = cache.get_or_build(&cfg).await else {
                    playlists.set(Vec::new());
                    loading.set(false);
                    return;
                };
                match backend.user_playlists().await {
                    Ok(items) => playlists.set(items),
                    Err(e) => error.set(Some(e.to_string())),
                }
                loading.set(false);
            });
        });
    }

    rsx! {
        div { class: "grid grid-cols-1 xl:grid-cols-[360px_1fr] gap-4",
            section { class: "rounded-xl border border-white/10 bg-white/[0.03] p-4",
                h2 { class: "text-white font-semibold mb-3", "Playlists" }
                if loading() {
                    div { class: "text-white/40 text-sm", "Loading Spotify playlists..." }
                }
                if let Some(e) = error() {
                    div { class: "text-red-300 text-sm", "{e}" }
                }
                div { class: "flex flex-col gap-1",
                    for playlist in playlists().into_iter() {
                        {
                            let playlist_for_click = playlist.clone();
                            let cache = cache.clone();
                            rsx! {
                                button {
                                    key: "{playlist.id}",
                                    class: "flex items-center gap-3 p-2 rounded hover:bg-white/5 text-left",
                                    onclick: move |_| {
                                        let cfg = config.read().spotify.clone();
                                        let playlist = playlist_for_click.clone();
                                        let cache = cache.clone();
                                        selected.set(Some(playlist.clone()));
                                        track_loading.set(true);
                                        tracks.set(Vec::new());
                                        spawn(async move {
                                            if let Some(backend) = cache.get_or_build(&cfg).await {
                                                match backend.playlist_tracks(&playlist.id).await {
                                                    Ok(items) => tracks.set(items),
                                                    Err(e) => error.set(Some(e.to_string())),
                                                }
                                            }
                                            track_loading.set(false);
                                        });
                                    },
                                    if let Some(url) = playlist.artwork_url.clone() {
                                        img { src: "{url}", class: "w-10 h-10 rounded object-cover" }
                                    } else {
                                        div { class: "w-10 h-10 rounded bg-white/10 flex items-center justify-center",
                                            i { class: "fa-solid fa-list text-white/30" }
                                        }
                                    }
                                    div { class: "min-w-0",
                                        div { class: "text-white truncate", "{playlist.name}" }
                                        div { class: "text-white/40 text-xs truncate", "Spotify playlist" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            section { class: "rounded-xl border border-white/10 bg-white/[0.03] p-4",
                if let Some(playlist) = selected() {
                    h2 { class: "text-white font-semibold mb-3", "{playlist.name}" }
                    if track_loading() {
                        div { class: "text-white/40 text-sm", "Loading tracks..." }
                    }
                    SpotifyTrackList { tracks: tracks(), empty_text: "Select a playlist to view tracks.".to_string() }
                } else {
                    div { class: "text-white/40 text-sm", "Select a Spotify playlist." }
                }
            }
        }
    }
}

#[component]
fn SpotifyTrackList(tracks: Vec<TrackSummary>, empty_text: String) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    if tracks.is_empty() {
        return rsx! { div { class: "text-white/40 text-sm", "{empty_text}" } };
    }

    rsx! {
        div { class: "flex flex-col gap-1",
            for track in tracks.into_iter() {
                {
                    let queue_track = spotify_summary_to_track(&track);
                    let title = track.title.clone();
                    let artist = track.artists.join(", ");
                    let album = track.album.clone();
                    let artwork = track.artwork_url.clone();
                    rsx! {
                        button {
                            key: "{track.uri}",
                            class: "flex items-center gap-3 p-2 rounded hover:bg-white/5 text-left",
                            onclick: move |_| ctrl.play_queue_linear(vec![queue_track.clone()]),
                            if let Some(url) = artwork {
                                img { src: "{url}", class: "w-10 h-10 rounded object-cover" }
                            } else {
                                div { class: "w-10 h-10 rounded bg-white/10 flex items-center justify-center",
                                    i { class: "fa-solid fa-music text-white/30" }
                                }
                            }
                            div { class: "min-w-0",
                                div { class: "text-white truncate", "{title}" }
                                div { class: "text-white/50 text-xs truncate", "{artist}" }
                                div { class: "text-white/30 text-xs truncate", "{album}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
