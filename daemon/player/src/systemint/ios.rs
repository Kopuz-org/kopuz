//! iOS system integration, the counterpart of `android.rs`.
//!
//! The platform half is Swift (`apps/legacy/kopuz/ios-src`), compiled by the
//! app's build script into a static library and reached through a plain C ABI:
//! these `kopuz_ios_*` functions are `@_cdecl` exports on the Swift side. The
//! other direction does not link a symbol back into Rust; `init` hands Swift a
//! function pointer instead, so nothing depends on the linker keeping an
//! otherwise unreferenced `#[no_mangle]` export alive.

use std::ffi::{CStr, CString, c_char};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Copy)]
pub enum SystemEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek(f64),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
}

/// Mirrors the desktop `RepeatMode` so the UI can push one mode enum to every
/// platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Off,
    Playlist,
    Track,
}

impl RepeatMode {
    /// Matches `KopuzRepeat` in `KopuzBridge.swift`.
    fn as_swift(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::Playlist => 1,
            Self::Track => 2,
        }
    }

    fn from_swift(value: f64) -> Self {
        match value as i32 {
            1 => Self::Playlist,
            2 => Self::Track,
            _ => Self::Off,
        }
    }
}

type ActionCallback = extern "C" fn(action: *const c_char, value: f64);

unsafe extern "C" {
    fn kopuz_ios_init(on_action: ActionCallback);
    fn kopuz_ios_configure_webview();
    fn kopuz_ios_now_playing(
        title: *const c_char,
        artist: *const c_char,
        album: *const c_char,
        duration: f64,
        position: f64,
        playing: bool,
        artwork: *const c_char,
    );
    fn kopuz_ios_modes(shuffle: bool, repeat: i32);
    fn kopuz_ios_stop_session();
    fn kopuz_ios_login_open(url: *const c_char) -> bool;
    fn kopuz_ios_login_cookies(url: *const c_char) -> *mut c_char;
    fn kopuz_ios_login_is_open() -> bool;
    fn kopuz_ios_login_close();
    fn kopuz_ios_free(ptr: *mut c_char);
}

unsafe extern "C" {
    fn CFRunLoopGetMain() -> *mut std::ffi::c_void;
    fn CFRunLoopWakeUp(rl: *mut std::ffi::c_void);
}

type BackgroundHandler = Arc<Mutex<Option<Box<dyn Fn(SystemEvent) + Send + Sync>>>>;
static BACKGROUND_HANDLER: OnceLock<BackgroundHandler> = OnceLock::new();

fn get_bg_handler() -> BackgroundHandler {
    BACKGROUND_HANDLER
        .get_or_init(|| Arc::new(Mutex::new(None)))
        .clone()
}

pub fn set_background_handler(handler: impl Fn(SystemEvent) + Send + Sync + 'static) {
    let binding = get_bg_handler();
    let mut guard = binding.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(Box::new(handler));
}

fn dispatch_event(event: SystemEvent) {
    if let Ok(guard) = get_bg_handler().lock()
        && let Some(ref handler) = *guard
    {
        handler(event);
    }
    wake_run_loop();
}

/// tao runs UIKit's main CFRunLoop; a wake makes dioxus poll the UI's tasks
/// now instead of on the next touch.
pub fn wake_run_loop() {
    unsafe { CFRunLoopWakeUp(CFRunLoopGetMain()) }
}

/// Where Swift reports a remote command, an audio interruption or a route
/// change. `value` carries the seek position in seconds, the shuffle flag, or
/// the repeat mode; the other actions ignore it.
extern "C" fn on_action(action: *const c_char, value: f64) {
    if action.is_null() {
        return;
    }
    let action = unsafe { CStr::from_ptr(action) }.to_string_lossy();
    let event = match action.as_ref() {
        "play" => SystemEvent::Play,
        "pause" => SystemEvent::Pause,
        "toggle" => SystemEvent::Toggle,
        "next" => SystemEvent::Next,
        "prev" => SystemEvent::Prev,
        "seek" => SystemEvent::Seek(value),
        "shuffle" => SystemEvent::SetShuffle(value != 0.0),
        "repeat" => SystemEvent::SetRepeat(RepeatMode::from_swift(value)),
        other => {
            tracing::debug!(action = other, "unknown iOS media action");
            return;
        }
    };
    dispatch_event(event);
}

/// Configure the audio session for background playback and register the
/// remote commands. Idempotent, and it has to run before the first output
/// opens: an output opened under the default session category goes silent with
/// the ring switch and stops when the app leaves the foreground.
pub fn init() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe { kopuz_ios_init(on_action) });
}

/// Let the page draw edge to edge and inset itself with
/// `env(safe-area-inset-*)`, as it does on Android. WKWebView otherwise insets
/// its own content by the safe area too, which pushes the page down by the
/// status bar and its bottom, the tab bar with it, off the screen.
pub fn configure_webview() {
    unsafe { kopuz_ios_configure_webview() }
}

/// Interior NULs cannot cross into a C string; a tag carrying one loses the
/// byte rather than the whole field.
fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

pub fn update_now_playing(
    title: &str,
    artist: &str,
    album: &str,
    duration: f64,
    position: f64,
    playing: bool,
    artwork: Option<&str>,
) {
    init();
    let title = c_string(title);
    let artist = c_string(artist);
    let album = c_string(album);
    let artwork = artwork.map(c_string);
    unsafe {
        kopuz_ios_now_playing(
            title.as_ptr(),
            artist.as_ptr(),
            album.as_ptr(),
            duration,
            position,
            playing,
            artwork
                .as_ref()
                .map_or(std::ptr::null(), |art| art.as_ptr()),
        );
    }
}

/// Push the queue's shuffle/repeat state to the remote command center so
/// CarPlay and the accessories that draw these buttons show what is in effect.
pub fn update_modes(shuffle: bool, repeat: RepeatMode) {
    init();
    unsafe { kopuz_ios_modes(shuffle, repeat.as_swift()) }
}

pub fn stop_session() {
    unsafe { kopuz_ios_stop_session() }
}

/// Present the in-app sign-in browser at `url` over a fresh, empty cookie jar,
/// so only the session made in it satisfies the caller's poll.
pub fn login_open(url: &str) -> Result<(), String> {
    init();
    let url = c_string(url);
    if unsafe { kopuz_ios_login_open(url.as_ptr()) } {
        Ok(())
    } else {
        Err("Could not open the sign-in WebView".into())
    }
}

/// The `Cookie:`-header string the sign-in browser holds for `url`, if any.
pub fn login_cookies(url: &str) -> Option<String> {
    let url = c_string(url);
    let raw = unsafe { kopuz_ios_login_cookies(url.as_ptr()) };
    if raw.is_null() {
        return None;
    }
    let header = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    unsafe { kopuz_ios_free(raw) };
    (!header.is_empty()).then_some(header)
}

/// Whether the sign-in browser is still on screen; `false` after the user
/// dismisses it, which the polling caller reports as a cancelled sign-in.
pub fn login_is_open() -> bool {
    unsafe { kopuz_ios_login_is_open() }
}

/// Dismiss the sign-in browser once the caller has what it needs.
pub fn login_close() {
    unsafe { kopuz_ios_login_close() }
}
