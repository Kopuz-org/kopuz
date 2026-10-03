// Windows system integration: System Media Transport Controls (SMTC),
// media keys, Now Playing info, and taskbar thumb buttons.
//
// Architecture:
// - COM must be initialized on the thread that uses WinRT APIs. Since the
//   Tokio thread pool does not call CoInitializeEx, setup runs on a
//   dedicated std::thread::spawn thread.
// - The daemon boots before the app window exists (and kopuzd never has
//   one), so `init` binds SMTC to a hidden window owned by that dedicated
//   thread, which then pumps messages for the life of the process so the
//   window stays alive. Once the app has a window it calls `attach_window`
//   and SMTC moves to it, which also gives the taskbar its thumb buttons.
// - SMTC button events (play/pause/next/prev/seek) are forwarded to the
//   player via an unbounded mpsc channel.
// - CoInitializeEx + WinRT/COM FFI is documented with // SAFETY: invariants.

use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex as StdMutex, OnceLock};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use windows::core::{PCWSTR, Ref, w};
use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
        SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    },
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        },
        System::WinRT::RoGetActivationFactory,
        UI::{
            Shell::{
                ITaskbarList3, THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_ENABLED, THBN_CLICKED,
                THUMBBUTTON, TaskbarList,
            },
            WindowsAndMessaging::{
                CallWindowProcW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_WNDPROC,
                GetMessageW, HICON, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadImageW, MSG,
                SetWindowLongPtrW, TranslateMessage, WINDOW_EX_STYLE, WM_COMMAND, WNDPROC,
                WS_OVERLAPPED,
            },
        },
    },
};

#[derive(Debug)]
pub enum SystemEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek(f64),
}

/// The live SMTC instance and the window it is bound to.
struct Binding {
    smtc: SystemMediaTransportControls,
    hwnd: isize,
}

static SMTC: StdMutex<Option<Binding>> = StdMutex::new(None);
/// The app's real window, once it has reported one; owns the taskbar buttons.
static APP_HWND: AtomicIsize = AtomicIsize::new(0);
/// The last state pushed, replayed onto SMTC when it moves to a new window.
static LAST_NOW_PLAYING: StdMutex<Option<NowPlaying>> = StdMutex::new(None);

#[derive(Clone)]
struct NowPlaying {
    title: String,
    artist: String,
    album: String,
    duration: f64,
    position: f64,
    playing: bool,
    artwork: Option<String>,
}

fn current_smtc() -> Option<SystemMediaTransportControls> {
    SMTC.lock()
        .ok()?
        .as_ref()
        .map(|binding| binding.smtc.clone())
}

static EVENT_SENDER: OnceLock<UnboundedSender<SystemEvent>> = OnceLock::new();
static EVENT_RECEIVER: OnceLock<Mutex<UnboundedReceiver<SystemEvent>>> = OnceLock::new();

fn get_tx() -> UnboundedSender<SystemEvent> {
    EVENT_SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            let _ = EVENT_RECEIVER.set(Mutex::new(rx));
            tx
        })
        .clone()
}

pub fn poll_event() -> Option<SystemEvent> {
    EVENT_RECEIVER.get()?.try_lock().ok()?.try_recv().ok()
}

pub async fn wait_event() -> Option<SystemEvent> {
    // Create the channel here too: the listener can start before SMTC setup
    // has run, and must not mistake "not set up yet" for "closed".
    let _ = get_tx();
    if let Some(rx) = EVENT_RECEIVER.get() {
        let mut guard = rx.lock().await;
        guard.recv().await
    } else {
        None
    }
}

/// A hidden top-level window for SMTC to bind to until the app has one.
/// Owned by the calling thread, which must keep pumping messages.
fn create_hidden_window() -> Option<HWND> {
    // SAFETY:
    // - "STATIC" is a system window class, so no registration is needed.
    // - The window is never shown (no WS_VISIBLE), so it has no visual or
    //   taskbar presence; the return value is checked before use.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("KopuzSMTC"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )
    };
    match hwnd {
        Ok(h) if !h.0.is_null() => Some(h),
        _ => None,
    }
}

const TASKBAR_PREV_ID: u32 = 0x4b01;
const TASKBAR_PLAY_PAUSE_ID: u32 = 0x4b02;
const TASKBAR_NEXT_ID: u32 = 0x4b03;

static TASKBAR_HWND: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_BUTTONS_ADDED: AtomicBool = AtomicBool::new(false);
static TASKBAR_SUBCLASS_LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
static TASKBAR_BUTTONS_LOCK: OnceLock<StdMutex<()>> = OnceLock::new();

unsafe extern "system" fn taskbar_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_COMMAND {
        let command_id = (wparam.0 & 0xffff) as u32;
        let notification_code = ((wparam.0 >> 16) & 0xffff) as u32;

        if notification_code == THBN_CLICKED {
            let event = match command_id {
                TASKBAR_PREV_ID => Some(SystemEvent::Prev),
                TASKBAR_PLAY_PAUSE_ID => Some(SystemEvent::Toggle),
                TASKBAR_NEXT_ID => Some(SystemEvent::Next),
                _ => None,
            };

            if let Some(event) = event {
                let _ = get_tx().send(event);
                return LRESULT(0);
            }
        }
    }

    call_taskbar_prev_wndproc(hwnd, msg, wparam, lparam)
}

fn call_taskbar_prev_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let prev = TASKBAR_PREV_WNDPROC.load(Ordering::Acquire);
    if prev == 0 {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    let prev_proc: WNDPROC = unsafe { std::mem::transmute(prev) };
    unsafe { CallWindowProcW(prev_proc, hwnd, msg, wparam, lparam) }
}

fn install_taskbar_subclass(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }

    let Ok(_guard) = TASKBAR_SUBCLASS_LOCK
        .get_or_init(|| StdMutex::new(()))
        .lock()
    else {
        return;
    };

    let installed_hwnd = TASKBAR_HWND.load(Ordering::Acquire);
    if installed_hwnd == hwnd.0 as isize {
        return;
    }
    if installed_hwnd != 0 {
        tracing::debug!("taskbar subclass already installed for another HWND");
        return;
    }

    let wndproc = taskbar_wndproc as *const () as isize;
    let prev = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc) };
    if prev != 0 {
        TASKBAR_PREV_WNDPROC.store(prev, Ordering::Release);
        TASKBAR_HWND.store(hwnd.0 as isize, Ordering::Release);
    }
}

fn fill_tip(buf: &mut [u16; 260], text: &str) {
    for (idx, unit) in text.encode_utf16().take(buf.len() - 1).enumerate() {
        buf[idx] = unit;
    }
}

fn make_icon(kind: TaskbarIconKind) -> Option<HICON> {
    let icon_path = find_toolbar_icon(kind)?;
    let mut wide_path: Vec<u16> = icon_path.as_os_str().encode_wide().collect();
    wide_path.push(0);

    let handle = unsafe {
        LoadImageW(
            None,
            PCWSTR(wide_path.as_ptr()),
            IMAGE_ICON,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        )
        .ok()?
    };

    Some(HICON(handle.0))
}

fn find_toolbar_icon(kind: TaskbarIconKind) -> Option<std::path::PathBuf> {
    let file_name = match kind {
        TaskbarIconKind::Previous => "backward-step-solid-full.ico",
        TaskbarIconKind::Play => "play-solid-full.ico",
        TaskbarIconKind::Pause => "pause-solid-full.ico",
        TaskbarIconKind::Next => "forward-step-solid-full.ico",
    };

    let mut bases = Vec::new();

    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        bases.push(exe_dir.join("assets").join("toolbar_icons"));
        bases.push(exe_dir.join("kopuz").join("assets").join("toolbar_icons"));
    }

    if let Ok(current_dir) = std::env::current_dir() {
        bases.push(current_dir.join("assets").join("toolbar_icons"));
        bases.push(
            current_dir
                .join("kopuz")
                .join("assets")
                .join("toolbar_icons"),
        );
    }

    bases.push(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("kopuz")
            .join("assets")
            .join("toolbar_icons"),
    );

    bases
        .into_iter()
        .map(|base| base.join(file_name))
        .find(|path| path.is_file())
}

#[derive(Clone, Copy)]
enum TaskbarIconKind {
    Previous,
    Play,
    Pause,
    Next,
}

fn taskbar_button(id: u32, icon: HICON, tip: &str) -> THUMBBUTTON {
    let mut button = THUMBBUTTON {
        dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
        iId: id,
        iBitmap: 0,
        hIcon: icon,
        szTip: [0; 260],
        dwFlags: THBF_ENABLED,
    };
    fill_tip(&mut button.szTip, tip);
    button
}

fn create_taskbar_list() -> windows::core::Result<ITaskbarList3> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let taskbar: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)?;
        taskbar.HrInit()?;
        Ok(taskbar)
    }
}

fn setup_taskbar_buttons(hwnd: HWND, playing: bool) {
    if hwnd.0.is_null() {
        return;
    }

    install_taskbar_subclass(hwnd);

    let Ok(_guard) = TASKBAR_BUTTONS_LOCK
        .get_or_init(|| StdMutex::new(()))
        .lock()
    else {
        return;
    };

    let result = (|| -> windows::core::Result<()> {
        let taskbar = create_taskbar_list()?;
        let Some(prev_icon) = make_icon(TaskbarIconKind::Previous) else {
            return Ok(());
        };
        let Some(play_pause_icon) = make_icon(if playing {
            TaskbarIconKind::Pause
        } else {
            TaskbarIconKind::Play
        }) else {
            return Ok(());
        };
        let Some(next_icon) = make_icon(TaskbarIconKind::Next) else {
            return Ok(());
        };
        let play_pause_tip = if playing { "Pause" } else { "Play" };
        let buttons = [
            taskbar_button(TASKBAR_PREV_ID, prev_icon, "Previous"),
            taskbar_button(TASKBAR_PLAY_PAUSE_ID, play_pause_icon, play_pause_tip),
            taskbar_button(TASKBAR_NEXT_ID, next_icon, "Next"),
        ];

        unsafe {
            if TASKBAR_BUTTONS_ADDED.load(Ordering::Acquire) {
                taskbar.ThumbBarUpdateButtons(hwnd, &buttons)?;
            } else {
                taskbar.ThumbBarAddButtons(hwnd, &buttons)?;
                TASKBAR_BUTTONS_ADDED.store(true, Ordering::Release);
            }
        }
        Ok(())
    })();

    if let Err(e) = result {
        tracing::warn!(error = ?e, "taskbar media buttons setup failed");
    }
}

// SMTC setup
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

fn create_smtc(hwnd: HWND) -> windows::core::Result<SystemMediaTransportControls> {
    // SAFETY:
    // - RoGetActivationFactory is a WinRT API that is safe to call
    //   after CoInitializeEx has been initialized on this thread.
    // - ISystemMediaTransportControlsInterop::GetForWindow is safe
    //   with a valid HWND owned by this process.
    // - All subsequent SMTC method calls are thread-safe COM/WinRT
    //   operations that do not violate memory safety.
    // - The TypedEventHandler closures capture the sender by value
    //   and do not introduce data races.
    unsafe {
        let class_id = windows::core::HSTRING::from("Windows.Media.SystemMediaTransportControls");
        let interop: ISystemMediaTransportControlsInterop = RoGetActivationFactory(&class_id)?;
        let smtc: SystemMediaTransportControls = interop.GetForWindow(hwnd)?;

        smtc.SetIsEnabled(true)?;
        smtc.SetIsPlayEnabled(true)?;
        smtc.SetIsPauseEnabled(true)?;
        smtc.SetIsNextEnabled(true)?;
        smtc.SetIsPreviousEnabled(true)?;
        smtc.SetIsStopEnabled(true)?;

        let tx = get_tx();
        let seek_tx = tx.clone();

        smtc.ButtonPressed(&TypedEventHandler::new(
            move |_: Ref<SystemMediaTransportControls>,
                  args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>|
                  -> windows::core::Result<()> {
                if let Some(args) = args.as_ref() {
                    let btn: SystemMediaTransportControlsButton = args.Button()?;
                    let evt = if btn == SystemMediaTransportControlsButton::Play
                        || btn == SystemMediaTransportControlsButton::Pause
                    {
                        Some(SystemEvent::Toggle)
                    } else if btn == SystemMediaTransportControlsButton::Next {
                        Some(SystemEvent::Next)
                    } else if btn == SystemMediaTransportControlsButton::Previous {
                        Some(SystemEvent::Prev)
                    } else {
                        None
                    };
                    if let Some(e) = evt {
                        let _ = tx.send(e);
                    }
                }
                Ok(())
            },
        ))?;

        smtc.PlaybackPositionChangeRequested(&TypedEventHandler::new(
            move |_: Ref<SystemMediaTransportControls>,
                  args: Ref<PlaybackPositionChangeRequestedEventArgs>|
                  -> windows::core::Result<()> {
                if let Some(args) = args.as_ref() {
                    let pos = args.RequestedPlaybackPosition()?;
                    let secs = pos.Duration as f64 / 10_000_000.0;
                    let _ = seek_tx.send(SystemEvent::Seek(secs));
                }
                Ok(())
            },
        ))?;

        Ok(smtc)
    }
}

/// Bind SMTC to `hwnd`, retiring any previous binding. With `only_if_unbound`
/// an existing binding wins, so the fallback never displaces the app window.
fn bind_smtc(hwnd: HWND, only_if_unbound: bool) -> bool {
    let Ok(mut slot) = SMTC.lock() else {
        return false;
    };
    match slot.as_ref() {
        Some(_) if only_if_unbound => return false,
        Some(binding) if binding.hwnd == hwnd.0 as isize => return false,
        _ => {}
    }
    match create_smtc(hwnd) {
        Ok(smtc) => {
            if let Some(old) = slot.replace(Binding {
                smtc,
                hwnd: hwnd.0 as isize,
            }) {
                // Two enabled sessions would show up as two players.
                let _ = old.smtc.SetIsEnabled(false);
            }
            tracing::debug!("SMTC bound to window");
            true
        }
        Err(e) => {
            tracing::warn!(error = ?e, "SMTC setup failed");
            false
        }
    }
}

pub fn init() {
    // The event channel has to exist before anyone waits on it.
    let _ = get_tx();
    static INIT_ONCE: OnceLock<()> = OnceLock::new();
    INIT_ONCE.get_or_init(|| {
        std::thread::spawn(|| {
            // CoInitializeEx must be called on the thread that uses WinRT/COM.
            // The tokio thread pool does not do this, so setup runs here.
            // SAFETY:
            // - CoInitializeEx initializes COM for the calling thread with
            //   the specified concurrency model (apartment-threaded).
            // - It is safe to call once per thread; subsequent calls return
            //   S_FALSE or RPC_E_CHANGED_MODE, which we ignore.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }

            let Some(hwnd) = create_hidden_window() else {
                tracing::warn!("could not create a window for SMTC");
                return;
            };
            if bind_smtc(hwnd, true) {
                replay_now_playing();
            }

            // A window dies with the thread that created it, so this thread
            // stays and pumps its messages for the life of the process.
            let mut msg = MSG::default();
            // SAFETY: a standard message loop over an owned MSG buffer.
            unsafe {
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
    });
}

/// Move SMTC onto the app's window and give it taskbar thumb buttons. Call
/// from the thread that owns the window, once it exists.
pub fn attach_window(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    let _ = get_tx();
    // SAFETY: see `init`; a thread that already initialised COM (as the UI
    // thread has) just gets S_FALSE or RPC_E_CHANGED_MODE back.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    APP_HWND.store(hwnd, Ordering::Release);
    let hwnd = HWND(hwnd as _);
    bind_smtc(hwnd, false);
    let playing = LAST_NOW_PLAYING
        .lock()
        .ok()
        .and_then(|last| last.as_ref().map(|it| it.playing))
        .unwrap_or(false);
    setup_taskbar_buttons(hwnd, playing);
    replay_now_playing();
}

fn replay_now_playing() {
    let last = LAST_NOW_PLAYING.lock().ok().and_then(|last| last.clone());
    if let Some(it) = last {
        update_now_playing(
            &it.title,
            &it.artist,
            &it.album,
            it.duration,
            it.position,
            it.playing,
            it.artwork.as_deref(),
        );
    }
}

// convert seconds to a Windows TimeSpan (unit is 100-nanosecond ticks)
#[inline]
fn secs_to_timespan(secs: f64) -> TimeSpan {
    TimeSpan {
        Duration: (secs * 10_000_000.0) as i64,
    }
}

// helper funcs: wrap raw bytes in an in-memory stream SMTC can read
// or fetch image bytes from either a local path or an url
fn stream_ref_from_bytes(bytes: &[u8]) -> Option<RandomAccessStreamReference> {
    let stream = InMemoryRandomAccessStream::new().ok()?;
    let writer = DataWriter::CreateDataWriter(&stream).ok()?;
    writer.WriteBytes(bytes).ok()?;
    tokio::runtime::Builder::new_current_thread()
        .build()
        .ok()?
        .block_on(async { writer.StoreAsync().ok()?.await.ok() })?;
    writer.DetachStream().ok()?;
    stream.Seek(0).ok()?; // rewind so SMTC reads from the start
    RandomAccessStreamReference::CreateFromStream(&stream).ok()
}

fn fetch_artwork_bytes(path: &str) -> Option<Vec<u8>> {
    if path.starts_with("http://") || path.starts_with("https://") {
        let resp = reqwest::blocking::get(path).ok()?;
        if resp.status().is_success() {
            resp.bytes().ok().map(|b| b.to_vec())
        } else {
            None
        }
    } else {
        std::fs::read(path).ok()
    }
}

/// The artwork SMTC shows, keyed by the path or URL it came from: `None` while
/// it is still loading, so play/pause and seek updates reuse one fetch.
static THUMBNAIL: StdMutex<Option<(String, Option<RandomAccessStreamReference>)>> =
    StdMutex::new(None);

/// What is known about `art`: `None` when nothing has been asked for it yet,
/// `Some(None)` while it loads.
fn cached_thumbnail(art: &str) -> Option<Option<RandomAccessStreamReference>> {
    let cache = THUMBNAIL.lock().ok()?;
    cache
        .as_ref()
        .filter(|(key, _)| key == art)
        .map(|(_, stream)| stream.clone())
}

/// Fetch `art` off the calling thread and hand SMTC the bytes, URL or file
/// alike: SMTC fetching a URL itself shows nothing for an unpackaged desktop
/// app. An answer that lands after the track moved on is dropped, so a slow
/// cover cannot replace the next one.
fn load_thumbnail(art: String) {
    if let Ok(mut cache) = THUMBNAIL.lock() {
        *cache = Some((art.clone(), None));
    }
    std::thread::spawn(move || {
        let Some(stream_ref) =
            fetch_artwork_bytes(&art).and_then(|bytes| stream_ref_from_bytes(&bytes))
        else {
            tracing::debug!("SMTC artwork could not be loaded");
            return;
        };
        let Ok(mut cache) = THUMBNAIL.lock() else {
            return;
        };
        if cache.as_ref().is_none_or(|(key, _)| *key != art) {
            return;
        }
        *cache = Some((art, Some(stream_ref.clone())));
        drop(cache);
        if let Some(updater) = current_smtc().and_then(|smtc| smtc.DisplayUpdater().ok()) {
            let _ = updater.SetThumbnail(&stream_ref);
            let _ = updater.Update();
        }
    });
}

pub fn update_now_playing(
    title: &str,
    artist: &str,
    album: &str,
    duration: f64,
    position: f64,
    playing: bool,
    artwork_path: Option<&str>,
) {
    if let Ok(mut last) = LAST_NOW_PLAYING.lock() {
        *last = Some(NowPlaying {
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            duration,
            position,
            playing,
            artwork: artwork_path.map(str::to_string),
        });
    }

    // init in case init() wasn't called before the first track plays; the
    // state just stored is replayed once SMTC is up.
    init();

    let Some(smtc) = current_smtc() else { return };

    let _ = smtc.SetPlaybackStatus(if playing {
        MediaPlaybackStatus::Playing
    } else {
        MediaPlaybackStatus::Paused
    });

    if let Ok(updater) = smtc.DisplayUpdater() {
        let _ = updater.SetType(MediaPlaybackType::Music);
        if let Ok(props) = updater.MusicProperties() {
            let _ = props.SetTitle(&windows::core::HSTRING::from(title));
            let _ = props.SetArtist(&windows::core::HSTRING::from(artist));
            let _ = props.SetAlbumTitle(&windows::core::HSTRING::from(album));
        }

        if let Some(art) = artwork_path.filter(|art| !art.is_empty()) {
            match cached_thumbnail(art) {
                Some(Some(stream_ref)) => {
                    let _ = updater.SetThumbnail(&stream_ref);
                }
                Some(None) => {}
                None => load_thumbnail(art.to_string()),
            }
        }

        let _ = updater.Update();
    }

    let app_hwnd = APP_HWND.load(Ordering::Acquire);
    if app_hwnd != 0 {
        setup_taskbar_buttons(HWND(app_hwnd as _), playing);
    }

    if duration > 0.0
        && let Ok(timeline) = SystemMediaTransportControlsTimelineProperties::new()
    {
        let _ = timeline.SetStartTime(secs_to_timespan(0.0));
        let _ = timeline.SetEndTime(secs_to_timespan(duration));
        let _ = timeline.SetPosition(secs_to_timespan(position));
        let _ = timeline.SetMinSeekTime(secs_to_timespan(0.0));
        let _ = timeline.SetMaxSeekTime(secs_to_timespan(duration));
        let _ = smtc.UpdateTimelineProperties(&timeline);
    }
}
