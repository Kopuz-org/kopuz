#[cfg(target_arch = "wasm32")]
use crate::web_storage::{
    clear_web_queue_state, load_web_config, load_web_favorites, load_web_library,
    load_web_playlists, load_web_queue_state, load_web_ui_state, save_web_config,
    save_web_favorites, save_web_library, save_web_playlists, save_web_queue_state,
    save_web_ui_state,
};
use components::{
    bottombar::Bottombar, download_overlay::DownloadOverlay, fullscreen::Fullscreen,
    rightbar::Rightbar, sidebar::Sidebar, titlebar::Titlebar,
};
#[cfg(not(target_arch = "wasm32"))]
use dioxus::desktop::RequestAsyncResponder;
#[cfg(not(target_arch = "wasm32"))]
use dioxus::desktop::tao::dpi::LogicalSize;
#[cfg(not(target_arch = "wasm32"))]
use dioxus::desktop::tao::window::Icon;
#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
use dioxus::desktop::tao::platform::windows::WindowExtWindows;
#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
use windows::Win32::Foundation::HWND;
use dioxus::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use discord_presence::Presence;
use kopuz_route::Route;
use pages::server::download_manager::DownloadQueue;
use player::player::Player;
use queue_state::PersistedQueueState;
use reader::FavoritesStore;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod queue_state;
mod web_storage;
#[cfg(target_os = "windows")]
mod windows_titlebar;

#[cfg(not(target_arch = "wasm32"))]
fn migrate_legacy_locations() {
    let Some(dirs) = directories::ProjectDirs::from("com", "temidaradev", "kopuz") else {
        return;
    };
    let new_config = dirs.config_dir().to_path_buf();
    let sentinel = new_config.join(".migrated");
    if sentinel.exists() {
        return;
    }

    let old_cache = dirs.cache_dir().to_path_buf();
    let files = [
        "library.json",
        "playlists.json",
        "favorites.json",
        "queue_state.json",
    ];
    for file in files {
        let src = old_cache.join(file);
        let dst = new_config.join(file);
        if src.exists() && !dst.exists() {
            if let Err(e) = std::fs::rename(&src, &dst) {
                tracing::warn!("Failed to migrate {file} from cache to config: {e}");
            } else {
                tracing::info!("Migrated {file} to config dir");
            }
        }
    }

    let _ = std::fs::write(&sentinel, "");
}

const FAVICON: Asset = asset!("../assets/favicon.ico");
const MAIN_CSS: Asset = asset!("../assets/main.css");
const THEME_CSS: Asset = asset!("../assets/themes.css");
const TAILWIND_CSS: Asset = asset!("../assets/tailwind.css");
const REDUCED_ANIMATIONS_CSS: Asset = asset!("../assets/reduced-animations.css");
const QUEUE_STATE_SAVE_DEBOUNCE_MS: u64 = 1200;
const QUEUE_STATE_PROGRESS_STEP_SECS: u64 = 5;

#[cfg(not(target_arch = "wasm32"))]
static PRESENCE: std::sync::OnceLock<Option<Arc<Presence>>> = std::sync::OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
fn build_window_icon() -> Option<Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/logo-512.png")).ok()?;
    let image = image.into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).ok()
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct AvailableUpdate {
    version: String,
    release_url: String,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_version_parts(version: &str) -> Option<Vec<u64>> {
    let core = version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()
        .unwrap_or_default();
    let parts: Option<Vec<u64>> = core
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect();
    parts.filter(|parts| !parts.is_empty())
}

#[cfg(not(target_arch = "wasm32"))]
fn is_newer_version(current: &str, candidate: &str) -> bool {
    let Some(current_parts) = parse_version_parts(current) else {
        return false;
    };
    let Some(candidate_parts) = parse_version_parts(candidate) else {
        return false;
    };

    let max_len = current_parts.len().max(candidate_parts.len());
    for idx in 0..max_len {
        let current_part = *current_parts.get(idx).unwrap_or(&0);
        let candidate_part = *candidate_parts.get(idx).unwrap_or(&0);
        match candidate_part.cmp(&current_part) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => {}
        }
    }

    false
}

#[cfg(not(target_arch = "wasm32"))]
async fn fetch_available_update() -> Option<AvailableUpdate> {
    let client = reqwest::Client::builder()
        .user_agent(format!("kopuz/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;
    let release = client
        .get("https://api.github.com/repos/Kopuz-org/kopuz/releases/latest")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<GithubRelease>()
        .await
        .ok()?;

    if is_newer_version(env!("CARGO_PKG_VERSION"), &release.tag_name) {
        Some(AvailableUpdate {
            version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
            release_url: release.html_url,
        })
    } else {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn persist_config_snapshot(config_snapshot: config::AppConfig, path: std::path::PathBuf) {
    spawn(async move {
        let result = tokio::task::spawn_blocking(move || config_snapshot.save(&path)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!("Failed to save config: {}", e),
            Err(e) => tracing::error!("Failed to join config save task: {}", e),
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn persist_config_snapshot(config_snapshot: config::AppConfig, _path: std::path::PathBuf) {
    save_web_config(&config_snapshot);
}

#[cfg(not(target_arch = "wasm32"))]
async fn persist_queue_state_snapshot(
    queue_state: Option<PersistedQueueState>,
    path: std::path::PathBuf,
) {
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if let Some(queue_state) = queue_state {
            queue_state.save(&path)
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!("Failed to save queue state: {}", e),
        Err(e) => tracing::error!("Failed to join queue state save task: {}", e),
    }
}

#[cfg(target_arch = "wasm32")]
async fn persist_queue_state_snapshot(
    queue_state: Option<PersistedQueueState>,
    _path: std::path::PathBuf,
) {
    if let Some(queue_state) = queue_state {
        save_web_queue_state(&queue_state);
    } else {
        clear_web_queue_state();
    }
}

fn is_server_queue_track(track: &reader::Track) -> bool {
    matches!(
        track
            .path
            .to_string_lossy()
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "jellyfin" | "subsonic" | "custom"
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn is_restorable_queue_track(track: &reader::Track) -> bool {
    is_server_queue_track(track) || track.path.exists()
}

#[cfg(target_arch = "wasm32")]
fn is_restorable_queue_track(_track: &reader::Track) -> bool {
    true
}

fn sanitize_queue_state(state: PersistedQueueState) -> Option<PersistedQueueState> {
    if state.queue.is_empty() {
        return None;
    }

    let original_index = state
        .current_queue_index
        .min(state.queue.len().saturating_sub(1));
    let mut selected_track_survived = false;
    let survivors: Vec<(usize, reader::Track)> = state
        .queue
        .into_iter()
        .enumerate()
        .filter(|(idx, track)| {
            let keep = is_restorable_queue_track(track);
            if keep && *idx == original_index {
                selected_track_survived = true;
            }
            keep
        })
        .collect();

    if survivors.is_empty() {
        return None;
    }

    let restored_index = if selected_track_survived {
        survivors
            .iter()
            .position(|(idx, _)| *idx == original_index)
            .unwrap_or(0)
    } else {
        survivors
            .iter()
            .enumerate()
            .min_by_key(|(_, (idx, _))| (idx.abs_diff(original_index), *idx > original_index))
            .map(|(restored_idx, _)| restored_idx)
            .unwrap_or(0)
    };

    let old_queue_len = survivors
        .iter()
        .map(|(old_idx, _)| *old_idx)
        .max()
        .map_or(0, |m| m + 1);

    let mut old_to_new_index: Vec<Option<usize>> = vec![None; old_queue_len];
    for (new_idx, (old_idx, _)) in survivors.iter().enumerate() {
        old_to_new_index[*old_idx] = Some(new_idx);
    }

    let shuffle_order: Vec<usize> = state
        .shuffle_order
        .into_iter()
        .filter_map(|old_idx| old_to_new_index.get(old_idx).and_then(|&new_idx| new_idx))
        .collect();

    let queue: Vec<_> = survivors.into_iter().map(|(_, track)| track).collect();
    let progress_secs = if selected_track_survived {
        queue
            .get(restored_index)
            .map(|track| state.progress_secs.min(track.duration))
            .unwrap_or(0)
    } else {
        0
    };

    Some(PersistedQueueState {
        version: state.version,
        queue,
        current_queue_index: restored_index,
        progress_secs,
        shuffle_order,
        shuffle_enabled: state.shuffle_enabled,
    })
}

fn build_queue_state_snapshot(
    queue: &[reader::Track],
    current_queue_index: usize,
    current_song_progress: u64,
    is_playing: bool,
    shuffle_order: &[usize],
    shuffle_enabled: bool,
) -> Option<PersistedQueueState> {
    if queue.is_empty() {
        return None;
    }

    let current_idx = current_queue_index.min(queue.len() - 1);
    let progress_secs = queue
        .get(current_idx)
        .map(|track| current_song_progress.min(track.duration))
        .unwrap_or(0);
    let progress_secs = if is_playing {
        progress_secs - (progress_secs % QUEUE_STATE_PROGRESS_STEP_SECS)
    } else {
        progress_secs
    };

    Some(PersistedQueueState {
        version: 1,
        queue: queue.to_vec(),
        current_queue_index: current_idx,
        progress_secs,
        shuffle_order: shuffle_order.to_vec(),
        shuffle_enabled,
    })
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn read_titlebar_mode_from_disk() -> config::TitlebarMode {
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.config_dir().join("config.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<config::AppConfig>(&s).ok())
        .map(|c| c.titlebar_mode)
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn thumb_cache_path(file_path: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file_path.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir().join(format!("rusic_thumb_{:016x}.jpg", hash))
}

#[cfg(not(target_arch = "wasm32"))]
fn hq_cache_path(file_path: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "hq".hash(&mut hasher);
    file_path.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir().join(format!("rusic_hq_{:016x}.jpg", hash))
}

#[cfg(not(target_arch = "wasm32"))]
fn make_thumbnail(raw: &[u8], cache_path: &std::path::Path) -> Option<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    let img = image::load_from_memory(raw).ok()?;
    const MAX: u32 = 400;
    let img = if img.width() > MAX || img.height() > MAX {
        img.thumbnail(MAX, MAX)
    } else {
        img
    };
    let mut out: Vec<u8> = Vec::new();
    img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, 75))
        .ok()?;
    let _ = std::fs::write(cache_path, &out);
    Some(out)
}

// Returns Some(compressed) when the image exceeded the size/dimension limit,
// or None when the original is already small enough to serve as-is.
#[cfg(not(target_arch = "wasm32"))]
fn make_hq_image(raw: &[u8], cache_path: &std::path::Path) -> Option<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    const SIZE_LIMIT: usize = 2 * 1024 * 1024; // 2 MB
    const MAX_DIM: u32 = 1920;
    const QUALITY: u8 = 85;

    if raw.len() <= SIZE_LIMIT {
        return None;
    }
    let img = image::load_from_memory(raw).ok()?;
    let img = if img.width() > MAX_DIM || img.height() > MAX_DIM {
        img.thumbnail(MAX_DIM, MAX_DIM)
    } else {
        img
    };
    let mut out: Vec<u8> = Vec::new();
    img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, QUALITY))
        .ok()?;
    let _ = std::fs::write(cache_path, &out);
    Some(out)
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let log_dir = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
            .map(|dirs| dirs.cache_dir().join("logs"))
            .unwrap_or_else(|| std::path::PathBuf::from("logs"));
        let _ = std::fs::create_dir_all(&log_dir);

        let file_appender = tracing_appender::rolling::daily(&log_dir, "kopuz.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(non_blocking),
            )
            .init();
        tracing::info!("Log file: {}", log_dir.display());

        migrate_legacy_locations();

        let presence: Option<Arc<Presence>> = match Presence::new("1470087339639443658") {
            Ok(p) => {
                tracing::info!("Discord presence connected");
                Some(Arc::new(p))
            }
            Err(e) => {
                tracing::warn!("Failed to connect to Discord: {e}");
                None
            }
        };

        PRESENCE.set(presence).ok();

        #[cfg(target_os = "macos")]
        {
            player::systemint::init();
        }

        let mut window = dioxus::desktop::WindowBuilder::new()
            .with_title("Kopuz")
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(1350.0, 800.0));

        if let Some(icon) = build_window_icon() {
            window = window.with_window_icon(Some(icon));
        }

        #[cfg(target_os = "macos")]
        {
            window = window
                .with_title_hidden(true)
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true);
        }

        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let initial_titlebar_mode = read_titlebar_mode_from_disk();
            window = window.with_decorations(initial_titlebar_mode == config::TitlebarMode::System);
        }

        let webview_data_dir = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
            .map(|dirs| dirs.cache_dir().join("webview"))
            .unwrap_or_else(|| std::path::PathBuf::from("./cache/webview"));
        let _ = std::fs::create_dir_all(&webview_data_dir);

        let config = dioxus::desktop::Config::new()
            .with_custom_head(
                "<style>html,body{background:#000;margin:0;padding:0}body{opacity:0}</style>"
                    .to_string(),
            )
            .with_data_directory(webview_data_dir)
            .with_window(window)
            .with_asynchronous_custom_protocol(
                "artwork",
                |_id, request, responder: RequestAsyncResponder| {
                    let uri = request.uri().clone();

                    tokio::spawn(async move {
                        let query = uri.query().unwrap_or_default();
                        let file_path: String = query
                            .split('&')
                            .find_map(|kv| kv.strip_prefix("p="))
                            .map(|encoded| {
                                percent_encoding::percent_decode_str(encoded)
                                    .decode_utf8_lossy()
                                    .into_owned()
                            })
                            .unwrap_or_default();
                        let high_quality = query.split('&').any(|kv| kv == "hq=1");

                        if file_path.is_empty() {
                            responder.respond(
                                http::Response::builder()
                                    .status(400)
                                    .body(std::borrow::Cow::from(Vec::new()))
                                    .unwrap(),
                            );
                            return;
                        }

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

                        if high_quality {
                            let hq_path = hq_cache_path(&file_path);
                            if hq_path.exists() {
                                if let Ok(b) = tokio::fs::read(&hq_path).await {
                                    responder.respond(
                                        http::Response::builder()
                                            .header("Content-Type", "image/jpeg")
                                            .header("Access-Control-Allow-Origin", "*")
                                            .header("Cache-Control", "public, max-age=31536000")
                                            .body(std::borrow::Cow::from(b))
                                            .unwrap(),
                                    );
                                    return;
                                }
                            }
                            match tokio::fs::read(&file_path).await {
                                Ok(raw) => {
                                    let file_path_clone = file_path.clone();
                                    let result = tokio::task::spawn_blocking(move || {
                                        make_hq_image(&raw, &hq_path)
                                            .map(|b| (b, "image/jpeg"))
                                            .unwrap_or_else(|| {
                                                let mime = if file_path_clone.ends_with(".png") {
                                                    "image/png"
                                                } else {
                                                    "image/jpeg"
                                                };
                                                (raw, mime)
                                            })
                                    })
                                    .await;
                                    match result {
                                        Ok((bytes, mime)) => responder.respond(
                                            http::Response::builder()
                                                .header("Content-Type", mime)
                                                .header("Access-Control-Allow-Origin", "*")
                                                .header("Cache-Control", "public, max-age=31536000")
                                                .body(std::borrow::Cow::from(bytes))
                                                .unwrap(),
                                        ),
                                        Err(_) => responder.respond(
                                            http::Response::builder()
                                                .status(500)
                                                .body(std::borrow::Cow::from(Vec::new()))
                                                .unwrap(),
                                        ),
                                    }
                                }
                                Err(_) => responder.respond(
                                    http::Response::builder()
                                        .status(404)
                                        .body(std::borrow::Cow::from(Vec::new()))
                                        .unwrap(),
                                ),
                            }
                            return;
                        }

                        let thumb_path = thumb_cache_path(&file_path);

                        let (bytes, mime) = if thumb_path.exists() {
                            match tokio::fs::read(&thumb_path).await {
                                Ok(b) => (b, "image/jpeg"),
                                Err(_) => {
                                    let _ = std::fs::remove_file(&thumb_path);
                                    match tokio::fs::read(&file_path).await {
                                        Ok(b) => (
                                            b,
                                            if file_path.ends_with(".png") {
                                                "image/png"
                                            } else {
                                                "image/jpeg"
                                            },
                                        ),
                                        Err(_) => {
                                            responder.respond(
                                                http::Response::builder()
                                                    .status(404)
                                                    .body(std::borrow::Cow::from(Vec::new()))
                                                    .unwrap(),
                                            );
                                            return;
                                        }
                                    }
                                }
                            }
                        } else {
                            match tokio::fs::read(&file_path).await {
                                Ok(raw) => {
                                    let thumb_path_clone = thumb_path.clone();
                                    match tokio::task::spawn_blocking(move || match make_thumbnail(
                                        &raw,
                                        &thumb_path_clone,
                                    ) {
                                        Some(b) => Ok(b),
                                        None => Err(raw),
                                    })
                                    .await
                                    {
                                        Ok(Ok(b)) => (b, "image/jpeg"),
                                        Ok(Err(raw)) => (
                                            raw,
                                            if file_path.ends_with(".png") {
                                                "image/png"
                                            } else {
                                                "image/jpeg"
                                            },
                                        ),
                                        Err(_) => {
                                            responder.respond(
                                                http::Response::builder()
                                                    .status(500)
                                                    .body(std::borrow::Cow::from(Vec::new()))
                                                    .unwrap(),
                                            );
                                            return;
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("[artwork] not found {}: {}", file_path, e);
                                    responder.respond(
                                        http::Response::builder()
                                            .status(404)
                                            .body(std::borrow::Cow::from(Vec::new()))
                                            .unwrap(),
                                    );
                                    return;
                                }
                            }
                        };

                        responder.respond(
                            http::Response::builder()
                                .header("Content-Type", mime)
                                .header("Access-Control-Allow-Origin", "*")
                                .header("Cache-Control", "public, max-age=31536000")
                                .body(std::borrow::Cow::from(bytes))
                                .unwrap(),
                        );
                    });
                },
            );

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
    let mut scroll_positions: Signal<std::collections::HashMap<Route, f64>> =
        use_signal(std::collections::HashMap::new);
    let cache_dir = use_memo(move || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
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
            let path = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| std::path::Path::new("./config").to_path_buf());
            let _ = std::fs::create_dir_all(&path);
            path
        }
        #[cfg(target_arch = "wasm32")]
        std::path::PathBuf::from("./config")
    });
    let lib_path = use_memo(move || config_dir().join("library.json"));
    let config_path = use_memo(move || config_dir().join("config.json"));
    let mut config = use_signal(config::AppConfig::default);
    #[allow(unused_variables)]
    let playlist_path = use_memo(move || config_dir().join("playlists.json"));
    let mut playlist_store = use_signal(reader::PlaylistStore::default);
    #[allow(unused_variables)]
    let favorites_path = use_memo(move || config_dir().join("favorites.json"));
    let queue_state_path = use_memo(move || config_dir().join("queue_state.json"));
    let mut favorites_store = use_signal(FavoritesStore::default);
    let mut initial_load_done = use_signal(|| false);
    #[allow(unused_variables)]
    let cover_cache = use_memo(move || cache_dir().join("covers"));
    #[cfg(not(target_arch = "wasm32"))]
    let _ = std::fs::create_dir_all(cover_cache());
    let download_queue = use_signal(DownloadQueue::default);
    let mut trigger_rescan = use_signal(|| 0);
    let mut scan_current_file = use_signal(|| Option::<String>::None);
    let current_playing = use_signal(|| 0);
    let mut player = use_signal(Player::new);
    let current_song_cover_url = use_signal(String::new);
    let current_song_title = use_signal(String::new);
    let current_song_artist = use_signal(String::new);
    let current_song_album = use_signal(String::new);
    let current_song_duration = use_signal(|| 0u64);
    let current_song_khz = use_signal(|| 0u32);
    let current_song_bitrate = use_signal(|| 0u16);
    let current_song_progress = use_signal(|| 0u64);
    let current_track_snapshot = use_signal(|| None::<reader::Track>);
    let mut volume = use_signal(|| 1.0f32);
    let mut persisted_volume = use_signal(|| 1.0f32);
    let mut configured_music_dirs = use_signal(|| config.peek().music_directory.clone());

    let is_playing = use_signal(|| false);
    let mut is_fullscreen = use_signal(|| false);
    let is_rightbar_open = use_signal(|| false);
    let rightbar_width = use_signal(|| 320usize);
    let mut palette = use_signal(|| Option::<Vec<utils::color::Color>>::None);
    let mut pending_queue_state_snapshot = use_signal(|| None::<PersistedQueueState>);
    let mut pending_queue_state_revision = use_signal(|| 0u64);

    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    use_effect(move || {
        let _ = dioxus::document::eval(
            r#"(function(){
            try {
                var ctx = new (window.AudioContext||window.webkitAudioContext)({sampleRate:8000});
                var buf = ctx.createBuffer(1,1,8000);
                var src = ctx.createBufferSource();
                src.buffer = buf;
                src.loop = true;
                src.connect(ctx.destination);
                src.start(0);
                document.addEventListener('visibilitychange', function(){
                    if (ctx.state === 'suspended') ctx.resume();
                });
            } catch(e) {}
        })()"#,
        );
    });

    use_effect(move || {
        let _ = dioxus::document::eval(
            r#"(function(){
                function show(){document.body.style.transition='opacity .15s';document.body.style.opacity='1';}
                var links=document.querySelectorAll('link[rel="stylesheet"]');
                if(!links.length){show();return;}
                var loaded=0;
                function onLoad(){if(++loaded>=links.length)show();}
                links.forEach(function(l){if(l.sheet){onLoad();}else{l.addEventListener('load',onLoad);l.addEventListener('error',onLoad);}});
            })();"#,
        );
    });

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

    use_effect(move || {
        let next_dirs = config.read().music_directory.clone();
        if *configured_music_dirs.peek() != next_dirs {
            configured_music_dirs.set(next_dirs);
        }
    });

    #[cfg(not(target_arch = "wasm32"))]
    let presence = PRESENCE.get().cloned().flatten();
    #[cfg(not(target_arch = "wasm32"))]
    provide_context(presence.clone());

    let mut selected_album_id = use_signal(String::new);
    let mut selected_playlist_id = use_signal(|| None::<String>);
    let mut selected_artist_name = use_signal(String::new);
    let fetched_artist_images: Signal<std::collections::HashMap<String, String>> =
        use_signal(std::collections::HashMap::new);
    let is_fetching_artist_images = use_signal(|| false);
    let search_query = use_signal(String::new);
    let mut last_server_playlist_key = use_signal(|| None::<String>);
    let mut server_playlist_key_initialized = use_signal(|| false);
    let mut queue = use_signal(Vec::<reader::Track>::new);
    let current_queue_index = use_signal(|| 0usize);

    let mut network_banner: Signal<Option<bool>> = use_signal(|| None);
    #[cfg(not(target_arch = "wasm32"))]
    let mut update_banner: Signal<Option<AvailableUpdate>> = use_signal(|| None);
    #[cfg(not(target_arch = "wasm32"))]
    let mut did_check_updates = use_signal(|| false);
    let mut auto_switched_to_offline = use_signal(|| false);
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
        current_track_snapshot,
        volume,
        library,
        config,
    );

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

    #[cfg(not(target_arch = "wasm32"))]
    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        if !config.read().auto_check_updates {
            update_banner.set(None);
            if *did_check_updates.peek() {
                did_check_updates.set(false);
            }
            return;
        }

        if *did_check_updates.read() {
            return;
        }

        did_check_updates.set(true);
        spawn(async move {
            if let Some(update) = fetch_available_update().await {
                update_banner.set(Some(update));
            }
        });
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }
        let mut config_snapshot = config.read().clone();
        config_snapshot.volume = *volume.peek();
        persist_config_snapshot(config_snapshot, config_path());
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        let committed_volume = *persisted_volume.read();
        let mut config_snapshot = config.peek().clone();
        config_snapshot.volume = committed_volume;
        persist_config_snapshot(config_snapshot, config_path());
    });

    #[cfg(all(
        not(target_arch = "wasm32"),
        any(target_os = "linux", target_os = "windows")
    ))]
    use_effect(move || {
        let mode = config.read().titlebar_mode;
        let win = dioxus::desktop::use_window();
        win.set_decorations(mode == config::TitlebarMode::System);
    });

    #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
    use_effect(move || {
        let mode = config.read().titlebar_mode;
        let win = dioxus::desktop::use_window();
        let hwnd = HWND(win.window.hwnd() as _);
        windows_titlebar::install(hwnd);
        windows_titlebar::set_custom_titlebar_enabled(mode == config::TitlebarMode::Custom);
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
                    tracing::error!("Failed to save playlists: {}", e);
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
                    tracing::error!("Failed to save library: {}", e);
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
                    tracing::error!("Failed to save favorites: {}", e);
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let store_snapshot = favorites_store.read().clone();
            save_web_favorites(&store_snapshot);
        }
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        let queue_snapshot = queue.read().clone();
        let shuffle_order_snapshot = ctrl.shuffle_order.read().clone();
        let shuffle_enabled_snapshot = *ctrl.shuffle.read();

        let queue_state = build_queue_state_snapshot(
            &queue_snapshot,
            *current_queue_index.read(),
            *current_song_progress.read(),
            *is_playing.read(),
            &shuffle_order_snapshot,
            shuffle_enabled_snapshot,
        );

        if *pending_queue_state_snapshot.peek() != queue_state {
            pending_queue_state_snapshot.set(queue_state);
            pending_queue_state_revision.with_mut(|revision| *revision += 1);
        }
    });

    use_future(move || {
        let path = queue_state_path();
        async move {
            let mut flushed_revision = 0u64;

            loop {
                let pending_revision = *pending_queue_state_revision.read();
                if pending_revision == flushed_revision {
                    utils::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }

                utils::sleep(std::time::Duration::from_millis(
                    QUEUE_STATE_SAVE_DEBOUNCE_MS,
                ))
                .await;

                let latest_revision = *pending_queue_state_revision.read();
                if latest_revision != pending_revision {
                    continue;
                }

                let snapshot = pending_queue_state_snapshot.read().clone();
                persist_queue_state_snapshot(snapshot, path.clone()).await;
                flushed_revision = latest_revision;
            }
        }
    });

    let mut is_offline = use_signal(|| false);
    use_context_provider(|| is_offline);

    // Network connectivity monitor — only active in server mode and on non-wasm targets
    #[cfg(not(target_arch = "wasm32"))]
    use_future(move || async move {
        loop {
            if *initial_load_done.read() {
                break;
            }
            utils::sleep(std::time::Duration::from_millis(500)).await;
        }
        let mut was_reachable = true;
        let mut consecutive_failures: u8 = 0;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build();
        let Ok(client) = client else { return };
        loop {
            utils::sleep(std::time::Duration::from_secs(30)).await;

            let server_url = {
                let conf = config.read();
                if conf.active_source != config::MusicSource::Server {
                    was_reachable = true;
                    consecutive_failures = 0;
                    continue;
                }
                conf.server.as_ref().map(|s| s.url.clone())
            };

            let Some(base_url) = server_url else {
                was_reachable = true;
                consecutive_failures = 0;
                continue;
            };

            let ping_url = format!("{}/System/Ping", base_url.trim_end_matches('/'));
            let reachable = client
                .get(&ping_url)
                .send()
                .await
                .map(|r| r.status().as_u16() < 500)
                .unwrap_or(false);

            if reachable {
                consecutive_failures = 0;
            } else {
                consecutive_failures = consecutive_failures.saturating_add(1);
            }

            if !reachable && consecutive_failures >= 2 && was_reachable {
                was_reachable = false;
                is_offline.set(true);
                auto_switched_to_offline.set(true);
                config.write().active_source = config::MusicSource::Local;
                network_banner.set(Some(true));
            } else if reachable && !was_reachable {
                was_reachable = true;
                consecutive_failures = 0;
                is_offline.set(false);
                if *auto_switched_to_offline.read() {
                    auto_switched_to_offline.set(false);
                    config.write().active_source = config::MusicSource::Server;
                    network_banner.set(Some(false));
                    spawn(async move {
                        utils::sleep(std::time::Duration::from_secs(4)).await;
                        if network_banner.read().as_ref() == Some(&false) {
                            network_banner.set(None);
                        }
                    });
                }
            }
        }
    });

    use_hook(move || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let lib_path = lib_path();
            let config_path = config_path();
            let playlist_path = playlist_path();
            let favorites_path = favorites_path();
            let queue_state_path = queue_state_path();
            let mut ctrl = ctrl;

            spawn(async move {
                let lib_path_c = lib_path.clone();
                let config_path_c = config_path.clone();
                let playlist_path_c = playlist_path.clone();
                let favorites_path_c = favorites_path.clone();
                let queue_state_path_c = queue_state_path.clone();

                let (lib_res, cfg_res, pl_res, fav_res, queue_res) = tokio::join!(
                    tokio::task::spawn_blocking(move || reader::Library::load(&lib_path_c)),
                    tokio::task::spawn_blocking(move || config::AppConfig::load(&config_path_c)),
                    tokio::task::spawn_blocking(move || reader::PlaylistStore::load(
                        &playlist_path_c
                    )),
                    tokio::task::spawn_blocking(move || FavoritesStore::load(&favorites_path_c)),
                    tokio::task::spawn_blocking(move || {
                        PersistedQueueState::load(&queue_state_path_c)
                    }),
                );

                if let Ok(Ok(loaded)) = lib_res {
                    library.set(loaded.clone());
                }
                if let Ok(loaded) = cfg_res {
                    config.set(loaded.clone());
                    configured_music_dirs.set(loaded.music_directory.clone());
                    volume.set(loaded.volume);
                    persisted_volume.set(loaded.volume);
                    player.write().set_volume(loaded.volume);
                    player.write().set_channel_mode(loaded.channel_mode);
                    player.write().set_equalizer(loaded.equalizer.clone());
                    i18n::set_locale(&loaded.language);
                }
                if let Ok(Ok(loaded)) = pl_res {
                    playlist_store.set(loaded);
                }
                if let Ok(Ok(loaded)) = fav_res {
                    favorites_store.set(loaded);
                }

                {
                    let cfg = config.peek();
                    let no_local_tracks = library.peek().tracks.is_empty();
                    let server_connected = cfg
                        .server
                        .as_ref()
                        .and_then(|s| s.access_token.as_ref())
                        .is_some();
                    let not_explicitly_set = !cfg.source_explicitly_set;
                    drop(cfg);
                    if no_local_tracks && server_connected && not_explicitly_set {
                        config.write().active_source = config::MusicSource::Server;
                    }
                }

                if let Ok(Ok(loaded_queue_state)) = queue_res {
                    if let Some(queue_state) = sanitize_queue_state(loaded_queue_state) {
                        ctrl.restore_queue_state(
                            queue_state.queue,
                            queue_state.current_queue_index,
                            queue_state.progress_secs,
                            queue_state.shuffle_order,
                            queue_state.shuffle_enabled,
                        );
                    }
                }

                initial_load_done.set(true);
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let mut ctrl = ctrl;
            let mut loaded = load_web_config().unwrap_or_default();
            if loaded.server.is_none() {
                loaded.active_source = config::MusicSource::Server;
            }
            let loaded_volume = loaded.volume;
            let loaded_language = loaded.language.clone();
            configured_music_dirs.set(loaded.music_directory.clone());
            config.set(loaded);
            volume.set(loaded_volume);
            persisted_volume.set(loaded_volume);
            player.write().set_volume(loaded_volume);
            player.write().set_channel_mode(config.read().channel_mode);
            player
                .write()
                .set_equalizer(config.read().equalizer.clone());
            i18n::set_locale(&loaded_language);

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
            if let Some(loaded_queue_state) = load_web_queue_state() {
                if let Some(queue_state) = sanitize_queue_state(loaded_queue_state) {
                    ctrl.restore_queue_state(
                        queue_state.queue,
                        queue_state.current_queue_index,
                        queue_state.progress_secs,
                        queue_state.shuffle_order,
                        queue_state.shuffle_enabled,
                    );
                }
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
        let configured_dirs = configured_music_dirs.read().clone();
        let _ = trigger_rescan.read();

        #[cfg(not(target_arch = "wasm32"))]
        spawn(async move {
            let scannable_dirs: Vec<PathBuf> = configured_dirs
                .iter()
                .filter(|d| d.exists())
                .cloned()
                .collect();
            let mut current_lib = library.peek().clone();

            let current_roots: std::collections::HashSet<_> =
                current_lib.root_paths.iter().cloned().collect();
            let new_roots: std::collections::HashSet<_> = configured_dirs.iter().cloned().collect();

            if current_roots != new_roots {
                current_lib.root_paths = configured_dirs.clone();
                current_lib.tracks.clear();
                current_lib.albums.clear();
                library.set(current_lib.clone());
            }

            if !configured_dirs.is_empty() {
                current_lib.local_artist_images.clear();
                scan_current_file.set(Some(String::new()));

                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                spawn(async move {
                    while let Some(file) = rx.recv().await {
                        scan_current_file.set(Some(file));
                    }
                    scan_current_file.set(None);
                });

                let progress_cb: std::sync::Arc<dyn Fn(String) + Send + Sync> =
                    std::sync::Arc::new(move |file: String| {
                        let _ = tx.send(file);
                    });
                for dir in &scannable_dirs {
                    let _ = reader::scan_directory(
                        dir.clone(),
                        cover_cache(),
                        &mut current_lib,
                        progress_cb.clone(),
                    )
                    .await;
                }
                drop(progress_cb);

                current_lib.tracks.retain(|t| {
                    let in_configured_root = configured_dirs.iter().any(|d| t.path.starts_with(d));
                    let in_scannable_root = scannable_dirs.iter().any(|d| t.path.starts_with(d));

                    in_configured_root && (!in_scannable_root || t.path.exists())
                });

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
            } else {
                current_lib.tracks.clear();
                current_lib.albums.clear();
                current_lib.root_paths.clear();
                library.set(current_lib.clone());
                let _ = current_lib.save(&lib_path());
            }
        });
    });

    use_effect(move || {
        let route = *current_route.read();
        let pos = scroll_positions.peek().get(&route).copied().unwrap_or(0.0);
        let _ = dioxus::document::eval(&format!(
            "let el = document.getElementById('main-scroll-area'); if (el) el.scrollTop = {pos};"
        ));
    });

    provide_context(ctrl);
    provide_context(config);
    provide_context(download_queue);
    provide_context(scroll_positions);
    provide_context(fetched_artist_images);
    provide_context(is_fetching_artist_images);
    provide_context(components::NavigationController {
        current_route,
        selected_artist_name,
        selected_album_id,
    });

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

    let theme_class = use_memo(move || {
        if config.read().theme == "album-art" {
            "theme-default".to_string()
        } else {
            format!("theme-{}", config.read().theme)
        }
    });

    let is_rtl = i18n::is_rtl();
    let dir = if is_rtl { "rtl" } else { "ltr" };
    let content_row_class = "flex flex-1 overflow-hidden";
    #[cfg(not(target_arch = "wasm32"))]
    let update_banner_state = update_banner.read().clone();

    let background_style = use_memo(move || {
        if config.read().theme == "album-art" {
            utils::color::get_background_style(palette.read().as_deref())
        } else {
            "background-color: var(--color-black); background-image: none;".to_string()
        }
    });

    let reduce_animations = use_memo(move || config.read().reduce_animations);
    let active_source = use_memo(move || config.read().active_source);

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: THEME_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Link { rel: "stylesheet", href: REDUCED_ANIMATIONS_CSS }
        document::Script {
            "(function(){{
                ['https://fonts.bunny.net/css?family=jetbrains-mono:400,500,700,800&display=swap',
                 'https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css']
                .forEach(function(href){{
                    var l=document.createElement('link');
                    l.rel='stylesheet';l.href=href;
                    document.head.appendChild(l);
                }});
            }})();"
        }
        div {
            class: "flex flex-col h-screen text-white select-none {theme_class}",
            style: "{background_style}",
            dir: "{dir}",
            "data-reduce-animations": "{reduce_animations}",
            tabindex: "0",
            autofocus: true,
            onkeydown: move |evt| {
                use dioxus::prelude::Key;
                let key = evt.key();
                if key == Key::Escape {
                    is_fullscreen.set(false);
                } else if key == Key::Character(" ".into()) {
                    ctrl.toggle();
                }
            },
            if cfg!(any(target_os = "linux", target_os = "windows")) {
                div { dir: "ltr", Titlebar {} }
            }

            if active_source == config::MusicSource::Local {
                if let Some(file) = scan_current_file.read().clone() {
                    div {
                        class: "flex-shrink-0",
                        div {
                            class: "h-[2px] bg-white/5 overflow-hidden",
                            div { class: "h-full w-1/4 bg-[var(--color-primary,#6366f1)] animate-scan" }
                        }
                        div {
                            class: "px-3 py-[3px] flex items-center gap-2 bg-black/30 border-b border-white/5",
                            i { class: "fa-solid fa-compact-disc fa-spin text-[9px] text-white/30 flex-shrink-0" }
                            span {
                                class: "text-[10px] text-white/35 font-mono truncate",
                                if file.is_empty() {
                                    "Scanning library…"
                                } else {
                                    "{file}"
                                }
                            }
                        }
                    }
                }
            }

            if let Some(is_offline) = *network_banner.read() {
                div {
                    class: "flex-shrink-0",
                    div {
                        class: if is_offline {
                            "flex items-center justify-between gap-3 px-4 py-2 bg-amber-500/15 border-b border-amber-500/20 text-amber-300 text-sm"
                        } else {
                            "flex items-center justify-between gap-3 px-4 py-2 bg-emerald-500/15 border-b border-emerald-500/20 text-emerald-300 text-sm"
                        },
                        div {
                            class: "flex items-center gap-2",
                            i { class: if is_offline { "fa-solid fa-wifi-slash text-xs" } else { "fa-solid fa-wifi text-xs" } }
                            span {
                                if is_offline {
                                    "No internet connection — switched to offline mode"
                                } else {
                                    "Back online — switched to server mode"
                                }
                            }
                            if is_offline {
                                button {
                                    class: "ml-2 text-xs underline opacity-70 hover:opacity-100 transition-opacity",
                                    onclick: move |_| {
                                        config.write().active_source = config::MusicSource::Server;
                                        network_banner.set(None);
                                    },
                                    "Keep server mode"
                                }
                            }
                        }
                        button {
                            class: "opacity-50 hover:opacity-100 transition-opacity p-1",
                            onclick: move |_| network_banner.set(None),
                            i { class: "fa-solid fa-xmark text-xs" }
                        }
                    }
                }
            }

            if let Some(update) = {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    update_banner_state.clone()
                }
                #[cfg(target_arch = "wasm32")]
                {
                    None
                }
            } {
                div {
                    class: "flex-shrink-0",
                    div {
                        class: "flex items-center justify-between gap-3 px-4 py-2 bg-sky-500/15 border-b border-sky-500/20 text-sky-200 text-sm",
                        div {
                            class: "flex items-center gap-2",
                            i { class: "fa-solid fa-download text-xs" }
                            span { class: "font-medium", "{i18n::t(\"update_available\")} - " }
                            span { "{i18n::t_with(\"update_banner_message\", &[(\"version\", update.version.clone())])}" }
                            button {
                                class: "ml-2 text-xs underline opacity-80 hover:opacity-100 transition-opacity",
                                onclick: {
                                    let release_url = update.release_url.clone();
                                    move |_| {
                                        if let Err(e) = webbrowser::open(&release_url) {
                                            tracing::error!("Failed to open release page: {}", e);
                                        }
                                    }
                                },
                                "{i18n::t(\"view_release\")}"
                            }
                        }
                        button {
                            class: "opacity-50 hover:opacity-100 transition-opacity p-1",
                            onclick: move |_| update_banner.set(None),
                            i { class: "fa-solid fa-xmark text-xs" }
                        }
                    }
                }
            }

            if config.read().player_bar_position == config::PlayerBarPosition::Top {
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
                    persisted_volume: persisted_volume,
                    is_rightbar_open: is_rightbar_open,
                }
            }
            div {
                class: "{content_row_class}",
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
                    onscroll: move |evt| {
                        let pos = evt.scroll_top();
                        scroll_positions.write().insert(*current_route.peek(), pos);
                    },
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
                                on_select_album: move |id: String| {
                                    selected_album_id.set(id);
                                    current_route.set(Route::Album);
                                },
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
                                on_navigate: move |album_id| {
                                    selected_album_id.set(album_id);
                                    current_route.set(Route::Album);
                                },
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
                                selected_playlist_id: selected_playlist_id,
                            }
                        },
                        Route::Activity => rsx! {
                          pages::activity::Activity {
                              library: library,
                              config: config,
                          }
                        },
                        Route::Radio => rsx! {
                            pages::radio::Radio {
                                config: config,
                            }
                        },
                        #[cfg(not(target_arch = "wasm32"))]
                        Route::Ytdlp => rsx! { pages::ytdlp::YtdlpPage { config, trigger_rescan } },
                        #[cfg(target_arch = "wasm32")]
                        Route::Ytdlp => rsx! { pages::settings::Settings { config } },
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
                persisted_volume: persisted_volume,
                palette: palette,
            }
            DownloadOverlay { queue: download_queue }
            if config.read().player_bar_position == config::PlayerBarPosition::Bottom {
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
                    persisted_volume: persisted_volume,
                    is_rightbar_open: is_rightbar_open,
                }
            }
        }
    }
}
