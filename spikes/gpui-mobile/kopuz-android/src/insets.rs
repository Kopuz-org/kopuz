//! What the app's phone layout (`theme::mobile`) follows from the system: the safe-area insets, the
//! soft keyboard's height, and the IME being closed with back.
//!
//! gpui-pre-mobile updates the platform window's insets from the activity's content rect on
//! `InitWindow` and `WindowResized`, after it has fired GPUI's resize callback, so a GPUI bounds
//! observer would read the old ones. The keyboard's height arrives from the host activity on its UI
//! thread. So the follower polls both, as `lifecycle.rs` polls the surface, and hands a change to
//! `theme::mobile::set_mobile`, which redraws the shell.
//!
//! Lock rule (see `lifecycle.rs`): `primary_window()` locks the platform state, so the window is
//! looked up once, from the `Application::run` callback. `safe_area_insets_logical` locks only the
//! window's own state, which nothing holds while tasks run, so the task may read it.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyWindowHandle, App, Pixels, px};
use gpui_mobile::android::jni;
use gpui_mobile::android::window::AndroidWindow;
use theme::mobile::{Host, Insets, Mobile};

/// Short enough that the layout keeps pace with the keyboard sliding in.
const TICK: Duration = Duration::from_millis(50);

/// The platform window's safe-area insets and the keyboard's height, in logical pixels. Some phones
/// shrink the content rect for the keyboard, which makes it the bottom inset too; then the bottom is
/// `resting`, the system bars' inset last seen with the keyboard down.
fn read(window: Option<&AndroidWindow>, resting: &mut Pixels) -> Mobile {
    let insets = window
        .map(|window| {
            let insets = window.safe_area_insets_logical();
            Insets {
                top: px(insets.top),
                bottom: px(insets.bottom),
                left: px(insets.left),
                right: px(insets.right),
            }
        })
        .unwrap_or_default();
    let keyboard = px(gpui_mobile::keyboard_height());
    let mut insets = insets;
    if keyboard > px(0.) && insets.bottom + px(1.) >= keyboard {
        insets.bottom = *resting;
    } else if keyboard <= px(0.) {
        *resting = insets.bottom;
    }
    Mobile { insets, keyboard }
}

fn log(reason: &str, mobile: Mobile) {
    let insets = mobile.insets;
    log::info!(
        "KOPUZ insets {reason} top={:?} bottom={:?} left={:?} right={:?} keyboard={:?}",
        insets.top,
        insets.bottom,
        insets.left,
        insets.right,
        mobile.keyboard
    );
}

/// The platform window, looked up while no platform lock is held.
pub struct Follower {
    window: Option<Arc<AndroidWindow>>,
}

impl Follower {
    /// Call from the `Application::run` callback, never from a task. gpui-pre-mobile opens its
    /// one window on `InitWindow`, before that callback, and keeps it across surface loss.
    pub fn new() -> Self {
        let window = jni::platform().and_then(|platform| platform.primary_window());
        if window.is_none() {
            log::error!("KOPUZ error insets: no platform window");
        }
        Self { window }
    }

    /// Install the phone layout as it is now, and the host's services: before the main window
    /// opens, so its first frame is already the phone's.
    pub fn install(&self, cx: &mut App) {
        let mobile = read(self.window.as_deref(), &mut px(0.));
        log("initial", mobile);
        theme::mobile::set_mobile(mobile, cx);
        theme::mobile::set_host(
            Host {
                minimize: Rc::new(|| {
                    if let Err(error) = jni::move_task_to_back() {
                        log::error!("KOPUZ error minimize: {error}");
                    }
                }),
            },
            cx,
        );
    }

    /// Follow the system through rotations, surface changes and the keyboard, redrawing `handle`.
    /// Back closing the keyboard blurs the input, so its next tap opens the keyboard again.
    pub fn spawn(self, cx: &mut App, handle: AnyWindowHandle) {
        let window = self.window;
        let mut shown = cx.try_global::<Mobile>().copied().unwrap_or_default();
        let mut resting = shown.insets.bottom;
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if gpui_mobile::android::take_ime_dismissed() {
                    let blurred = handle.update(cx, |_, window, cx| window.blur(cx));
                    if let Err(error) = blurred {
                        log::error!("KOPUZ error blur: {error}");
                    }
                }
                let now = read(window.as_deref(), &mut resting);
                if now == shown {
                    continue;
                }
                shown = now;
                log("changed", now);
                let updated = handle.update(cx, |_, window, cx| {
                    theme::mobile::set_mobile(now, cx);
                    window.refresh();
                });
                if let Err(error) = updated {
                    log::error!("KOPUZ error insets update: {error}");
                    return;
                }
            }
        })
        .detach();
    }
}
