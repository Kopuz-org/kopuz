//! FFI (Foreign Function Interface) module for iOS.
//!
//! This module exposes C-compatible functions that can be called from
//! Objective-C code in the iOS app delegate to initialize and control
//! the GPUI application lifecycle.
//!
//! ## Typical call sequence from Obj-C
//!
//! ```text
//! gpui_ios_run_demo()          // sets up platform + invokes finish-launching
//! gpui_ios_get_window()        // retrieve the GPUI window pointer
//! gpui_ios_set_frame_waker(ptr, waker, ctx) // optional: lets the host pause its display link
//! gpui_ios_request_frame(ptr)  // called every CADisplayLink tick; returns whether GPUI wants another
//! ```

use gpui::{App, AppContext, Application, RequestFrameOptions, WindowOptions};
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::OnceLock;

/// Global storage for the GPUI application state.
/// This is set during initialization and used by FFI callbacks.
static IOS_APP_STATE: OnceLock<IosAppState> = OnceLock::new();

/// Holds the state needed for iOS FFI callbacks.
/// Note: On iOS, all UI code runs on the main thread, so we use a RefCell
/// instead of Mutex and don't require Send.
struct IosAppState {
    /// The callback to invoke when the app finishes launching.
    /// This is the closure passed to Application::run().
    /// Using std::cell::UnsafeCell since this is only accessed from the main thread.
    finish_launching: std::cell::UnsafeCell<Option<Box<dyn FnOnce()>>>,
}

// Safety: On iOS, all GPUI operations happen on the main thread.
// The FFI functions are only called from the iOS app delegate which runs on main thread.
// We implement both Send and Sync because OnceLock requires Send for its value type,
// and we need Sync for the static. The actual access is always single-threaded.
unsafe impl Send for IosAppState {}
unsafe impl Sync for IosAppState {}

// Safety wrapper for window list - only accessed from main thread
pub(crate) struct WindowListWrapper(
    pub(crate) std::cell::UnsafeCell<Vec<*const super::window::IosWindow>>,
);
unsafe impl Send for WindowListWrapper {}
unsafe impl Sync for WindowListWrapper {}

pub(crate) static IOS_WINDOW_LIST: OnceLock<WindowListWrapper> = OnceLock::new();

/// Initialize the GPUI iOS application.
///
/// This should be called from `application:didFinishLaunchingWithOptions:`
/// in the iOS app delegate, before any other GPUI functions.
///
/// Returns a pointer to the app state that should be passed to other FFI functions.
/// Returns null if initialization fails.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_initialize() -> *mut c_void {
    log::info!("GPUI iOS: Initializing");

    // Initialize the app state
    let state = IosAppState {
        finish_launching: std::cell::UnsafeCell::new(None),
    };

    if IOS_APP_STATE.set(state).is_err() {
        log::error!("GPUI iOS: Already initialized");
        return std::ptr::null_mut();
    }

    // Initialize the window list
    let _ = IOS_WINDOW_LIST.set(WindowListWrapper(std::cell::UnsafeCell::new(Vec::new())));

    // Return a non-null pointer to indicate success
    // The actual state is stored in the static
    std::ptr::dangling_mut::<c_void>()
}

/// Register a window with the FFI layer.
///
/// This is called internally when a new IosWindow is created.
/// The window pointer can then be retrieved by Objective-C code.
///
/// # Safety
/// This must only be called from the main thread.
pub(crate) fn register_window(window: *const super::window::IosWindow) {
    if let Some(wrapper) = IOS_WINDOW_LIST.get() {
        unsafe {
            (*wrapper.0.get()).push(window);
            log::info!("GPUI iOS: Registered window {:p}", window);
        }
    }
}

/// Get the most recently created window pointer.
///
/// Returns the pointer to the IosWindow that was most recently registered,
/// or null if no windows have been created.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_get_window() -> *mut c_void {
    if let Some(wrapper) = IOS_WINDOW_LIST.get() {
        unsafe {
            let windows = &*wrapper.0.get();
            if let Some(&window) = windows.last() {
                log::info!("GPUI iOS: Returning window {:p}", window);
                return window as *mut c_void;
            }
        }
    }
    log::warn!("GPUI iOS: No windows registered");
    std::ptr::null_mut()
}

/// Store the finish launching callback.
///
/// This is called internally by IosPlatform::run() to store the callback
/// that will be invoked when the app finishes launching.
///
/// # Safety
/// This must only be called from the main thread.
pub(crate) fn set_finish_launching_callback(callback: Box<dyn FnOnce()>) {
    if let Some(state) = IOS_APP_STATE.get() {
        // Safety: Only called from main thread
        unsafe {
            *state.finish_launching.get() = Some(callback);
        }
    }
}

/// Called when the iOS app has finished launching.
///
/// This should be called from `application:didFinishLaunchingWithOptions:`
/// in the iOS app delegate, after `gpui_ios_initialize()` returns.
///
/// This invokes the callback passed to Application::run().
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_did_finish_launching(_app_ptr: *mut c_void) {
    log::info!("GPUI iOS: Did finish launching");

    if let Some(state) = IOS_APP_STATE.get() {
        // Safety: Only called from main thread
        let callback = unsafe { (*state.finish_launching.get()).take() };
        if let Some(callback) = callback {
            log::info!("GPUI iOS: Invoking finish launching callback");
            callback();
        } else {
            log::warn!("GPUI iOS: No finish launching callback registered");
        }
    } else {
        log::error!("GPUI iOS: Not initialized");
    }
}

/// Called when the iOS app will enter the foreground.
///
/// This should be called from `applicationWillEnterForeground:` in the app delegate.
/// This notifies all GPUI windows that the app is becoming active.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_will_enter_foreground(_app_ptr: *mut c_void) {
    log::info!("GPUI iOS: Will enter foreground");

    // Notify all windows that they're becoming active
    if let Some(wrapper) = IOS_WINDOW_LIST.get() {
        unsafe {
            let windows = &*wrapper.0.get();
            for &window_ptr in windows.iter() {
                if !window_ptr.is_null() {
                    let window = &*window_ptr;
                    window.notify_active_status_change(true);
                }
            }
        }
    }
}

/// Called when the iOS app did become active.
///
/// This should be called from `applicationDidBecomeActive:` in the app delegate.
/// This indicates the app is now in the foreground and receiving events.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_did_become_active(_app_ptr: *mut c_void) {
    log::info!("GPUI iOS: Did become active");

    // App is now fully active - windows should be notified
    if let Some(wrapper) = IOS_WINDOW_LIST.get() {
        unsafe {
            let windows = &*wrapper.0.get();
            for &window_ptr in windows.iter() {
                if !window_ptr.is_null() {
                    let window = &*window_ptr;
                    window.notify_active_status_change(true);
                }
            }
        }
    }
}

/// Called when the iOS app will resign active.
///
/// This should be called from `applicationWillResignActive:` in the app delegate.
/// This indicates the app is about to become inactive (e.g., incoming call, switching apps).
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_will_resign_active(_app_ptr: *mut c_void) {
    log::info!("GPUI iOS: Will resign active");

    // App is about to become inactive
    if let Some(wrapper) = IOS_WINDOW_LIST.get() {
        unsafe {
            let windows = &*wrapper.0.get();
            for &window_ptr in windows.iter() {
                if !window_ptr.is_null() {
                    let window = &*window_ptr;
                    window.notify_active_status_change(false);
                }
            }
        }
    }
}

/// Called when the iOS app did enter the background.
///
/// This should be called from `applicationDidEnterBackground:` in the app delegate.
/// At this point, the app should have already saved any user data and released
/// shared resources. The app will be suspended shortly after this returns.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_did_enter_background(_app_ptr: *mut c_void) {
    log::info!("GPUI iOS: Did enter background");

    // Notify windows they're no longer visible
    if let Some(wrapper) = IOS_WINDOW_LIST.get() {
        unsafe {
            let windows = &*wrapper.0.get();
            for &window_ptr in windows.iter() {
                if !window_ptr.is_null() {
                    let window = &*window_ptr;
                    window.notify_active_status_change(false);
                }
            }
        }
    }
}

/// Called when the iOS app will terminate.
///
/// This should be called from `applicationWillTerminate:` in the app delegate.
/// This is a good place to save any unsaved data.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_will_terminate(_app_ptr: *mut c_void) {
    log::info!("GPUI iOS: Will terminate");

    // Quit callbacks would be invoked here if registered.
}

/// Called when a touch event occurs.
///
/// This bridges UIKit touch events to GPUI's input system.
/// Parameters:
/// - `window_ptr`: Pointer to the IosWindow
/// - `touch_ptr`: Pointer to the UITouch object
/// - `event_ptr`: Pointer to the UIEvent object
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_handle_touch(
    window_ptr: *mut c_void,
    touch_ptr: *mut c_void,
    event_ptr: *mut c_void,
) {
    if window_ptr.is_null() || touch_ptr.is_null() {
        return;
    }

    // Cast to IosWindow and forward the touch event
    let window = unsafe { &*(window_ptr as *const super::window::IosWindow) };
    window.handle_touch(
        touch_ptr as *mut objc2::runtime::AnyObject,
        event_ptr as *mut objc2::runtime::AnyObject,
    );
}

/// Give GPUI a frame. Returns whether GPUI wants another one.
///
/// Call this from the `CADisplayLink` callback. GPUI draws only if something
/// changed; the return value says whether more frames are wanted (a view was
/// notified during this one, an animation asked for the next frame, text
/// input is waiting). A host that registered a waker with
/// [`gpui_ios_set_frame_waker`] can pause its display link on `false` and
/// resume it from the waker; a host that ticks unconditionally may ignore
/// the return value. A null `window_ptr` returns `false`: there is no window
/// to draw, so a pausing host stays paused until it registers a real one.
///
/// The window_ptr should be the value returned by gpui_ios_get_window().
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_request_frame(window_ptr: *mut c_void) -> bool {
    if window_ptr.is_null() {
        return false;
    }

    // Safety: window_ptr must be a valid pointer to an IosWindow
    let window = unsafe { &*(window_ptr as *const super::window::IosWindow) };

    // Check if text input arrived since last frame — if so, force a render
    // so drain_pending_text() runs and the UI updates.
    let text_dirty = crate::TEXT_INPUT_DIRTY.swap(false, std::sync::atomic::Ordering::AcqRel);

    // Demand raised during the frame (a view notified mid-draw, an animation
    // asking for its next frame) must survive into the return value, so the
    // slate is cleared before the frame, not after.
    window.frame_demand.clear();

    // Take the callback, invoke it, then restore it
    // We must complete the borrow before invoking the callback,
    // as the callback might try to borrow the same RefCell
    let callback = window.request_frame_callback.borrow_mut().take();
    if let Some(mut cb) = callback {
        cb(RequestFrameOptions {
            force_render: text_dirty,
            ..Default::default()
        });
        // Restore the callback for the next frame
        window.request_frame_callback.borrow_mut().replace(cb);
    }

    window.frame_demand.is_pending()
        || crate::TEXT_INPUT_DIRTY.load(std::sync::atomic::Ordering::Acquire)
}

/// Register the host callback that resumes frame delivery.
///
/// GPUI calls `waker(context)` on the main thread whenever it wants a frame
/// and the host may have stopped ticking: after `gpui_ios_request_frame`
/// returned `false`, a notified view, an animation, arriving text input.
/// Typically the callback un-pauses the `CADisplayLink`. Pass a null `waker`
/// to clear it; the host must keep `context` valid until then.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_set_frame_waker(
    window_ptr: *mut c_void,
    waker: Option<unsafe extern "C" fn(*mut c_void)>,
    context: *mut c_void,
) {
    if window_ptr.is_null() {
        return;
    }
    // Safety: window_ptr must be a valid pointer to an IosWindow
    let window = unsafe { &*(window_ptr as *const super::window::IosWindow) };
    window
        .frame_demand
        .set_host_waker(waker.map(|waker| (waker, context)));
}

/// Ask every registered window for a frame. Used for demand that arrives
/// outside GPUI, such as text from the software keyboard.
pub(crate) fn wake_windows() {
    if let Some(wrapper) = IOS_WINDOW_LIST.get() {
        unsafe {
            for &window in (*wrapper.0.get()).iter() {
                if !window.is_null() {
                    (*window).frame_demand.wake();
                }
            }
        }
    }
}

/// Show the software keyboard.
///
/// Call this when a text input field gains focus.
/// The window_ptr should be the value returned by gpui_ios_get_window().
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_show_keyboard(window_ptr: *mut c_void) {
    if window_ptr.is_null() {
        return;
    }

    log::info!("GPUI iOS: Show keyboard requested");

    let window = unsafe { &*(window_ptr as *const super::window::IosWindow) };
    window.show_keyboard_with_type(crate::KeyboardType::Default);
}

/// Hide the software keyboard.
///
/// Call this when a text input field loses focus.
/// The window_ptr should be the value returned by gpui_ios_get_window().
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_hide_keyboard(window_ptr: *mut c_void) {
    if window_ptr.is_null() {
        return;
    }

    log::info!("GPUI iOS: Hide keyboard requested");

    let window = unsafe { &*(window_ptr as *const super::window::IosWindow) };
    window.hide_keyboard();
}

/// Handle text input from the software keyboard.
///
/// This is called when the user types on the keyboard.
/// Parameters:
/// - `window_ptr`: Pointer to the IosWindow
/// - `text_ptr`: Pointer to NSString with the entered text
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_handle_text_input(window_ptr: *mut c_void, text_ptr: *mut c_void) {
    if window_ptr.is_null() || text_ptr.is_null() {
        return;
    }

    log::info!("GPUI iOS: Handle text input");

    let window = unsafe { &*(window_ptr as *const super::window::IosWindow) };
    window.handle_text_input(text_ptr as *mut objc2::runtime::AnyObject);
}

/// Handle a key event from an external keyboard.
///
/// Parameters:
/// - `window_ptr`: Pointer to the IosWindow
/// - `key_code`: The key code from UIKeyboardHIDUsage
/// - `modifiers`: Modifier flags from UIKeyModifierFlags
/// - `is_key_down`: true for key down, false for key up
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_handle_key_event(
    window_ptr: *mut c_void,
    key_code: u32,
    modifiers: u32,
    is_key_down: bool,
) {
    if window_ptr.is_null() {
        return;
    }

    log::info!(
        "GPUI iOS: Handle key event - code: {}, modifiers: {}, down: {}",
        key_code,
        modifiers,
        is_key_down
    );

    let window = unsafe { &*(window_ptr as *const super::window::IosWindow) };
    window.handle_key_event(key_code, modifiers, is_key_down);
}

/// Called from ObjC when the app receives a URL to open.
///
/// Parameters:
/// - url_ptr: Pointer to an NSString containing the URL
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_handle_open_url(url_ptr: *mut c_void) {
    if url_ptr.is_null() {
        return;
    }

    let url_string = unsafe {
        use objc2::msg_send;
        let ns_str = url_ptr as *mut objc2::runtime::AnyObject;
        let cstr: *const std::ffi::c_char = msg_send![ns_str, UTF8String];
        if cstr.is_null() {
            return;
        }
        std::ffi::CStr::from_ptr(cstr)
            .to_string_lossy()
            .into_owned()
    };

    log::info!("GPUI iOS: Received deep link: {}", url_string);

    #[cfg(feature = "deeplink")]
    {
        crate::packages::deeplink::ios::handle_open_url(url_string);
    }
}

// ── App callback storage ─────────────────────────────────────────────────────

/// Wrapper around an `UnsafeCell<Option<Box<dyn FnOnce(&mut App)>>>`.
///
/// # Safety
/// On iOS all UI work happens on the main thread.  The FFI entry points
/// (`set_app_callback`, `run_app`, `gpui_ios_run_demo`) are only ever
/// called from the main thread, so interior-mutable access is safe.
#[allow(clippy::type_complexity)]
struct AppCallbackCell(std::cell::UnsafeCell<Option<Box<dyn FnOnce(&mut App)>>>);

// Safety: only accessed from the iOS main thread.
unsafe impl Send for AppCallbackCell {}
unsafe impl Sync for AppCallbackCell {}

static APP_CALLBACK: OnceLock<AppCallbackCell> = OnceLock::new();

thread_local! {
    // UIKit owns the run loop. Keep GPUI alive after the launch callback returns
    // so native window pointers and their callbacks remain valid.
    static IOS_APPLICATION: std::cell::RefCell<Option<gpui::ApplicationHandle>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Register a callback that will be invoked inside `Application::run`.
///
/// This must be called **before** [`run_app`] so that the run-loop
/// has something to do (open a window, create views, etc.).
///
/// # Safety
/// Must be called from the main thread only.
pub fn set_app_callback(cb: Box<dyn FnOnce(&mut App)>) {
    let cell = APP_CALLBACK.get_or_init(|| AppCallbackCell(std::cell::UnsafeCell::new(None)));
    unsafe {
        *cell.0.get() = Some(cb);
    }
}

#[allow(clippy::type_complexity)]
fn take_app_callback() -> Option<Box<dyn FnOnce(&mut App)>> {
    APP_CALLBACK
        .get()
        .and_then(|cell| unsafe { (*cell.0.get()).take() })
}

/// C entry point called from `main.m`'s app delegate.
///
/// Consumer crates should call [`set_app_callback`] **before** this function
/// to register their root view.  If no callback is registered an empty
/// window is opened as a fallback.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_run_demo() {
    run_app();
}

/// Run the GPUI iOS application.
///
/// This initialises the platform, creates the `Application`, and enters the
/// GPUI run loop.  The actual UI is determined by a callback previously
/// registered via [`set_app_callback`].  If no callback was registered a
/// default empty window is opened so the app doesn't crash.
pub fn run_app() {
    log::info!("GPUI iOS: Starting application");

    // Initialise the FFI layer if not already done.
    if IOS_APP_STATE.get().is_none() {
        let state = IosAppState {
            finish_launching: std::cell::UnsafeCell::new(None),
        };
        let _ = IOS_APP_STATE.set(state);
        let _ = IOS_WINDOW_LIST.set(WindowListWrapper(std::cell::UnsafeCell::new(Vec::new())));
    }

    let platform = Rc::new(super::IosPlatform::new());
    let application = Application::with_platform(platform).run_embedded(|cx: &mut App| {
        if let Some(cb) = take_app_callback() {
            log::info!("GPUI iOS: Invoking user-provided app callback");
            cb(cx);
        } else {
            log::warn!("GPUI iOS: No app callback registered — opening default empty window");
            cx.open_window(
                WindowOptions {
                    window_bounds: None,
                    ..Default::default()
                },
                |_, cx| cx.new(|_| gpui::Empty),
            )
            .expect("Failed to open default window");
            cx.activate(true);
        }
    });

    IOS_APPLICATION.with(|slot| *slot.borrow_mut() = Some(application));

    // On iOS, run_embedded() stores the callback and returns immediately.
    // The finish-launching callback is forwarded to set_finish_launching_callback
    // and invoked here synchronously (in a real app the app delegate does this).
    if let Some(state) = IOS_APP_STATE.get() {
        let callback = unsafe { (*state.finish_launching.get()).take() };
        if let Some(callback) = callback {
            log::info!("GPUI iOS: Invoking Application::run callback");
            callback();
        }
    }
}

// Embedded mode is configured on the UIKit main thread before startup.
thread_local! {
    static EMBEDDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn is_embedded() -> bool {
    EMBEDDED.with(|value| value.get())
}

/// Select view embedding before starting the application (main thread only).
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_set_embedded() {
    EMBEDDED.with(|value| value.set(true));
}

/// Borrow the GPUI controller for UIKit child-controller containment.
/// The application must remain alive while the host uses this controller.
///
/// # Safety
/// Call on the main thread with null or a live pointer from `gpui_ios_get_window`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpui_ios_view_controller(window: *mut c_void) -> *mut c_void {
    if window.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        (&*(window as *const super::window::IosWindow))
            .view_controller_ptr()
            .cast()
    }
}

/// Forward host layout changes after assigning the embedded view's bounds.
///
/// # Safety
/// Call on the main thread with null or a live pointer from `gpui_ios_get_window`.
/// Do not call re-entrantly from a GPUI update.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpui_ios_layout_view(window: *mut c_void) {
    if !window.is_null() {
        unsafe {
            (&*(window as *const super::window::IosWindow)).handle_layout_change();
        }
    }
}
