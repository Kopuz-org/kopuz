//! What Android's back gesture closes before it leaves the page.
//!
//! Modals, prompts and selection mode mount a [`BackDismiss`] while they are
//! open. Back closes the most recently mounted one, so a half-filled dialog is
//! dismissed instead of the page behind it navigating away.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::prelude::*;

thread_local! {
    static LAYERS: RefCell<Vec<(u64, Callback<()>)>> = const { RefCell::new(Vec::new()) };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// Close the topmost open layer, reporting whether there was one.
pub fn dismiss_top() -> bool {
    let top = LAYERS.with(|layers| layers.borrow().last().map(|(_, close)| *close));
    match top {
        Some(close) => {
            close.call(());
            true
        }
        None => false,
    }
}

/// Registers `on_back` as the handler for the back gesture while mounted.
/// Render it inside whatever is open: `if editing { BackDismiss { .. } }`.
#[component]
pub fn BackDismiss(on_back: Callback<()>) -> Element {
    let id = use_hook(|| {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        LAYERS.with(|layers| layers.borrow_mut().push((id, on_back)));
        id
    });
    // Props can be replaced by a fresh closure on re-render; keep the stored
    // handler pointing at the current one.
    LAYERS.with(|layers| {
        if let Some(entry) = layers.borrow_mut().iter_mut().find(|(at, _)| *at == id) {
            entry.1 = on_back;
        }
    });
    use_drop(move || {
        LAYERS.with(|layers| layers.borrow_mut().retain(|(at, _)| *at != id));
    });
    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_with_nothing_open_is_left_to_navigation() {
        assert!(!dismiss_top());
    }
}
