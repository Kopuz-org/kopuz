use crate::player::player::Player;
#[cfg(target_os = "macos")]
use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
use dioxus::prelude::*;

mod components;
pub mod config;
pub mod hooks;
pub mod pages;
pub mod player;
pub mod reader;
pub mod utils;
use components::{bottombar::Bottombar, sidebar::Sidebar};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Route {
    Home,
    Search,
    Library,
    Album,
    Playlists,
    Settings,
}

const FAVICON: Asset = asset!("assets/favicon.ico");
const MAIN_CSS: Asset = asset!("assets/main.css");
const THEME_CSS: Asset = asset!("assets/themes.css");
const TAILWIND_CSS: Asset = asset!("assets/tailwind.css");

fn main() {
    let mut window = dioxus::desktop::WindowBuilder::new()
        .with_title("Rusic")
        .with_resizable(true);

    #[cfg(target_os = "macos")]
    {
        window = window
            .with_title_hidden(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true);
    }

    let config = dioxus::desktop::Config::new()
        .with_window(window)
        .with_custom_protocol("artwork", |_headers, request| {
            let path = request.uri().path();
            let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();

            let mime = if decoded.ends_with(".png") {
                "image/png"
            } else {
                "image/jpeg"
            };

            let decoded_path = decoded.as_ref();
            let content = std::fs::read(std::path::Path::new(decoded_path))
                .or_else(|_| {
                    if decoded_path.starts_with('/') {
                        std::fs::read(std::path::Path::new(&decoded_path[1..]))
                    } else {
                        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
                    }
                })
                .map(|bytes| std::borrow::Cow::from(bytes))
                .unwrap_or_else(|_| std::borrow::Cow::from(Vec::new()));

            http::Response::builder()
                .header("Content-Type", mime)
                .header("Access-Control-Allow-Origin", "*")
                .body(content)
                .unwrap()
        });

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

#[component]
fn App() -> Element {
    let mut library = use_signal(reader::Library::default);
    let mut current_route = use_signal(|| Route::Home);
    let cache_dir = use_memo(|| std::path::Path::new("./cache").to_path_buf());
    let lib_path = use_memo(move || cache_dir().join("library.json"));
    let config_path = use_memo(move || cache_dir().join("config.json"));
    let config = use_signal(|| config::AppConfig::load(&config_path()));
    let playlist_path = use_memo(move || cache_dir().join("playlists.json"));
    let playlist_store =
        use_signal(|| reader::PlaylistStore::load(&playlist_path()).unwrap_or_default());
    let cover_cache = use_memo(move || std::path::Path::new("./cache/covers").to_path_buf());
    let mut trigger_rescan = use_signal(|| 0);
    let current_playing = use_signal(|| 0);
    let player = use_signal(Player::new);
    let mut current_song_cover_url = use_signal(|| String::new());
    let current_song_title = use_signal(|| String::new());
    let current_song_artist = use_signal(|| String::new());
    let current_song_duration = use_signal(|| 0u64);
    let current_song_progress = use_signal(|| 0u64);
    let volume = use_signal(|| 1.0f32);

    let is_playing = use_signal(|| false);

    let mut selected_album_id = use_signal(|| String::new());
    let mut search_query = use_signal(|| String::new());

    use_effect(move || {
        let _ = config.read().save(&config_path());
    });

    use_effect(move || {
        let _ = playlist_store.read().save(&playlist_path());
    });

    use_hook(move || {
        spawn(async move {
            if let Ok(loaded) = reader::Library::load(&lib_path()) {
                library.set(loaded);
            }
        });
    });

    use_effect(move || {
        let music_dir = config.read().music_directory.clone();
        let _ = trigger_rescan.read();

        spawn(async move {
            if music_dir.exists() {
                let mut current_lib = library.peek().clone();

                if current_lib.root_path != music_dir {
                    current_lib = reader::Library::new(music_dir.clone());
                    library.set(current_lib.clone());
                }

                if let Ok(_) =
                    reader::scan_directory(music_dir, cover_cache(), &mut current_lib).await
                {
                    library.set(current_lib.clone());
                    let _ = current_lib.save(&lib_path());
                }
            }
        });
    });

    let queue = use_signal(|| Vec::<reader::Track>::new());
    let current_queue_index = use_signal(|| 0usize);

    use_future(move || {
        let mut player = player;
        let mut is_playing = is_playing;
        let mut current_song_progress = current_song_progress;
        let mut current_song_duration = current_song_duration;
        let mut current_song_title = current_song_title;
        let mut current_song_artist = current_song_artist;
        let mut current_queue_index = current_queue_index;
        let queue = queue;
        let library = library;

        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                if *is_playing.read() {
                    let pos = player.read().get_position();
                    current_song_progress.set(pos.as_secs());

                    if player.read().is_empty() {
                        let idx = *current_queue_index.read();
                        let q = queue.read();
                        if idx + 1 < q.len() {
                            let next_idx = idx + 1;
                            let track = &q[next_idx];
                            if let Ok(file) = std::fs::File::open(&track.path) {
                                if let Ok(source) =
                                    rodio::Decoder::new(std::io::BufReader::new(file))
                                {
                                    player.write().play(source);
                                    player.read().set_volume(*volume.peek());
                                    current_song_title.set(track.title.clone());
                                    current_song_artist.set(track.artist.clone());
                                    current_song_duration.set(track.duration);
                                    current_song_progress.set(0);

                                    // Update cover
                                    let lib = library.read();
                                    if let Some(album) =
                                        lib.albums.iter().find(|a| a.id == track.album_id)
                                    {
                                        if let Some(url) = crate::utils::format_artwork_url(
                                            album.cover_path.as_ref(),
                                        ) {
                                            current_song_cover_url.set(url);
                                        } else {
                                            current_song_cover_url.set(String::new());
                                        }
                                    } else {
                                        current_song_cover_url.set(String::new());
                                    }

                                    current_queue_index.set(next_idx);
                                }
                            }
                        } else {
                            is_playing.set(false);
                            player.write().pause();
                        }
                    }
                }
            }
        }
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: THEME_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Link { rel: "stylesheet", href: "https://fonts.bunny.net/css?family=jetbrains-mono:400,500,700,800&display=swap" }
        document::Link { rel: "stylesheet", href: "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css" }
        div {
            class: "flex flex-col h-screen theme-{config.read().theme}",
            div {
                class: "flex flex-1 overflow-hidden",
                Sidebar {
                    current_route,
                    on_navigate: move |route| {
                        if route == Route::Album {
                            selected_album_id.set(String::new());
                        }
                        if route == Route::Search && !search_query.read().is_empty() {
                            // Keep search query if already set? Or maybe clear it?
                            // For now keep it.
                        }
                        current_route.set(route);
                    }
                }
                div {
                    class: "flex-1 overflow-y-auto bg-black",
                    match *current_route.read() {
                        Route::Home => rsx! {
                            pages::home::Home {
                                library,
                                playlist_store,
                                on_select_album: move |id| {
                                    selected_album_id.set(id);
                                    current_route.set(Route::Album);
                                },
                                on_search_artist: move |artist| {
                                    search_query.set(artist);
                                    current_route.set(Route::Search);
                                }
                            }
                        },
                        Route::Search => rsx! {
                            pages::search::Search {
                                library: library,
                                playlist_store: playlist_store,
                                search_query: search_query,
                                player: player.clone(),
                                is_playing: is_playing,
                                current_playing: current_playing,
                                current_song_cover_url: current_song_cover_url,
                                current_song_title: current_song_title,
                                current_song_artist: current_song_artist,
                                current_song_duration: current_song_duration,
                                current_song_progress: current_song_progress,
                                queue: queue,
                                current_queue_index: current_queue_index,
                            }
                        },
                        Route::Library => rsx! {
                            pages::library::LibraryPage {
                                library: library,
                                playlist_store: playlist_store,
                                on_rescan: move |_| *trigger_rescan.write() += 1,
                                player: player.clone(),
                                is_playing: is_playing,
                                current_playing: current_playing,
                                current_song_cover_url: current_song_cover_url,
                                current_song_title: current_song_title,
                                current_song_artist: current_song_artist,
                                current_song_duration: current_song_duration,
                                current_song_progress: current_song_progress,
                                queue: queue,
                                current_queue_index: current_queue_index,
                            }
                        },
                        Route::Album => rsx! {
                            pages::album::Album {
                                library: library,
                                album_id: selected_album_id,
                                playlist_store: playlist_store,
                                player: player.clone(),
                                is_playing: is_playing,
                                current_playing: current_playing,
                                current_song_cover_url: current_song_cover_url,
                                current_song_title: current_song_title,
                                current_song_artist: current_song_artist,
                                current_song_duration: current_song_duration,
                                current_song_progress: current_song_progress,
                                queue: queue,
                                current_queue_index: current_queue_index,
                            }
                        },
                        Route::Playlists => rsx! {
                            pages::playlists::PlaylistsPage {
                                playlist_store: playlist_store,
                                library: library,
                                player: player.clone(),
                                is_playing: is_playing,
                                current_playing: current_playing,
                                current_song_cover_url: current_song_cover_url,
                                current_song_title: current_song_title,
                                current_song_artist: current_song_artist,
                                current_song_duration: current_song_duration,
                                current_song_progress: current_song_progress,
                                queue: queue,
                                current_queue_index: current_queue_index,
                            }
                        },
                        Route::Settings => rsx! { pages::settings::Settings { config } },
                    }
                }
            }
            Bottombar {
                library: library,
                current_song_cover_url: current_song_cover_url,
                current_song_title: current_song_title,
                current_song_artist: current_song_artist,
                player: player,
                is_playing: is_playing,
                current_song_duration: current_song_duration,
                current_song_progress: current_song_progress,
                queue: queue,
                current_queue_index: current_queue_index,
                volume: volume,
            }
        }
    }
}
