use components::{
    bottombar::Bottombar, fullscreen::Fullscreen, rightbar::Rightbar, sidebar::Sidebar,
    titlebar::Titlebar,
};
#[cfg(not(target_arch = "wasm32"))]
use dioxus::desktop::tao::dpi::LogicalSize;
#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
use dioxus::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use discord_presence::Presence;
use player::player::Player;
use reader::FavoritesStore;
use rusic_route::Route;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use crate::web_storage::{
    load_web_config, load_web_favorites, load_web_library, load_web_playlists, load_web_ui_state,
    save_web_config, save_web_favorites, save_web_library, save_web_playlists,
    save_web_ui_state,
};

mod web_storage;

rust_i18n::i18n!("../locales");

const FAVICON: Asset = asset!("../assets/favicon.ico");
const MAIN_CSS: Asset = asset!("../assets/main.css");
const THEME_CSS: Asset = asset!("../assets/themes.css");
const TAILWIND_CSS: Asset = asset!("../assets/tailwind.css");
const REDUCED_ANIMATIONS_CSS: Asset = asset!("../assets/reduced-animations.css");

#[cfg(not(target_arch = "wasm32"))]
static PRESENCE: std::sync::OnceLock<Option<Arc<Presence>>> = std::sync::OnceLock::new();

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let presence: Option<Arc<Presence>> = match Presence::new("1470087339639443658") {
            Ok(p) => {
                println!("Discord presence connected!");
                Some(Arc::new(p))
            }
            Err(e) => {
                eprintln!("Failed to connect to Discord: {e}");
                None
            }
        };

        PRESENCE.set(presence).ok();

        #[cfg(target_os = "macos")]
        {
            player::systemint::init();
        }

        let mut window = dioxus::desktop::WindowBuilder::new()
            .with_title("Rusic")
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(1350.0, 800.0));

        #[cfg(target_os = "macos")]
        {
            window = window
                .with_title_hidden(true)
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true);
        }

        #[cfg(target_os = "linux")]
        {
            window = window.with_decorations(false);
        }

        let config = dioxus::desktop::Config::new()
            .with_window(window)
            .with_custom_protocol("artwork", |_headers, request| {
                let uri = request.uri();

                let file_path: String = uri
                    .query()
                    .and_then(|q| {
                        q.split('&')
                            .find_map(|kv| kv.strip_prefix("p="))
                            .map(|encoded| {
                                percent_encoding::percent_decode_str(encoded)
                                    .decode_utf8_lossy()
                                    .into_owned()
                            })
                    })
                    .unwrap_or_default();

                if file_path.is_empty() {
                    return http::Response::builder()
                        .status(400)
                        .body(std::borrow::Cow::from(Vec::new()))
                        .unwrap();
                }

                // convert forward slashes back to backslashes for proper path handling
                #[cfg(target_os = "windows")]
                let file_path = file_path.replace('/', "\\");

                #[cfg(not(target_os = "windows"))]
                let file_path = if file_path.starts_with('~') {
                    if let Ok(home) = std::env::var("HOME") {
                        file_path.replacen('~', &home, 1)
                    } else {
                        file_path
                    }
                } else {
                    file_path
                };

                let path = std::path::Path::new(&file_path);

                if !path.exists() {
                    eprintln!("[artwork] File not found: {}", file_path);
                    return http::Response::builder()
                        .status(404)
                        .body(std::borrow::Cow::from(Vec::new()))
                        .unwrap();
                }

                let mime = if file_path.ends_with(".png") {
                    "image/png"
                } else {
                    "image/jpeg"
                };

                match std::fs::read(path) {
                    Ok(content) => http::Response::builder()
                        .header("Content-Type", mime)
                        .header("Access-Control-Allow-Origin", "*")
                        .header("Cache-Control", "public, max-age=31536000")
                        .body(std::borrow::Cow::from(content))
                        .unwrap(),
                    Err(e) => {
                        eprintln!("[artwork] Failed to read file {}: {}", file_path, e);
                        http::Response::builder()
                            .status(500)
                            .body(std::borrow::Cow::from(Vec::new()))
                            .unwrap()
                    }
                }
            });

        dioxus::LaunchBuilder::desktop()
            .with_cfg(config)
            .launch(App);
    }

    #[cfg(target_arch = "wasm32")]
    {
        dioxus::launch(App);
    }
}

#[component]
fn App() -> Element {
    let mut library = use_signal(reader::Library::default);
    let mut current_route = use_signal(|| Route::Home);
    let cache_dir = use_memo(move || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = directories::ProjectDirs::from("com", "temidaradev", "rusic")
                .map(|dirs| dirs.cache_dir().to_path_buf())
                .unwrap_or_else(|| std::path::Path::new("./cache").to_path_buf());
            let _ = std::fs::create_dir_all(&path);
            path
        }
        #[cfg(target_arch = "wasm32")]
        std::path::PathBuf::from("./cache")
    });
    let config_dir = use_memo(move || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = directories::ProjectDirs::from("com", "temidaradev", "rusic")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::path::Path::new("./config").to_path_buf());
            let _ = std::fs::create_dir_all(&path);
            path
        }
        #[cfg(target_arch = "wasm32")]
        std::path::PathBuf::from("./config")
    });
    let lib_path = use_memo(move || cache_dir().join("library.json"));
    let config_path = use_memo(move || config_dir().join("config.json"));
    let mut config = use_signal(config::AppConfig::default);
    #[allow(unused_variables)]
    let playlist_path = use_memo(move || cache_dir().join("playlists.json"));
    let mut playlist_store = use_signal(reader::PlaylistStore::default);
    #[allow(unused_variables)]
    let favorites_path = use_memo(move || cache_dir().join("favorites.json"));
    let mut favorites_store = use_signal(FavoritesStore::default);
    let mut initial_load_done = use_signal(|| false);
    #[allow(unused_variables)]
    let cover_cache = use_memo(move || cache_dir().join("covers"));
    #[cfg(not(target_arch = "wasm32"))]
    let _ = std::fs::create_dir_all(cover_cache());
    let mut trigger_rescan = use_signal(|| 0);
    let current_playing = use_signal(|| 0);
    let mut player = use_signal(Player::new);
    let current_song_cover_url = use_signal(String::new);
    let current_song_title = use_signal(String::new);
    let current_song_artist = use_signal(String::new);
    let current_song_album = use_signal(String::new);
    let current_song_duration = use_signal(|| 0u64);
    let current_song_khz = use_signal(|| 0u32);
    let current_song_bitrate = use_signal(|| 0u8);
    let current_song_progress = use_signal(|| 0u64);
    let mut volume = use_signal(|| 1.0f32);

    let is_playing = use_signal(|| false);
    let is_fullscreen = use_signal(|| false);
    let is_rightbar_open = use_signal(|| false);
    let rightbar_width = use_signal(|| 320usize);
    let mut palette = use_signal(|| Option::<Vec<utils::color::Color>>::None);

    use_effect(move || {
        let url = current_song_cover_url.read().clone();
        if !url.is_empty() {
            spawn(async move {
                if let Some(colors) = utils::color::get_palette_from_url(&url).await {
                    palette.set(Some(colors));
                }
            });
        } else {
            palette.set(None);
        }
    });

    #[cfg(not(target_arch = "wasm32"))]
    let presence = PRESENCE.get().cloned().flatten();
    #[cfg(not(target_arch = "wasm32"))]
    provide_context(presence.clone());

    let mut selected_album_id = use_signal(String::new);
    let mut selected_playlist_id = use_signal(|| None::<String>);
    let mut selected_artist_name = use_signal(String::new);
    let search_query = use_signal(String::new);
    let mut last_server_playlist_key = use_signal(|| None::<String>);
    let mut server_playlist_key_initialized = use_signal(|| false);

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        let current_server_key = {
            let conf = config.read();
            conf.server.as_ref().map(|server| {
                format!(
                    "{:?}|{}|{}|{}",
                    server.service,
                    server.url,
                    server.user_id.as_deref().unwrap_or_default(),
                    server.access_token.as_deref().unwrap_or_default()
                )
            })
        };

        if !*server_playlist_key_initialized.read() {
            last_server_playlist_key.set(current_server_key);
            server_playlist_key_initialized.set(true);
            return;
        }

        if *last_server_playlist_key.read() != current_server_key {
            last_server_playlist_key.set(current_server_key);
            selected_playlist_id.set(None);
            playlist_store.write().jellyfin_playlists.clear();
        }
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let config_snapshot = config.read().clone();
            let path = config_path();
            spawn(async move {
                let result = tokio::task::spawn_blocking(move || config_snapshot.save(&path)).await;
                if let Ok(Err(e)) = result {
                    eprintln!("Failed to save config: {}", e);
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let cfg_snapshot = config.read().clone();
            save_web_config(&cfg_snapshot);
        }
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let store_snapshot = playlist_store.read().clone();
            let path = playlist_path();
            spawn(async move {
                let result = tokio::task::spawn_blocking(move || store_snapshot.save(&path)).await;
                if let Ok(Err(e)) = result {
                    eprintln!("Failed to save playlists: {}", e);
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let store_snapshot = playlist_store.read().clone();
            save_web_playlists(&store_snapshot);
        }
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let lib_snapshot = library.read().clone();
            let path = lib_path();
            spawn(async move {
                let result = tokio::task::spawn_blocking(move || lib_snapshot.save(&path)).await;
                if let Ok(Err(e)) = result {
                    eprintln!("Failed to save library: {}", e);
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let lib_snapshot = library.read().clone();
            save_web_library(&lib_snapshot);
        }
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let store_snapshot = favorites_store.read().clone();
            let path = favorites_path();
            spawn(async move {
                let result = tokio::task::spawn_blocking(move || store_snapshot.save(&path)).await;
                if let Ok(Err(e)) = result {
                    eprintln!("Failed to save favorites: {}", e);
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let store_snapshot = favorites_store.read().clone();
            save_web_favorites(&store_snapshot);
        }
    });

    use_hook(move || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let lib_path = lib_path();
            let config_path = config_path();
            let playlist_path = playlist_path();
            let favorites_path = favorites_path();

            spawn(async move {
                let lib_path_c = lib_path.clone();
                let config_path_c = config_path.clone();
                let playlist_path_c = playlist_path.clone();
                let favorites_path_c = favorites_path.clone();

                let (lib_res, cfg_res, pl_res, fav_res) = tokio::join!(
                    tokio::task::spawn_blocking(move || reader::Library::load(&lib_path_c)),
                    tokio::task::spawn_blocking(move || config::AppConfig::load(&config_path_c)),
                    tokio::task::spawn_blocking(move || reader::PlaylistStore::load(
                        &playlist_path_c
                    )),
                    tokio::task::spawn_blocking(move || FavoritesStore::load(&favorites_path_c)),
                );

                if let Ok(Ok(loaded)) = lib_res {
                    library.set(loaded);
                }
                if let Ok(loaded) = cfg_res {
                    config.set(loaded.clone());
                    volume.set(loaded.volume);
                    player.write().set_volume(loaded.volume);
                    rust_i18n::set_locale(&loaded.language);
                }
                if let Ok(Ok(loaded)) = pl_res {
                    playlist_store.set(loaded);
                }
                if let Ok(Ok(loaded)) = fav_res {
                    favorites_store.set(loaded);
                }

                initial_load_done.set(true);
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let mut loaded = load_web_config().unwrap_or_default();
            if loaded.server.is_none() {
                loaded.active_source = config::MusicSource::Server;
            }
            let loaded_volume = loaded.volume;
            let loaded_language = loaded.language.clone();
            config.set(loaded);
            volume.set(loaded_volume);
            player.write().set_volume(loaded_volume);
            rust_i18n::set_locale(&loaded_language);

            if let Some((
                route,
                saved_album_id,
                saved_playlist_id,
                saved_artist_name,
                saved_search_query,
            )) = load_web_ui_state()
            {
                current_route.set(route);
                selected_album_id.set(saved_album_id);
                selected_playlist_id.set(saved_playlist_id);
                selected_artist_name.set(saved_artist_name);
                search_query.set(saved_search_query);
            }

            if let Some(loaded_library) = load_web_library() {
                library.set(loaded_library);
            }
            if let Some(loaded_playlists) = load_web_playlists() {
                playlist_store.set(loaded_playlists);
            }
            if let Some(loaded_favorites) = load_web_favorites() {
                favorites_store.set(loaded_favorites);
            }

            initial_load_done.set(true);
        }
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        #[cfg(target_arch = "wasm32")]
        {
            let route = *current_route.read();
            let album_id = selected_album_id.read().clone();
            let playlist_id = selected_playlist_id.read().clone();
            let artist_name = selected_artist_name.read().clone();
            let query = search_query.read().clone();

            save_web_ui_state(
                route,
                &album_id,
                playlist_id.as_deref(),
                &artist_name,
                &query,
            );
        }
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }
        let music_dir = config.read().music_directory.clone();
        let _ = trigger_rescan.read();

        #[cfg(not(target_arch = "wasm32"))]
        spawn(async move {
            if music_dir.exists() {
                let mut current_lib = library.peek().clone();

                if current_lib.root_path != music_dir {
                    current_lib = reader::Library::new(music_dir.clone());
                    library.set(current_lib.clone());
                }

                if (reader::scan_directory(music_dir, cover_cache(), &mut current_lib).await)
                    .is_ok()
                {
                    current_lib.tracks.retain(|t| t.path.exists());
                    let valid_album_ids: std::collections::HashSet<_> = current_lib
                        .tracks
                        .iter()
                        .map(|t| t.album_id.clone())
                        .collect();
                    current_lib
                        .albums
                        .retain(|a| valid_album_ids.contains(&a.id));

                    library.set(current_lib.clone());
                    let _ = current_lib.save(&lib_path());
                }
            }
        });
    });

    use_effect(move || {
        let _ = current_route.read();
        let _ = dioxus::document::eval(
            "let el = document.getElementById('main-scroll-area'); if (el) el.scrollTop = 0;",
        );
    });

    let mut queue = use_signal(Vec::<reader::Track>::new);
    let current_queue_index = use_signal(|| 0usize);

    let mut ctrl = hooks::use_player_controller(
        player,
        is_playing,
        queue,
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
    );
    provide_context(ctrl);
    provide_context(config);

    hooks::use_player_task(ctrl);

    // Inject CSS for all custom themes reactively
    let custom_themes_css = use_memo(move || {
        config
            .read()
            .custom_themes
            .iter()
            .map(|(id, ct)| utils::themes::custom_theme_to_css(id, &ct.vars))
            .collect::<Vec<_>>()
            .join("\n\n")
    });

    use_effect(move || {
        let css = custom_themes_css.read().clone();
        // Serialize as a JSON string literal so no CSS content can escape the JS context
        let css_json = serde_json::to_string(&css).unwrap_or_else(|_| "\"\"".to_string());
        let _ = dioxus::document::eval(&format!(
            r#"(function(){{
                let el = document.getElementById('custom-themes-style');
                if (!el) {{ el = document.createElement('style'); el.id = 'custom-themes-style'; document.head.appendChild(el); }}
                el.textContent = {css_json};
            }})()"#
        ));
    });

    let theme_class = if config.read().theme == "album-art" {
        "theme-default".to_string()
    } else {
        format!("theme-{}", config.read().theme)
    };

    let background_style = if config.read().theme == "album-art" {
        utils::color::get_background_style(palette.read().as_deref())
    } else {
        "background-color: var(--color-black); background-image: none;".to_string()
    };
    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: THEME_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Link { rel: "stylesheet", href: REDUCED_ANIMATIONS_CSS }
        document::Link { rel: "stylesheet", href: "https://fonts.bunny.net/css?family=jetbrains-mono:400,500,700,800&display=swap" }
        document::Link { rel: "stylesheet", href: "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css" }
        div {
            class: "flex flex-col h-screen text-white select-none {theme_class}",
            style: "{background_style}",
            "data-reduce-animations": "{config.read().reduce_animations}",
            tabindex: "0",
            autofocus: true,
            onkeydown: move |evt| {
                use dioxus::prelude::Key;
                let key = evt.key();
                if key == Key::Character(" ".into()) {
                    ctrl.toggle();
                }
            },
            if cfg!(target_os = "linux") {
                Titlebar {}
            }
            div {
                class: "flex flex-1 overflow-hidden",
                Sidebar {
                    current_route,
                    on_navigate: move |route| {
                        if route == Route::Album {
                            selected_album_id.set(String::new());
                        }
                        if route == Route::Artist {
                            selected_artist_name.set(String::new());
                        }
                        current_route.set(route);
                    }
                }
                div {
                    id: "main-scroll-area",
                    class: "flex-1 overflow-y-auto",
                    match *current_route.read() {
                        Route::Home => rsx! {
                            pages::home::Home {
                                library,
                                playlist_store,
                                favorites_store,
                                on_select_album: move |id: String| {
                                    selected_album_id.set(id);
                                    current_route.set(Route::Album);
                                },
                                on_play_album: move |id: String| {
                                    selected_album_id.set(id.clone());

                                    let lib = library.peek();
                                    let is_jelly = id.starts_with("jellyfin:");
                                    let mut tracks: Vec<reader::Track> = if is_jelly {
                                        lib.jellyfin_tracks.iter().filter(|t| t.album_id == id).cloned().collect()
                                    } else {
                                        lib.tracks.iter().filter(|t| t.album_id == id).cloned().collect()
                                    };

                                    if !tracks.is_empty() {
                                        tracks.sort_by(|a, b| {
                                            let disc_cmp = a.disc_number.unwrap_or(1).cmp(&b.disc_number.unwrap_or(1));
                                            if disc_cmp == std::cmp::Ordering::Equal {
                                                a.track_number.unwrap_or(0).cmp(&b.track_number.unwrap_or(0))
                                            } else {
                                                disc_cmp
                                            }
                                        });
                                        queue.set(tracks);
                                        ctrl.play_track(0);
                                    }
                                    current_route.set(Route::Album);
                                },
                                on_select_playlist: move |id: String| {
                                    selected_playlist_id.set(Some(id));
                                    current_route.set(Route::Playlists);
                                },
                                on_search_artist: move |artist: String| {
                                    selected_artist_name.set(artist);
                                    current_route.set(Route::Artist);
                                }
                            }
                        },
                        Route::Search => rsx! {
                            pages::search::Search {
                                library: library,
                                config: config,
                                playlist_store: playlist_store,
                                search_query: search_query,
                                player: player,
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
                                config: config,
                                playlist_store: playlist_store,
                                on_rescan: move |_| *trigger_rescan.write() += 1,
                                player: player,
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
                                config: config,
                                album_id: selected_album_id,
                                playlist_store: playlist_store,
                                player: player,
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
                        Route::Artist => rsx! {
                            pages::artist::Artist {
                                library: library,
                                config: config,
                                artist_name: selected_artist_name,
                                playlist_store: playlist_store,
                                player: player,
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
                        Route::Favorites => rsx! {
                            pages::favorites::FavoritesPage {
                                favorites_store,
                                library,
                                config,
                                playlist_store,
                                player,
                                is_playing,
                                current_playing,
                                current_song_cover_url,
                                current_song_title,
                                current_song_artist,
                                current_song_duration,
                                current_song_progress,
                                queue,
                                current_queue_index,
                            }
                        },
                        Route::Playlists => rsx! {
                            pages::playlists::PlaylistsPage {
                                playlist_store: playlist_store,
                                library: library,
                                config: config,
                                player: player,
                                is_playing: is_playing,
                                current_playing: current_playing,
                                current_song_cover_url: current_song_cover_url,
                                current_song_title: current_song_title,
                                current_song_artist: current_song_artist,
                                current_song_duration: current_song_duration,
                                current_song_progress: current_song_progress,
                                queue: queue,
                                current_queue_index: current_queue_index,
                                selected_playlist_id: selected_playlist_id,
                            }
                        },
                        Route::Logs => rsx! {
                          pages::logs::Logs {
                              library: library,
                              config: config,
                          }
                        },
                        Route::Settings => rsx! { pages::settings::Settings { config } },
                        Route::ThemeEditor => rsx! { pages::theme_editor::ThemeEditorPage { config } },
                    }
                }
                Rightbar {
                    library: library,
                    is_rightbar_open: is_rightbar_open,
                    width: rightbar_width,
                    current_song_duration: current_song_duration,
                    current_song_progress: current_song_progress,
                    queue: queue,
                    current_queue_index: current_queue_index,
                    current_song_title: current_song_title,
                    current_song_artist: current_song_artist,
                    current_song_album: current_song_album,
                }
            }
            Fullscreen {
                library: library,
                player: player,
                is_playing: is_playing,
                is_fullscreen: is_fullscreen,
                current_song_duration: current_song_duration,
                current_song_progress: current_song_progress,
                queue: queue,
                current_song_album: current_song_album,
                current_queue_index: current_queue_index,
                current_song_title: current_song_title,
                current_song_khz: current_song_khz,
                current_song_bitrate: current_song_bitrate,
                current_song_artist: current_song_artist,
                current_song_cover_url: current_song_cover_url,
                volume: volume,
                palette: palette,
            }
            Bottombar {
                library: library,
                favorites_store,
                config,
                current_song_cover_url: current_song_cover_url,
                current_song_title: current_song_title,
                current_song_artist: current_song_artist,
                player: player,
                is_playing: is_playing,
                is_fullscreen: is_fullscreen,
                current_song_duration: current_song_duration,
                current_song_progress: current_song_progress,
                queue: queue,
                current_queue_index: current_queue_index,
                volume: volume,
                is_rightbar_open: is_rightbar_open,
            }
        }
    }
}
