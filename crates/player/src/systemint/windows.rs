// Windows system integration: System Media Transport Controls (SMTC),
// media keys, Now Playing info, and HWND discovery.
//
// Architecture:
// - COM must be initialized on the thread that uses WinRT APIs. Since the
//   Tokio thread pool does not call CoInitializeEx, setup runs on a
//   dedicated std::thread::spawn thread.
// - HWND discovery uses EnumWindows to find the process's visible window.
//   If none exists yet, a message-only window (HWND_MESSAGE) is created.
// - SMTC button events (play/pause/next/prev/seek) are forwarded to the
//   player via an unbounded mpsc channel.
// - CoInitializeEx + WinRT/COM FFI is documented with // SAFETY: invariants.

use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use windows::core::{BOOL, PCWSTR, Ref, w};
use windows::{
    Foundation::{TimeSpan, TypedEventHandler, Uri},
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
        System::Threading::GetCurrentProcessId,
        System::WinRT::RoGetActivationFactory,
        UI::{
            Shell::{
                ITaskbarList3, THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_ENABLED, THBN_CLICKED,
                THUMBBUTTON, TaskbarList,
            },
            WindowsAndMessaging::{
                CallWindowProcW, CreateWindowExW, DefWindowProcW, EnumWindows, GWLP_WNDPROC,
                GetWindowThreadProcessId, HICON, HWND_MESSAGE, IMAGE_ICON, IsWindowVisible,
                LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadImageW, SetWindowLongPtrW, WINDOW_EX_STYLE,
                WINDOW_STYLE, WM_COMMAND, WNDPROC,
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

static SMTC: OnceLock<SystemMediaTransportControls> = OnceLock::new();
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
    if let Some(rx) = EVENT_RECEIVER.get() {
        let mut guard = rx.lock().await;
        guard.recv().await
    } else {
        None
    }
}

// HWND discovery
struct EnumData {
    pid: u32,
    hwnd: HWND,
    // fallback for when no visible window exists yet
    any_hwnd: HWND,
}

// SAFETY:
// - This function matches the expected C callback signature for
//   EnumWindows (WNDENUMPROC). The system calls it for each top-level
//   window.
// - LPARAM contains a valid pointer to an EnumData struct allocated
//   on the stack in find_main_hwnd(). The reference does not escape
//   because EnumWindows calls this callback synchronously.
// - GetWindowThreadProcessId and IsWindowVisible are safe to call
//   with any valid HWND.
unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY:
    // - LPARAM is a valid pointer to an EnumData struct that lives
    //   on the caller's stack for the duration of EnumWindows.
    let data = unsafe { &mut *(lparam.0 as *mut EnumData) };
    let mut pid = 0u32;
    // SAFETY: GetWindowThreadProcessId is safe with a valid HWND
    // and a mutable output pointer.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == data.pid
    // SAFETY: IsWindowVisible is safe to call with any HWND.
    && unsafe { IsWindowVisible(hwnd).as_bool() }
    {
        data.hwnd = hwnd;
        BOOL(0) // stop enumeration
    } else {
        if pid == data.pid && data.any_hwnd.0.is_null() {
            data.any_hwnd = hwnd;
        }
        BOOL(1)
    }
}

fn create_message_window() -> Option<HWND> {
    // SAFETY:
    // - CreateWindowExW with HWND_MESSAGE creates a message-only window,
    //   which does not require a parent window or a window procedure.
    // - All parameters are well-formed: class name is "STATIC" (a
    //   standard Windows class), title is a valid wide string, and
    //   dimensions are zero (message windows have no visual presence).
    // - The return value is checked for null to ensure validity.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("KopuzSMTC"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
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
    if hwnd.0.is_null() || TASKBAR_HWND.load(Ordering::Acquire) == hwnd.0 as isize {
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

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            bases.push(exe_dir.join("assets").join("toolbar_icons"));
            bases.push(exe_dir.join("kopuz").join("assets").join("toolbar_icons"));
        }
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
    if hwnd.0.is_null() || hwnd == HWND_MESSAGE {
        return;
    }

    install_taskbar_subclass(hwnd);

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
            if TASKBAR_BUTTONS_ADDED.swap(true, Ordering::AcqRel) {
                taskbar.ThumbBarUpdateButtons(hwnd, &buttons)?;
            } else {
                taskbar.ThumbBarAddButtons(hwnd, &buttons)?;
            }
        }
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("[windows] Taskbar media buttons setup failed: {e:?}");
    }
}

fn find_main_hwnd() -> Option<HWND> {
    let mut data = EnumData {
        // SAFETY: GetCurrentProcessId is a simple system call that
        // always succeeds and requires no special setup.
        pid: unsafe { GetCurrentProcessId() },
        hwnd: HWND(std::ptr::null_mut()),
        any_hwnd: HWND(std::ptr::null_mut()),
    };

    // SAFETY:
    // - EnumWindows is safe to call with a valid callback pointer and
    //   a user-data parameter.
    // - enum_proc is a valid C callback matching the expected signature.
    // - LPARAM contains a valid pointer to `data`, which lives on the
    //   stack for the duration of the EnumWindows call.
    // - EnumWindows is synchronous, so the reference does not escape.
    let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut EnumData as isize)) };

    if !data.hwnd.0.is_null() {
        return Some(data.hwnd);
    }

    // hacky
    if !data.any_hwnd.0.is_null() {
        return Some(data.any_hwnd);
    }
    create_message_window()
}

// SMTC setup
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

fn setup_smtc(hwnd: HWND) {
    if SMTC.get().is_some() {
        return;
    }

    let result = (|| {
        // SAFETY:
        // - RoGetActivationFactory is a WinRT API that is safe to call
        //   after CoInitializeEx has been initialized on this thread.
        // - ISystemMediaTransportControlsInterop::GetForWindow is safe
        //   with a valid HWND (either a visible window or a message-only
        //   window).
        // - All subsequent SMTC method calls are thread-safe COM/WinRT
        //   operations that do not violate memory safety.
        // - The TypedEventHandler closures capture the sender by value
        //   and do not introduce data races.
        unsafe {
            let class_id =
                windows::core::HSTRING::from("Windows.Media.SystemMediaTransportControls");
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

            windows::core::Result::Ok(smtc)
        }
    })();

    match result {
        Ok(smtc) => {
            if SMTC.set(smtc).is_ok() {
                setup_taskbar_buttons(hwnd, false);
                println!("[windows] SMTC initialised");
            }
        }
        Err(e) => eprintln!("[windows] SMTC setup failed: {e:?}"),
    }
}

pub fn init() {
    if SMTC.get().is_some() {
        return;
    }
    static INIT_ONCE: OnceLock<()> = OnceLock::new();
    INIT_ONCE.get_or_init(|| {
        std::thread::spawn(|| {
            // CoInitializeEx must be called on the thread that uses WinRT/COM.
            // The tokio thread pool does not do this, so setup_smtc must run here.
            // SAFETY:
            // - CoInitializeEx initializes COM for the calling thread with
            //   the specified concurrency model (apartment-threaded).
            // - It is safe to call once per thread; subsequent calls return
            //   S_FALSE or RPC_E_CHANGED_MODE, which we ignore.
            // - The None parameter means we are not aggregating another
            //   COM object.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }

            match find_main_hwnd() {
                Some(hwnd) => setup_smtc(hwnd),
                None => eprintln!("[windows] Could not find main HWND for SMTC"),
            }
        });
    });
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

pub fn update_now_playing(
    title: &str,
    artist: &str,
    album: &str,
    _duration: f64,
    _position: f64,
    playing: bool,
    artwork_path: Option<&str>,
) {
    // init in case init() wasn't called before the first track plays
    if SMTC.get().is_none() {
        init();
    }

    let Some(smtc) = SMTC.get() else { return };

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

        if let Some(art) = artwork_path {
            if art.starts_with("http://") || art.starts_with("https://") {
                // Jellyfin: give the url directly to SMTC, it fetches lazily
                if let Ok(uri) = Uri::CreateUri(&windows::core::HSTRING::from(art)) {
                    if let Ok(stream_ref) = RandomAccessStreamReference::CreateFromUri(&uri) {
                        let _ = updater.SetThumbnail(&stream_ref);
                    }
                }
            } else {
                // Local: read bytes on a background thread, then apply thumbnail
                let art_owned = art.to_string();
                std::thread::spawn(move || {
                    if let Some(bytes) = fetch_artwork_bytes(&art_owned) {
                        if let Some(stream_ref) = stream_ref_from_bytes(&bytes) {
                            if let Some(smtc) = SMTC.get() {
                                if let Ok(updater) = smtc.DisplayUpdater() {
                                    let _ = updater.SetThumbnail(&stream_ref);
                                    let _ = updater.Update();
                                }
                            }
                        }
                    }
                });
            }
        }

        let _ = updater.Update();
    }

    let duration = _duration;
    let position = _position;
    if let Some(hwnd) = find_main_hwnd() {
        setup_taskbar_buttons(hwnd, playing);
    }

    if duration > 0.0 {
        if let Ok(timeline) = SystemMediaTransportControlsTimelineProperties::new() {
            let _ = timeline.SetStartTime(secs_to_timespan(0.0));
            let _ = timeline.SetEndTime(secs_to_timespan(duration));
            let _ = timeline.SetPosition(secs_to_timespan(position));
            let _ = timeline.SetMinSeekTime(secs_to_timespan(0.0));
            let _ = timeline.SetMaxSeekTime(secs_to_timespan(duration));
            let _ = smtc.UpdateTimelineProperties(&timeline);
        }
    }
}
