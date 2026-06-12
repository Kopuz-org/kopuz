use components::{
    bottombar::Bottombar, download_overlay::DownloadOverlay, fullscreen::Fullscreen,
    rightbar::Rightbar, sidebar::Sidebar, titlebar::Titlebar,
};
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use dioxus::desktop::RequestAsyncResponder;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use dioxus::desktop::tao::dpi::LogicalSize;
#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
use dioxus::desktop::tao::platform::windows::WindowExtWindows;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use dioxus::desktop::tao::window::Icon;
use dioxus::prelude::*;
use tracing::Instrument;
#[cfg(not(target_arch = "wasm32"))]
use discord_presence::Presence;
use kopuz_route::Route;
use pages::server::download_manager::DownloadQueue;
use player::player::Player;
use queue_state::PersistedQueueState;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
use windows::Win32::Foundation::HWND;

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
mod chrome_trace;
mod logging;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod pot_minter;
mod queue_state;
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
#[cfg(target_os = "windows")]
const TOOLBAR_ICONS: Asset = asset!("../assets/toolbar_icons", AssetOptions::folder());
const QUEUE_STATE_SAVE_DEBOUNCE_MS: u64 = 1200;
const QUEUE_STATE_PROGRESS_STEP_SECS: u64 = 5;
/// Store saves (config/library/playlists/favorites) are full-replace and
/// expensive; bursts of mutations (batch downloads, syncs) coalesce into one
/// save per settle+cooldown window instead of one per mutation.
const STORE_SAVE_SETTLE_MS: u64 = 600;
const STORE_SAVE_COOLDOWN_MS: u64 = 2500;

#[cfg(target_os = "windows")]
#[component]
fn WindowsToolbarIconAssets() -> Element {
    rsx! {
        div {
            hidden: true,
            "data-toolbar-icons": "{TOOLBAR_ICONS}",
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[component]
fn WindowsToolbarIconAssets() -> Element {
    rsx! {}
}

#[cfg(not(target_arch = "wasm32"))]
static PRESENCE: std::sync::OnceLock<Option<Arc<Presence>>> = std::sync::OnceLock::new();

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
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
async fn run_rotation(mut config: Signal<config::AppConfig>) {
    let cookies = match config.peek().server.as_ref() {
        Some(s) if s.service == config::MusicService::YtMusic => s.access_token.clone(),
        _ => return,
    };
    let Some(cookies) = cookies else { return };
    // Anonymous YT carries an empty token — nothing to keep alive.
    if cookies.is_empty() {
        return;
    }
    let started = std::time::Instant::now();
    // Logging policy: noisy in the rare cases (jar rotated, error),
    // silent on the steady-state OK-no-change tick. The keepalive
    // fires every 5 min and 99% of ticks are no-change; the original
    // per-tick OK line drowned stderr.
    match server::ytmusic::verify_session_keepalive::tick(&cookies).await {
        Ok(Some(updated)) => {
            tracing::debug!(
                secs = started.elapsed().as_secs_f32(),
                from = cookies.len(),
                to = updated.len(),
                "verify_session OK — jar rotated",
            );
            if let Some(srv) = config.write().server.as_mut() {
                srv.access_token = Some(updated);
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "verify_session failed"),
    }
}

async fn persist_queue_state_snapshot(db: db::Db, queue_state: Option<PersistedQueueState>) {
    let snap = queue_state.map(queue_snapshot).unwrap_or_default();
    if let Err(e) = db.save_queue(&snap).await {
        tracing::error!("Failed to save queue state: {}", e);
    }
}

fn queue_snapshot(q: PersistedQueueState) -> db::QueueSnapshot {
    db::QueueSnapshot {
        version: q.version,
        queue: q.queue,
        current_queue_index: q.current_queue_index,
        progress_secs: q.progress_secs,
        shuffle_order: q.shuffle_order,
        shuffle_enabled: q.shuffle_enabled,
    }
}

fn is_server_queue_track(track: &reader::Track) -> bool {
    matches!(
        track
            .id
            .uid()
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
    is_server_queue_track(track) || track.id.local_path().is_some_and(|p| p.exists())
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
    db::peek_config(&db::default_db_path())
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

/// Process-wide database handle. Opened (and the legacy JSON migrated) once in
/// `main` before the UI mounts, then provided to the app via context.
static DB_HANDLE: std::sync::OnceLock<db::Db> = std::sync::OnceLock::new();

/// Open the DB and run the one-shot legacy import, blocking. sqlx-sqlite does its
/// work on dedicated connection threads, so the throwaway runtime here is safe to
/// drop — the pool keeps working under the app's runtime afterwards.
#[cfg(not(target_arch = "wasm32"))]
fn init_db_blocking() -> db::Db {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime for db init");
    rt.block_on(async {
        let db_path = db::default_db_path();
        let handle = match db::init(&db_path).await {
            Ok(h) => h,
            Err(e) => {
                // A corrupt/unopenable DB must not brick the app before a
                // window exists. Move it aside (kept for inspection) and
                // recreate — the importer below repopulates from *.json.bak.
                tracing::error!(error = %e, "kopuz database failed to open — moving it aside and recreating");
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                for ext in ["", "-wal", "-shm"] {
                    let mut src = db_path.as_os_str().to_os_string();
                    src.push(ext);
                    let mut dst = db_path.as_os_str().to_os_string();
                    dst.push(format!(".corrupt-{ts}{ext}"));
                    let _ = std::fs::rename(src, dst);
                }
                db::init(&db_path).await.expect("recreate kopuz database")
            }
        };
        match handle.import_legacy_json(&db::config_dir()).await {
            Ok(r) if r.ran => tracing::info!(
                tracks = r.tracks,
                albums = r.albums,
                playlists = r.playlists,
                favorites = r.favorites,
                servers = r.servers,
                "kopuz: migrated legacy JSON store into SQLite"
            ),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "kopuz: legacy JSON import failed"),
        }
        // Nothing reads or writes the legacy JSONs anymore — move them aside as
        // *.json.bak (kept for downgrade; never deleted). RELEASE only: debug
        // builds leave them in place so deleting kopuz-debug.db re-tests the
        // migration without restoring anything (the import is gated on the DB
        // being empty, so the lingering JSONs are otherwise inert).
        if cfg!(debug_assertions) {
            tracing::info!("kopuz: debug build — leaving legacy *.json in place for re-testing");
        } else {
            match handle.finalize_migration(&db::config_dir()).await {
                Ok(n) if n > 0 => {
                    tracing::info!(files = n, "kopuz: legacy *.json renamed to *.json.bak")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "kopuz: legacy json backup rename failed"),
            }
        }
        server::ytmusic::player::init_tier_store(handle.clone());
        utils::db_cache::init(handle.clone());
        handle
    })
}

fn main() {
    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    {
        let log_dir = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
            .map(|dirs| dirs.cache_dir().join("logs"))
            .unwrap_or_else(|| std::path::PathBuf::from("logs"));
        let _ = std::fs::create_dir_all(&log_dir);

        // Read the persisted tracing toggle from the DB before the app (and its
        // config Signal) exists — the subscriber is built once here, so the
        // setting is applied at startup. Missing DB/blob defaults to off.
        let config_tracing_enabled = db::peek_config(&db::default_db_path())
            .map(|c| c.tracing_enabled)
            .unwrap_or(false);

        // Guards live in a global inside `logging`; flushed by
        // logging::shutdown() after launch returns or on Ctrl+C.
        logging::init(&log_dir, config_tracing_enabled);

        migrate_legacy_locations();

        let _ = DB_HANDLE.set(init_db_blocking());

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
            .with_background_color((0, 0, 0, 255))
            .with_data_directory(webview_data_dir)
            .with_window(window)
            // Anon PoToken minter: stand up the hidden music.youtube.com webview
            // once we have the event-loop target (issue #349).
            .with_custom_event_handler(|_event, _target| {
                crate::pot_minter::install_if_wanted(_target);
                crate::pot_minter::pump();
            })
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
                            if hq_path.exists()
                                && let Ok(b) = tokio::fs::read(&hq_path).await
                            {
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
                    }.instrument(tracing::info_span!("artwork.serve")));
                },
            );

        dioxus::LaunchBuilder::desktop()
            .with_cfg(config)
            .launch(App);
        // Window closed → flush the log file tail + finalize the
        // chrome trace's closing bracket.
        logging::shutdown();
    }

    #[cfg(target_os = "android")]
    {
        // JNI media session + classloader cache. Player::new() also calls this (idempotent
        // OnceLock), but doing it up front means the session exists before first playback.
        player::systemint::init();

        let _ = DB_HANDLE.set(init_db_blocking());

        let config = dioxus::mobile::Config::new()
            .with_background_color((0, 0, 0, 255))
            // artwork://local?p=<percent-encoded-absolute-path> — the Android WebView mostly
            // receives base64 data URLs from utils, but keep a synchronous handler for any
            // code path that still emits artwork:// URLs.
            .with_custom_protocol("artwork".to_string(), |_headers, request| {
                let query = request.uri().query().unwrap_or("");
                let raw_p = query
                    .split('&')
                    .find_map(|kv| {
                        let mut parts = kv.splitn(2, '=');
                        if parts.next() == Some("p") {
                            parts.next()
                        } else {
                            None
                        }
                    })
                    .unwrap_or("");
                let decoded = percent_encoding::percent_decode_str(raw_p).decode_utf8_lossy();

                let mime = if decoded.ends_with(".png") {
                    "image/png"
                } else {
                    "image/jpeg"
                };

                let mut decoded_path = decoded.to_string();
                if decoded_path.starts_with("/~") {
                    if let Ok(home) = std::env::var("HOME") {
                        decoded_path = decoded_path.replacen("/~", &home, 1);
                    }
                } else if decoded_path.starts_with('~') {
                    if let Ok(home) = std::env::var("HOME") {
                        decoded_path = decoded_path.replacen('~', &home, 1);
                    }
                }

                let read_result =
                    std::fs::read(std::path::Path::new(&decoded_path)).or_else(|_| {
                        if decoded_path.strip_prefix('/').is_some() {
                            std::fs::read(std::path::Path::new(&decoded_path[1..]))
                        } else {
                            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
                        }
                    });

                match read_result {
                    Ok(bytes) => http::Response::builder()
                        .header("Content-Type", mime)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(std::borrow::Cow::from(bytes))
                        .unwrap(),
                    Err(e) => {
                        let status = if e.kind() == std::io::ErrorKind::NotFound {
                            404
                        } else {
                            500
                        };
                        http::Response::builder()
                            .status(status)
                            .header("Access-Control-Allow-Origin", "*")
                            .body(std::borrow::Cow::from(Vec::new()))
                            .unwrap()
                    }
                }
            });

        dioxus::LaunchBuilder::mobile().with_cfg(config).launch(App);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let _ = DB_HANDLE.set(db::init_stub());
        dioxus::launch(App);
    }
}

#[component]
fn App() -> Element {
    // tao's event loop calls process::exit() on window close, so the
    // logging::shutdown() after .launch() never runs and the chrome trace
    // would be left truncated (cut mid-event, unloadable). Flush on the
    // loop's final event so a normally-closed window still yields a valid
    // trace. (Ctrl+C is covered separately by the SIGINT handler.)
    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    dioxus::desktop::use_wry_event_handler(|event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        // Flush the moment the window starts closing (CloseRequested),
        // well before tao calls process::exit; LoopDestroyed is the final
        // backstop. shutdown() is idempotent so firing on both is fine.
        if matches!(
            event,
            Event::LoopDestroyed
                | Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                }
        ) {
            crate::logging::shutdown();
        }
    });

    // Native YouTube sig/n deciphering runs in this WebView's own
    // JavaScriptCore (issue #349): register a JS engine that forwards each
    // solver program to this task, which executes it via `document::eval` and
    // returns the printed result. No external JS runtime, no yt-dlp, no
    // botguard — the decipher path uses the engine already loaded for the UI.
    use_hook(|| {
        let (engine, mut rx) = server::ytmusic::decipher::webview_channel();
        if server::ytmusic::decipher::set_engine(engine).is_err() {
            tracing::warn!("yt-decipher engine already registered — webview solver not active");
        }
        spawn(async move {
            while let Some(req) = rx.recv().await {
                // Always send exactly one message back: the printed result, or
                // the JS error prefixed with a NUL marker. Without the
                // try/catch a throwing solver would never call `dioxus.send`
                // and `recv()` below would hang forever, stalling playback.
                let wrapped = format!(
                    "globalThis.print=function(s){{dioxus.send(s);}};\
                     try{{{}}}catch(e){{dioxus.send('\\u0000ERR'+(e&&e.stack?e.stack:e));}}",
                    req.program
                );
                // Bound the wait — a non-returning solver script must not stall
                // the decipher queue forever.
                let mut eval = dioxus::document::eval(&wrapped);
                let result = match tokio::time::timeout(
                    std::time::Duration::from_secs(20),
                    eval.recv::<String>(),
                )
                .await
                {
                    Ok(Ok(s)) => match s.strip_prefix('\u{0}') {
                        Some(err) => Err(format!("webview JS: {}", err.trim_start_matches("ERR"))),
                        None => Ok(s),
                    },
                    Ok(Err(e)) => Err(format!("webview eval recv: {e}")),
                    Err(_) => Err("webview decipher timed out".to_string()),
                };
                let _ = req.reply.send(result);
            }
        });
    });

    // The whole-Library signal is GONE — pages/components read the DB through
    // query hooks. The player controller's synchronous cover lookups get this
    // slim local-albums snapshot, kept fresh off the Albums generation.
    let mut local_albums = use_signal(Vec::<reader::Album>::new);
    let mut current_route = use_signal(|| Route::Home);
    let mut scroll_positions: Signal<std::collections::HashMap<Route, f64>> =
        use_signal(std::collections::HashMap::new);
    // Album/artist list and detail share one Route, so detail scroll is kept in a
    // separate map keyed by `album:<id>` / `artist:<name>`. This stops a detail's
    // scroll from clobbering the list scroll the user expects back on return.
    let mut detail_scroll_positions: Signal<std::collections::HashMap<String, f64>> =
        use_signal(std::collections::HashMap::new);
    let cache_dir = use_memo(move || {
        // Android: external/ProjectDirs paths aren't writable; use the app-internal files
        // dir (getFilesDir via JNI) so saves don't fail with EACCES.
        #[cfg(target_os = "android")]
        {
            let mut path = player::systemint::get_files_dir()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("cache");
            if std::fs::create_dir_all(&path).is_err() {
                path = std::path::PathBuf::from("./cache");
                let _ = std::fs::create_dir_all(&path);
            }
            path
        }
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
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
    let mut config = use_signal(config::AppConfig::default);
    let db = DB_HANDLE
        .get()
        .cloned()
        .expect("db initialized in main before launch");
    use_context_provider(|| db.clone());
    hooks::db_reactivity::use_generations_provider();
    // Start the PoToken minter whenever a YouTube Music server is active — not
    // just anon. A *signed-in but non-Premium* account streams the same 251 as
    // anon and also needs a content pot for deep ranges; only true Premium
    // subscribers (itag 774) are pot-exempt, and we can't know that until a
    // track resolves. So run the minter for any YtMusic session; Premium just
    // leaves it idle. Reactive: fires when config loads or the server changes.
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    use_effect(move || {
        let yt_active = config
            .read()
            .server
            .as_ref()
            .is_some_and(|s| s.service == config::MusicService::YtMusic);
        if yt_active {
            crate::pot_minter::request();
        }
    });
    hooks::use_sync_task::use_sync_task(config);
    let mut initial_load_done = use_signal(|| false);
    #[allow(unused_variables)]
    let cover_cache = use_memo(move || cache_dir().join("covers"));
    #[cfg(not(target_arch = "wasm32"))]
    let _ = std::fs::create_dir_all(cover_cache());
    let download_queue = use_signal(DownloadQueue::default);
    let download_progress = use_signal(::server::DownloadProgress::default);
    pages::server::download_manager::register_progress_signal(download_progress);
    let mut trigger_rescan = use_signal(|| 0);
    let mut last_scan_key = use_signal(|| None::<String>);
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
    // Config is the one remaining whole-value save: persisting a default that
    // exists only because the LOAD FAILED would wipe real settings/servers, so
    // its save stays disarmed unless the load demonstrably succeeded (a fresh
    // empty DB still counts). Library/playlists/favorites have no such flag
    // anymore — they're targeted per-row writes, never full-replace.
    let mut config_loaded_ok = use_signal(|| false);

    let mut pending_queue_state_snapshot = use_signal(|| None::<PersistedQueueState>);
    let mut pending_queue_state_revision = use_signal(|| 0u64);

    // tao calls process::exit() after CloseRequested, killing the debounced
    // save loops — without this, the last debounce window of queue/store
    // changes was lost on every quit. Persist everything synchronously (a
    // throwaway current-thread runtime; the sqlx pool works across runtimes,
    // same as init_db_blocking). Idempotent across CloseRequested/LoopDestroyed.
    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    dioxus::desktop::use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        if matches!(
            event,
            Event::LoopDestroyed
                | Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                }
        ) {
            if let (Some(db), Ok(rt)) = (
                DB_HANDLE.get(),
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build(),
            ) {
                let db = db.clone();
                let queue_snap = pending_queue_state_snapshot.peek().clone();
                rt.block_on(async move {
                    if let Some(snap) = queue_snap
                        && let Err(e) = db.save_queue(&queue_snapshot(snap)).await
                    {
                        tracing::warn!(error = %e, "queue flush on close failed");
                    }
                    // Library/playlists/favorites need no flush — every mutation
                    // already committed as a targeted write when it happened.
                    if *config_loaded_ok.peek() {
                        let mut cfg = config.peek().clone();
                        cfg.volume = *volume.peek();
                        let _ = db.save_config(&cfg).await;
                    }
                });
            }
        }
    });

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
        let _ = dioxus::document::eval(
            r#"document.addEventListener('error',function(e){
                var t=e.target;
                if(t.tagName==='IMG'&&!t.dataset.fallback&&t.src){
                    t.dataset.fallback='1';
                    t.src='data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%27400%27 height=%27400%27 viewBox=%270 0 400 400%27%3E%3Crect width=%27400%27 height=%27400%27 fill=%27%231e1b2e%27/%3E%3Ccircle cx=%27200%27 cy=%27180%27 r=%2770%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27/%3E%3Cpath d=%27M155 280 Q200 240 245 280%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27 stroke-linecap=%27round%27/%3E%3C/svg%3E';
                }
            },true);"#,
        );
    });

    use_effect(move || {
        let url = current_song_cover_url.read().clone();
        if !url.is_empty() {
            spawn(async move {
                if let Some(colors) = utils::color::get_palette_from_url(&url).await {
                    palette.set(Some(colors));
                }
            }.instrument(tracing::info_span!("ui.palette_fetch")));
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

    let mut station_registry = use_signal(radio::registry::StationRegistry::new);
    provide_context(station_registry);

    let mut last_radio_registry_key = use_signal(|| None::<String>);

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        let registry_paths: Vec<String> = config
            .read()
            .radio_registries
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.url.clone())
            .collect();

        let key = registry_paths.join(",");
        if *last_radio_registry_key.peek() == Some(key.clone()) {
            return;
        }
        last_radio_registry_key.set(Some(key));

        spawn(async move {
            let mut new_registry = radio::registry::StationRegistry::new();
            let mut import_count = 0;

            for path in registry_paths {
                match new_registry.import_registry(&path).await {
                    Ok(_) => import_count += 1,
                    Err(e) => tracing::warn!("Failed to import registry from {}: {}", path, e),
                }
            }

            station_registry.set(new_registry);

            if import_count > 0 {
                tracing::info!("Imported {} external radio registries", import_count);
            }
        }.instrument(tracing::info_span!("radio.registry_load")));
    });

    let mut selected_album_id = use_signal(String::new);
    let mut selected_playlist_id = use_signal(|| None::<String>);
    let mut discover_selected_playlist_id = use_signal(|| None::<String>);
    let mut discover_selected_playlist_title = use_signal(|| None::<String>);
    // YT channel id corresponding to selected_artist_name when known
    // (Discover tile / mix entry carries it). Left None when the
    // click only had a name — the YT artist page resolves it via
    // search at render time.
    let mut selected_artist_channel_id = use_signal(|| None::<String>);
    let mut selected_artist_name = use_signal(String::new);
    let fetched_artist_images: Signal<std::collections::HashMap<String, String>> =
        use_signal(std::collections::HashMap::new);
    let is_fetching_artist_images = use_signal(|| false);
    let mut search_query = use_signal(String::new);
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
        local_albums,
        config,
    );

    // Hydrate the controller's slim local-albums snapshot from the DB,
    // re-querying when the albums table changes.
    let gens_for_albums = hooks::db_reactivity::use_generations();
    let db_for_albums = db.clone();
    use_effect(move || {
        let _ = gens_for_albums.generation(hooks::db_reactivity::Table::Albums);
        let db = db_for_albums.clone();
        spawn(async move {
            if let Ok(albums) = db.albums(&db::Source::Local).await {
                local_albums.set(albums);
            }
        });
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        // Server identity excludes access_token: tokens rotate without making it a
        // different account, but their rotation would otherwise reset playback.
        let current_server_key = {
            let conf = config.read();
            conf.server.as_ref().map(|server| {
                format!(
                    "{:?}|{}|{}",
                    server.service,
                    server.url,
                    server.user_id.as_deref().unwrap_or_default(),
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
            ctrl.reset_for_backend_switch();
            // Nothing to reload: pages query by source, so switching servers is
            // just a key change — every hook re-queries the new server's rows.
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
        }.instrument(tracing::info_span!("app.update_check")));
    });

    // The store saves are FULL-REPLACE (hundreds-to-thousands of statements),
    // so saving on every signal mutation hammered the runtime — a batch
    // download bumping `offline_tracks` per finished song ran a complete
    // config save (≈840 listen-count upserts) per completion and starved the
    // audio stream into underruns. Each domain now marks itself dirty and a
    // debounced saver loop persists at most once per cooldown window,
    // coalescing bursts. The CloseRequested flush below covers quitting inside
    // the window.
    let mut config_dirty = use_signal(|| 0u64);
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let _ = config.read();
        config_dirty += 1;
    });
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let _ = *persisted_volume.read();
        config_dirty += 1;
    });
    let db_for_cfg_save = db.clone();
    use_future(move || {
        let db = db_for_cfg_save.clone();
        async move {
            let mut flushed = 0u64;
            loop {
                if *config_dirty.peek() == flushed {
                    utils::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
                utils::sleep(std::time::Duration::from_millis(STORE_SAVE_SETTLE_MS)).await;
                flushed = *config_dirty.peek();
                let mut snapshot = config.peek().clone();
                snapshot.volume = *volume.peek();
                if let Err(e) = db
                    .save_config(&snapshot)
                    .instrument(tracing::info_span!("config.persist"))
                    .await
                {
                    tracing::error!("Failed to save config: {}", e);
                }
                utils::sleep(std::time::Duration::from_millis(STORE_SAVE_COOLDOWN_MS)).await;
            }
        }
    });

    // Keepalive is rearm-on-account-change, not rearm-on-every-config-
    // write. Re-running the effect on every config save would spawn
    // a fresh loop that immediately fires run_rotation, spamming
    // /verify_session a dozen times a minute on any settings churn.
    //
    // The signal stores the YT identity (a stable hash of the SAPISID
    // cookie) we currently have a loop running against. The effect
    // re-runs cheap, but only spawns a new loop when the identity
    // changes (sign-in, account switch). Sign-out clears the
    // identity and the running loop exits on its next tick.
    #[cfg(not(target_arch = "wasm32"))]
    let mut yt_keepalive_identity = use_signal(|| None::<String>);
    #[cfg(not(target_arch = "wasm32"))]
    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }
        let yt_cookies: Option<String> = config.read().server.as_ref().and_then(|s| {
            (s.service == config::MusicService::YtMusic)
                .then(|| s.access_token.clone())
                .flatten()
                .filter(|t| !t.is_empty())
        });
        let live_identity = yt_cookies
            .as_deref()
            .and_then(server::ytmusic::derive_user_id);
        if live_identity == *yt_keepalive_identity.peek() {
            return;
        }
        // Identity changed (fresh sign-in, account switch, or
        // sign-out): the previously-running loop (if any) will read
        // the new identity on its next tick and exit. Update the
        // tracked identity; spawn a fresh loop only if we still have
        // valid auth.
        yt_keepalive_identity.set(live_identity.clone());
        let Some(my_identity) = live_identity else {
            return;
        };
        spawn(async move {
            run_rotation(config).await;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                if yt_keepalive_identity
                    .peek()
                    .as_deref()
                    != Some(my_identity.as_str())
                {
                    return;
                }
                run_rotation(config).await;
            }
        });
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

    // Library/playlists/favorites have no save loops anymore — every mutation
    // commits as a targeted write at the call site and bumps a generation.

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

    let db_for_queue_save = db.clone();
    use_future(move || {
        let db = db_for_queue_save.clone();
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
                persist_queue_state_snapshot(db.clone(), snapshot)
                    .instrument(tracing::info_span!("queue.persist"))
                    .await;
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
                .instrument(tracing::info_span!("net.probe"))
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

    let db_for_load = db.clone();
    use_hook(move || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let db = db_for_load;
            let mut ctrl = ctrl;

            spawn(async move {
                // Everything loads from the DB — the converted source of truth.
                // The legacy JSON files are never read or written; a fresh DB
                // with no blob yet just yields the default config.
                // Startup loads ONLY config + queue — everything else is queried
                // on demand by the page hooks. Config marks itself loaded ONLY
                // on success: its save is the one remaining whole-value write,
                // and persisting a default born of a read failure would wipe
                // real settings/servers.
                let cfg_loaded = match db.load_config().await {
                    Ok(c) => {
                        config_loaded_ok.set(true);
                        c
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to load config from db — config saves disabled this session");
                        None
                    }
                };
                let queue_loaded = db.load_queue().await.ok();

                let cfg_loaded = cfg_loaded.unwrap_or_default();
                {
                    let loaded = cfg_loaded;
                    config.set(loaded.clone());
                    configured_music_dirs.set(loaded.music_directory.clone());
                    volume.set(loaded.volume);
                    persisted_volume.set(loaded.volume);
                    player.write().set_volume(loaded.volume);
                    player.write().set_channel_mode(loaded.channel_mode);
                    player.write().set_equalizer(loaded.equalizer.clone());
                    i18n::set_locale(&loaded.language);
                }

                {
                    let no_local_tracks = db
                        .tracks_count(&db::TrackFilter::new(db::Source::Local))
                        .await
                        .map(|n| n == 0)
                        .unwrap_or(false);
                    let cfg = config.peek();
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

                if let Some(snap) = queue_loaded
                    && let Some(queue_state) = sanitize_queue_state(PersistedQueueState {
                        version: snap.version,
                        queue: snap.queue,
                        current_queue_index: snap.current_queue_index,
                        progress_secs: snap.progress_secs,
                        shuffle_order: snap.shuffle_order,
                        shuffle_enabled: snap.shuffle_enabled,
                    })
                {
                    ctrl.restore_queue_state(
                        queue_state.queue,
                        queue_state.current_queue_index,
                        queue_state.progress_secs,
                        queue_state.shuffle_order,
                        queue_state.shuffle_enabled,
                    );
                }

                initial_load_done.set(true);
                // Kick one reconcile shortly after startup so pending offline
                // likes from the previous session push now, not on the first
                // multi-minute interval.
                hooks::use_sync_task::nudge();
            }.instrument(tracing::info_span!("startup.load")));
        }
        // wasm: the stub Db yields defaults (web is not a shipped target); just
        // unblock the save effects.
        #[cfg(target_arch = "wasm32")]
        {
            config.write().active_source = config::MusicSource::Server;
            config_loaded_ok.set(true);
            initial_load_done.set(true);
        }
    });

    let db_for_rescan = db.clone();
    let db_for_play_album = db.clone();
    use_effect(move || {
        // config_loaded_ok matters here: a defaulted config (load failure) has
        // an empty music_directory, and the no-dirs branch below prunes the
        // local library — which must never happen off phantom state.
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let configured_dirs = configured_music_dirs.read().clone();
        let trigger = *trigger_rescan.read();
        let fetch_covers = config.peek().auto_fetch_covers;
        let fetch_strategy = config.peek().cover_fetch_strategy;
        let lastfm_key = {
            let key = config.peek().lastfm_api_key.trim().to_owned();
            (!key.is_empty()).then_some(key)
        };

        let scan_key = format!(
            "{}|{}",
            configured_dirs
                .iter()
                .map(|d| d.to_string_lossy())
                .collect::<Vec<_>>()
                .join(","),
            trigger,
        );
        if *last_scan_key.peek() == Some(scan_key.clone()) {
            return;
        }
        last_scan_key.set(Some(scan_key));

        let db_scan = db_for_rescan.clone();
        let gens_scan = gens_for_albums;
        #[cfg(not(target_arch = "wasm32"))]
        spawn(async move {
            let db = db_scan;
            let gens = gens_scan;
            let configured_dirs = configured_dirs;
            let scannable_dirs: Vec<PathBuf> = configured_dirs
                .iter()
                .filter(|d| d.exists())
                .cloned()
                .collect();
            // Seed the scan working set from the DB (the scanner skips files it
            // already knows; album-merge keeps manual covers).
            let mut current_lib = reader::Library {
                root_paths: configured_dirs.clone(),
                tracks: db
                    .tracks_all(&db::TrackFilter::new(db::Source::Local))
                    .await
                    .unwrap_or_default(),
                albums: db.albums(&db::Source::Local).await.unwrap_or_default(),
                ..Default::default()
            };

            if !configured_dirs.is_empty() {
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

                current_lib.tracks.retain(|t| {
                    let in_configured_root = configured_dirs
                        .iter()
                        .any(|d| t.id.local_path().is_some_and(|p| p.starts_with(d)));
                    let in_scannable_root = scannable_dirs
                        .iter()
                        .any(|d| t.id.local_path().is_some_and(|p| p.starts_with(d)));

                    in_configured_root
                        && (!in_scannable_root || t.id.local_path().is_some_and(|p| p.exists()))
                });

                let valid_album_ids: std::collections::HashSet<_> = current_lib
                    .tracks
                    .iter()
                    .map(|t| t.album_id.clone())
                    .collect();
                current_lib
                    .albums
                    .retain(|a| valid_album_ids.contains(&a.id));

                // Persist the scan directly: chunked upserts, prune what's gone,
                // bump so the page hooks re-query. No in-memory mirror.
                for chunk in current_lib.tracks.chunks(100) {
                    let _ = db.upsert_tracks(&db::Source::Local, chunk).await;
                    gens.bump_coalesced(hooks::db_reactivity::Table::Tracks);
                }
                let _ = db.upsert_albums(&db::Source::Local, &current_lib.albums).await;
                let keep_keys: Vec<String> = current_lib
                    .tracks
                    .iter()
                    .map(|t| t.id.key().into_owned())
                    .collect();
                let keep_albums: Vec<String> =
                    current_lib.albums.iter().map(|a| a.id.clone()).collect();
                let _ = db
                    .prune_source(&db::Source::Local, &keep_keys, &keep_albums)
                    .await;
                for (artist, img) in &current_lib.local_artist_images {
                    let p = img.to_string_lossy().into_owned();
                    let _ = db.set_artist_image(artist, "local", Some(&p)).await;
                }
                gens.bump(hooks::db_reactivity::Table::Tracks);
                gens.bump(hooks::db_reactivity::Table::Albums);

                if fetch_covers {
                    // Fetch missing covers in the background so the UI stays responsive.
                    // Passing `progress_cb` into the task keeps the scan-progress bar
                    // alive during fetching; it disappears automatically when the task ends.
                    // Albums that HAD no cover before the fetch get the fetched one
                    // written straight to the DB (manual covers were never in the
                    // missing set, so they can't be overwritten).
                    let lib_for_fetch = current_lib;
                    let db = db.clone();
                    spawn(async move {
                        let fetcher = reader::cover_fetcher::CoverFetcher::new(
                            cover_cache(),
                            fetch_strategy,
                            lastfm_key,
                            progress_cb,
                        );
                        let mut lib = lib_for_fetch;
                        let missing_before: std::collections::HashSet<String> = lib
                            .albums
                            .iter()
                            .filter(|a| a.cover_path.is_none() && !a.manual_cover)
                            .map(|a| a.id.clone())
                            .collect();
                        let report = fetcher.fetch_missing_covers(&mut lib).await;
                        tracing::info!(
                            "Cover auto-fetch: {} found, {} missing, {} errors",
                            report.found,
                            report.missing,
                            report.errors,
                        );
                        let mut changed = false;
                        for album in lib.albums.iter() {
                            if !missing_before.contains(&album.id) {
                                continue;
                            }
                            let Some(cover) = album.cover_path.as_ref() else {
                                continue;
                            };
                            let p = cover.to_string_lossy().into_owned();
                            if db
                                .update_album_cover(&db::Source::Local, &album.id, Some(&p), false)
                                .await
                                .is_ok()
                            {
                                changed = true;
                            }
                        }
                        if changed {
                            gens.bump(hooks::db_reactivity::Table::Albums);
                        }
                    }.instrument(tracing::info_span!("library.fetch_covers")));
                } else {
                    // No cover fetching — drop the callback so the progress bar closes.
                    drop(progress_cb);
                }
            } else {
                // No music directories configured: the local library is empty.
                let _ = db.prune_source(&db::Source::Local, &[], &[]).await;
                gens.bump(hooks::db_reactivity::Table::Tracks);
                gens.bump(hooks::db_reactivity::Table::Albums);
            }
        }.instrument(tracing::info_span!("library.rescan")));
    });

    use_effect(move || {
        let route = *current_route.read();
        // Read detail selections so this re-runs on list<->detail toggle, not just
        // on route change (album/artist list and detail are the same Route).
        let album_sel = selected_album_id.read().clone();
        let artist_sel = selected_artist_name.read().clone();
        let pos = match route {
            Route::Album if !album_sel.is_empty() => detail_scroll_positions
                .peek()
                .get(&format!("album:{album_sel}"))
                .copied()
                .unwrap_or(0.0),
            Route::Artist if !artist_sel.is_empty() => detail_scroll_positions
                .peek()
                .get(&format!("artist:{artist_sel}"))
                .copied()
                .unwrap_or(0.0),
            _ => scroll_positions.peek().get(&route).copied().unwrap_or(0.0),
        };
        let _ = dioxus::document::eval(&format!(
            "let el = document.getElementById('main-scroll-area'); if (el) el.scrollTop = {pos};"
        ));
    });

    provide_context(ctrl);
    provide_context(config);
    let discover_now_playing = use_signal(|| None::<String>);
    provide_context(pages::server::discover::DiscoverNowPlaying(
        discover_now_playing,
    ));
    let discover_prefetch_cache = use_signal(std::collections::HashMap::new);
    provide_context(pages::server::discover::DiscoverPrefetchCache(
        discover_prefetch_cache,
    ));
    provide_context(download_queue);
    provide_context(download_progress);
    provide_context(scroll_positions);
    provide_context(fetched_artist_images);
    provide_context(is_fetching_artist_images);
    provide_context(components::NavigationController {
        current_route,
        selected_artist_name,
        selected_artist_channel_id,
        selected_album_id,
    });

    // Sidebar collapse state. On Android the sidebar is an overlay drawer that
    // starts collapsed and is toggled by the mobile header hamburger; the
    // Sidebar component reads this from context.
    let mut is_sidebar_collapsed = use_signal(|| cfg!(target_os = "android"));
    use_context_provider(|| components::sidebar::SidebarCollapsed(is_sidebar_collapsed));

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
        WindowsToolbarIconAssets {}
        // Font stylesheets injected declaratively rather than via a
        // document::Script: a Script is processed once but App re-renders
        // on every signal change, and dioxus_document warns "Changing the
        // props of Script {} is not supported" each time. Links are
        // re-render-safe and do the same job (and load a touch earlier).
        document::Link {
            rel: "stylesheet",
            href: "https://fonts.bunny.net/css?family=jetbrains-mono:400,500,700,800&display=swap",
        }
        document::Link {
            rel: "stylesheet",
            href: "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css",
        }

        div {
            class: "flex flex-col h-screen text-white select-none overflow-x-hidden {theme_class}",
            style: "{background_style}",
            dir: "{dir}",
            "data-platform": if cfg!(target_os = "android") { "android" } else { "desktop" },
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
                    evt.prevent_default();
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

            // Only show playback errors when the active server is YouTube
            // Music — other backends (Jellyfin/Subsonic/Custom) surface
            // their own errors via the settings popup, and a lingering YT
            // error from a previous session shouldn't haunt a switched-to
            // server.
            if config
                .read()
                .server
                .as_ref()
                .map(|s| s.service == config::MusicService::YtMusic)
                .unwrap_or(false)
            {
                if let Some(msg) = ctrl.playback_error.read().clone() {
                    div {
                        class: "flex-shrink-0",
                        div {
                            class: "flex items-center justify-between gap-3 px-4 py-2 bg-rose-500/15 border-b border-rose-500/20 text-rose-200 text-sm",
                            div {
                                class: "flex items-center gap-2 whitespace-pre-line",
                                i { class: "fa-solid fa-triangle-exclamation text-xs" }
                                span { "{msg}" }
                            }
                            button {
                                class: "opacity-50 hover:opacity-100 transition-opacity p-1",
                                onclick: move |_| ctrl.playback_error.set(None),
                                i { class: "fa-solid fa-xmark text-xs" }
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
                            if !cfg!(target_os = "android") {
                                button {
                                    class: "ml-2 text-xs underline opacity-80 hover:opacity-100 transition-opacity",
                                    onclick: {
                                        let release_url = update.release_url.clone();
                                        move |_| {
                                            #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
                                            if let Err(e) = webbrowser::open(&release_url) {
                                                tracing::error!("Failed to open release page: {}", e);
                                            }
                                            #[cfg(target_os = "android")]
                                            let _ = &release_url;
                                        }
                                    },
                                    "{i18n::t(\"view_release\")}"
                                }
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
                            selected_artist_channel_id.set(None);
                        }
                        current_route.set(route);
                    }
                }
                div {
                    id: "main-scroll-area",
                    class: if cfg!(target_os = "android") { "flex-1 min-h-0 flex flex-col overflow-hidden relative" } else { "flex-1 overflow-y-auto" },
                    onscroll: move |evt| {
                        let pos = evt.scroll_top();
                        let route = *current_route.peek();
                        let album_sel = selected_album_id.peek().clone();
                        let artist_sel = selected_artist_name.peek().clone();
                        match route {
                            Route::Album if !album_sel.is_empty() => {
                                detail_scroll_positions
                                    .write()
                                    .insert(format!("album:{album_sel}"), pos);
                            }
                            Route::Artist if !artist_sel.is_empty() => {
                                detail_scroll_positions
                                    .write()
                                    .insert(format!("artist:{artist_sel}"), pos);
                            }
                            _ => {
                                scroll_positions.write().insert(route, pos);
                            }
                        }
                    },

                    if cfg!(target_os = "android") {
                        {
                            let is_details = match *current_route.read() {
                                Route::Album => !selected_album_id.read().is_empty(),
                                Route::Artist => !selected_artist_name.read().is_empty(),
                                Route::Playlists => selected_playlist_id.read().is_some(),
                                _ => false,
                            };
                            let page_title = match *current_route.read() {
                                Route::Home => i18n::t("home"),
                                Route::Search => i18n::t("search"),
                                Route::Library => i18n::t("library"),
                                Route::Album => if is_details { i18n::t("album") } else { i18n::t("albums") },
                                Route::Artist => if is_details { i18n::t("artist") } else { i18n::t("artists") },
                                Route::Playlists => i18n::t("playlists"),
                                Route::Favorites => i18n::t("favorites"),
                                Route::Settings => i18n::t("settings"),
                                _ => i18n::t("home"),
                            };
                            rsx! {
                                div { class: "shrink-0 z-[60] bg-black/60 backdrop-blur-2xl border-b border-white/5 pt-[env(safe-area-inset-top)] flex items-center h-[calc(env(safe-area-inset-top)_+_2.75rem)] px-3 shadow-xl",
                                    if is_details {
                                        button {
                                            class: "w-10 h-10 flex items-center justify-center rounded-xl bg-white/5 text-white active:scale-95 transition-all border border-white/10",
                                            onclick: move |_| {
                                                match *current_route.peek() {
                                                    Route::Album => selected_album_id.set(String::new()),
                                                    Route::Artist => {
                                                        selected_artist_name.set(String::new());
                                                        selected_artist_channel_id.set(None);
                                                    }
                                                    Route::Playlists => selected_playlist_id.set(None),
                                                    _ => {}
                                                }
                                            },
                                            i { class: "fa-solid fa-arrow-left text-lg" }
                                        }
                                    } else {
                                        button {
                                            class: "w-10 h-10 flex items-center justify-center rounded-xl bg-white/5 text-white active:scale-95 transition-all border border-white/10",
                                            onclick: move |_| is_sidebar_collapsed.toggle(),
                                            i { class: "fa-solid fa-bars text-lg" }
                                        }
                                    }
                                    div { class: "flex-1 flex justify-center pr-10",
                                        h2 {
                                            class: "text-[13px] font-black tracking-[0.2em] text-white/90 uppercase",
                                            style: "font-family: 'JetBrains Mono', monospace;",
                                            "{page_title}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    div { class: if cfg!(target_os = "android") { "relative flex-1 min-h-0 overflow-y-auto" } else { "contents" },
                    match *current_route.read() {
                        Route::Home => rsx! {
                            pages::home::Home {
                                on_select_album: move |id: String| {
                                    selected_album_id.set(id);
                                    current_route.set(Route::Album);
                                },
                                on_play_album: move |id: String| {
                                    selected_album_id.set(id.clone());

                                    let is_jelly = id.starts_with("jellyfin:");
                                    let source = if is_jelly {
                                        let c = config.peek();
                                        db::Source::Server(
                                            c.active_server_id
                                                .clone()
                                                .or_else(|| c.server.as_ref().and_then(|s| s.id.clone()))
                                                .unwrap_or_default(),
                                        )
                                    } else {
                                        db::Source::Local
                                    };
                                    let db = db_for_play_album.clone();
                                    spawn(async move {
                                        let mut tracks = db
                                            .tracks_all(&db::TrackFilter::album(source, id))
                                            .await
                                            .unwrap_or_default();
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
                                    });
                                    current_route.set(Route::Album);
                                },
                                on_select_playlist: move |id: String| {
                                    selected_playlist_id.set(Some(id));
                                    current_route.set(Route::Playlists);
                                },
                                on_search_artist: move |artist: String| {
                                    selected_artist_name.set(artist);
                                    selected_artist_channel_id.set(None);
                                    current_route.set(Route::Artist);
                                }
                            }
                        },
                        Route::Discover => rsx! {
                            pages::server::discover::DiscoverPage {
                                on_select_album: move |id: String| {
                                    selected_album_id.set(id);
                                    current_route.set(Route::Album);
                                },
                                on_select_playlist: move |(id, title): (String, String)| {
                                    discover_selected_playlist_id.set(Some(id));
                                    discover_selected_playlist_title.set(Some(title));
                                    current_route.set(Route::DiscoverPlaylist);
                                },
                                on_open_artist: move |(cid, name): (String, String)| {
                                    selected_artist_channel_id.set(Some(cid));
                                    selected_artist_name.set(name);
                                    current_route.set(Route::Artist);
                                },
                                on_search_artist: move |name: String| {
                                    search_query.set(name);
                                    current_route.set(Route::Search);
                                },
                            }
                        },
                        Route::DiscoverPlaylist => rsx! {
                            pages::server::discover::DiscoverPlaylistDetail {
                                selected_playlist_id: discover_selected_playlist_id,
                                selected_playlist_title: discover_selected_playlist_title,
                                on_back: move |_| {
                                    // Mirror DiscoverArtist: clear id so
                                    // re-opening the same playlist refetches.
                                    discover_selected_playlist_id.set(None);
                                    discover_selected_playlist_title.set(None);
                                    current_route.set(Route::Discover);
                                },
                            }
                        },
                        Route::Search => rsx! {
                            pages::search::Search {
                                config: config,
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
                                config: config,
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
                                config: config,
                                album_id: selected_album_id,
                                queue: queue,
                                current_queue_index: current_queue_index,
                            }
                        },
                        Route::Artist => {
                            // YT Music gets the rich YT-backed profile (banner,
                            // top songs, albums, related) ONLY when an artist
                            // is actually selected. The Artists sidebar tab /
                            // back-to-list navigation lands with both signals
                            // cleared — fall through to the library-driven
                            // grid in that case (populated on YT from followed
                            // artists + liked-song artists by the library
                            // sync). Local / Jellyfin / Subsonic keep the
                            // library-driven page in all cases.
                            let is_ytmusic = config
                                .read()
                                .server
                                .as_ref()
                                .map(|s| s.service == config::MusicService::YtMusic)
                                .unwrap_or(false);
                            let has_selection = !selected_artist_name.read().is_empty()
                                || selected_artist_channel_id.read().is_some();
                            if is_ytmusic && has_selection {
                                rsx! {
                                    pages::server::discover::DiscoverArtistPage {
                                        selected_artist_id: selected_artist_channel_id,
                                        selected_artist_name: selected_artist_name,
                                        on_back: move |_| {
                                            // Empty selection on Route::Artist renders the grid.
                                            selected_artist_name.set(String::new());
                                            selected_artist_channel_id.set(None);
                                            current_route.set(Route::Artist);
                                        },
                                        on_select_album: move |id: String| {
                                            selected_album_id.set(id);
                                            current_route.set(Route::Album);
                                        },
                                        on_select_playlist: move |(id, title): (String, String)| {
                                            discover_selected_playlist_id.set(Some(id));
                                            discover_selected_playlist_title.set(Some(title));
                                            current_route.set(Route::DiscoverPlaylist);
                                        },
                                        on_open_artist: move |(cid, name): (String, String)| {
                                            selected_artist_channel_id.set(Some(cid));
                                            selected_artist_name.set(name);
                                        },
                                        on_search_artist: move |name: String| {
                                            search_query.set(name);
                                            current_route.set(Route::Search);
                                        },
                                    }
                                }
                            } else {
                                rsx! {
                                    pages::artist::Artist {
                                        config: config,
                                        artist_name: selected_artist_name,
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
                                }
                            }
                        },
                        Route::Favorites => rsx! {
                            pages::favorites::FavoritesPage {
                                config,
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
                                config: config,
                                selected_playlist_id: selected_playlist_id,
                            }
                        },
                        Route::Activity => rsx! {
                          pages::activity::Activity {
                              config: config,
                          }
                        },
                        Route::Radio => rsx! {
                            pages::radio::Radio {
                                config: config,
                            }
                        },
                        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
                        Route::Ytdlp => rsx! { pages::ytdlp::YtdlpPage { config, trigger_rescan } },
                        #[cfg(target_arch = "wasm32")]
                        Route::Ytdlp => rsx! { pages::settings::Settings { config } },
                        Route::Settings => rsx! { pages::settings::Settings { config } },
                        #[cfg(not(target_os = "android"))]
                        Route::ThemeEditor => rsx! { pages::theme_editor::ThemeEditorPage { config } },
                    }
                    }
                }
                Rightbar {
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
                player: player,
                is_playing: is_playing,
                is_fullscreen: is_fullscreen,
                current_song_duration: current_song_duration,
                current_song_progress: current_song_progress,
                queue: queue,
                current_song_album: current_song_album,
                current_queue_index: current_queue_index,
                current_song_title: current_song_title,
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
