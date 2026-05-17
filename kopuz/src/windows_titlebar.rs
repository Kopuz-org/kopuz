#![cfg(target_os = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::MulDiv;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, EnableMenuItem, GWL_STYLE, GWLP_WNDPROC, GetSystemMenu,
    GetWindowLongPtrW, GetWindowRect, HTCAPTION, HTCLIENT, HTMAXBUTTON, HTSYSMENU,
    MF_BYCOMMAND, MF_ENABLED, MF_GRAYED, SC_MAXIMIZE, SC_MOVE, SC_RESTORE, SC_SIZE,
    SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NCHITTEST,
    WM_NCRBUTTONUP, WM_SYSCOMMAND, WNDPROC, WS_MAXIMIZE,
};

static CUSTOM_TITLEBAR_ENABLED: AtomicBool = AtomicBool::new(false);
static SUBCLASS_STATE: OnceLock<Mutex<SubclassState>> = OnceLock::new();

#[derive(Default)]
struct SubclassState {
    hwnd: Option<isize>,
    prev_wndproc: Option<isize>,
}

const TITLEBAR_HEIGHT_DIP: i32 = 36;
const TITLEBAR_BUTTON_WIDTH_DIP: i32 = 44;
const TITLEBAR_BUTTON_COUNT: i32 = 3;

pub fn set_custom_titlebar_enabled(enabled: bool) {
    CUSTOM_TITLEBAR_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn install(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }

    let Ok(mut state) = subclass_state().lock() else {
        return;
    };

    if state.hwnd == Some(hwnd.0 as isize) && state.prev_wndproc.is_some() {
        return;
    }

    if let (Some(old_hwnd), Some(old_prev)) = (state.hwnd, state.prev_wndproc) {
        unsafe {
            let _ = SetWindowLongPtrW(HWND(old_hwnd as _), GWLP_WNDPROC, old_prev);
        }
    }

    let prev = unsafe {
        SetWindowLongPtrW(
            hwnd,
            GWLP_WNDPROC,
            titlebar_wndproc as usize as isize,
        )
    };

    if prev != 0 {
        state.hwnd = Some(hwnd.0 as isize);
        state.prev_wndproc = Some(prev);
    }
}

unsafe extern "system" fn titlebar_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            let prev = call_prev_wndproc(hwnd, msg, wparam, lparam);
            if !CUSTOM_TITLEBAR_ENABLED.load(Ordering::Relaxed) || prev.0 != HTCLIENT as isize {
                return prev;
            }

            if let Some(hit) = custom_titlebar_hit_test(hwnd, lparam) {
                return LRESULT(hit as isize);
            }

            prev
        }
        WM_NCRBUTTONUP => {
            if CUSTOM_TITLEBAR_ENABLED.load(Ordering::Relaxed)
                && matches!(wparam.0 as i32, HTCAPTION | HTSYSMENU)
            {
                show_system_menu(hwnd, lparam);
                return LRESULT(0);
            }

            call_prev_wndproc(hwnd, msg, wparam, lparam)
        }
        _ => call_prev_wndproc(hwnd, msg, wparam, lparam),
    }
}

fn custom_titlebar_hit_test(hwnd: HWND, lparam: LPARAM) -> Option<i32> {
    let mut window_rect = RECT::default();
    if unsafe { !GetWindowRect(hwnd, &mut window_rect).as_bool() } {
        return None;
    }

    let point = point_from_lparam(lparam);
    let dpi = unsafe { GetDpiForWindow(hwnd) as i32 };
    let titlebar_height = MulDiv(TITLEBAR_HEIGHT_DIP, dpi, 96);
    let button_width = MulDiv(TITLEBAR_BUTTON_WIDTH_DIP, dpi, 96);

    if point.y < window_rect.top || point.y >= window_rect.top + titlebar_height {
        return None;
    }

    let buttons_left = window_rect.right - button_width * TITLEBAR_BUTTON_COUNT;
    let max_left = window_rect.right - button_width * 2;
    let close_left = window_rect.right - button_width;

    if point.x >= max_left && point.x < close_left {
        return Some(HTMAXBUTTON);
    }

    if point.x < buttons_left {
        return Some(HTCAPTION);
    }

    None
}

fn show_system_menu(hwnd: HWND, lparam: LPARAM) {
    let menu = unsafe { GetSystemMenu(hwnd, false) };
    if menu.0.is_null() {
        return;
    }

    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    let is_maximized = style & WS_MAXIMIZE.0 as isize != 0;

    unsafe {
        let _ = EnableMenuItem(
            menu,
            SC_MAXIMIZE,
            MF_BYCOMMAND | if is_maximized { MF_GRAYED } else { MF_ENABLED },
        );
        let _ = EnableMenuItem(
            menu,
            SC_RESTORE,
            MF_BYCOMMAND | if is_maximized { MF_ENABLED } else { MF_GRAYED },
        );
        let _ = EnableMenuItem(
            menu,
            SC_MOVE,
            MF_BYCOMMAND | if is_maximized { MF_GRAYED } else { MF_ENABLED },
        );
        let _ = EnableMenuItem(
            menu,
            SC_SIZE,
            MF_BYCOMMAND | if is_maximized { MF_GRAYED } else { MF_ENABLED },
        );
    }

    let point = point_from_lparam(lparam);
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            None,
        )
    };

    if command != 0 {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                hwnd,
                WM_SYSCOMMAND,
                WPARAM(command as usize),
                LPARAM(0),
            );
        }
    }
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    let raw = lparam.0 as u32;
    POINT {
        x: (raw & 0xFFFF) as i16 as i32,
        y: ((raw >> 16) & 0xFFFF) as i16 as i32,
    }
}

fn call_prev_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let prev = {
        let Ok(state) = subclass_state().lock() else {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        };
        if state.hwnd != Some(hwnd.0 as isize) {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        state.prev_wndproc
    };

    let Some(prev) = prev else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };

    let prev_proc: WNDPROC = unsafe { std::mem::transmute(prev) };
    unsafe { CallWindowProcW(prev_proc, hwnd, msg, wparam, lparam) }
}

fn subclass_state() -> &'static Mutex<SubclassState> {
    SUBCLASS_STATE.get_or_init(|| Mutex::new(SubclassState::default()))
}
