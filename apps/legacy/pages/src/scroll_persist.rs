//! Per-view scroll-position memory for pages whose list scrolls inside their own
//! inner container (Artists, Albums) rather than the shell's `main-scroll-area`.
//! The shell only persists scroll for elements it owns, so those inner scrollers
//! reset to the top every visit. This keeps the last position per string key,
//! process-lifetime, restored once after the list first renders.
//!
//! UI runs single-threaded, so a `thread_local` map is enough — no locking, no
//! cross-crate context plumbing.

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static POSITIONS: RefCell<HashMap<String, f64>> = RefCell::new(HashMap::new());
}

/// Remember `top` as the scroll offset for `key` (called from `onscroll`).
pub fn save(key: &str, top: f64) {
    POSITIONS.with(|m| {
        m.borrow_mut().insert(key.to_string(), top);
    });
}

/// Last saved offset for `key`, or 0.0 if none.
pub fn get(key: &str) -> f64 {
    POSITIONS.with(|m| m.borrow().get(key).copied().unwrap_or(0.0))
}

/// JS that restores element `el_id`'s `scrollTop` to the saved offset for `key`
/// on the next frame (after the list has painted). No-op when nothing saved.
pub fn restore_eval(el_id: &str, key: &str) -> String {
    let pos = get(key);
    format!(
        "requestAnimationFrame(() => {{ let el = document.getElementById('{el_id}'); \
         if (el) el.scrollTop = {pos}; }});"
    )
}
