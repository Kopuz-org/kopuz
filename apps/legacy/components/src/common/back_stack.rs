//! What Android's back gesture closes before it leaves the page.
//!
//! Modals and prompts mount a [`BackDismiss`] while they are open, and so does
//! selection mode, as a background layer. Back closes the most recently
//! mounted modal first, so a half-filled dialog is dismissed instead of the
//! page behind it navigating away. Background layers sit in the page itself,
//! so they wait until the shell's own surfaces (quick search, the player) have
//! closed over them.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::prelude::*;

struct Layer {
    id: u64,
    background: bool,
    close: Callback<()>,
}

thread_local! {
    static LAYERS: RefCell<Vec<Layer>> = const { RefCell::new(Vec::new()) };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn dismiss(background: bool) -> bool {
    let top = LAYERS.with(|layers| {
        layers
            .borrow()
            .iter()
            .rev()
            .find(|layer| layer.background == background)
            .map(|layer| layer.close)
    });
    match top {
        Some(close) => {
            close.call(());
            true
        }
        None => false,
    }
}

/// Close the topmost open modal or prompt, reporting whether there was one.
pub fn dismiss_top() -> bool {
    dismiss(false)
}

/// Close the topmost background layer (selection mode), reporting whether
/// there was one. For after the shell has nothing of its own left to close.
pub fn dismiss_background() -> bool {
    dismiss(true)
}

/// Registers `on_back` as the handler for the back gesture while mounted.
/// Render it inside whatever is open: `if editing { BackDismiss { .. } }`.
/// `background` marks state of the page itself rather than something drawn
/// over it.
#[component]
pub fn BackDismiss(on_back: Callback<()>, #[props(default)] background: bool) -> Element {
    let id = use_hook(|| {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        LAYERS.with(|layers| {
            layers.borrow_mut().push(Layer {
                id,
                background,
                close: on_back,
            })
        });
        id
    });
    // Props can be replaced by a fresh closure on re-render; keep the stored
    // handler pointing at the current one.
    LAYERS.with(|layers| {
        if let Some(layer) = layers.borrow_mut().iter_mut().find(|layer| layer.id == id) {
            layer.close = on_back;
        }
    });
    use_drop(move || {
        LAYERS.with(|layers| layers.borrow_mut().retain(|layer| layer.id != id));
    });
    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_with_nothing_open_is_left_to_navigation() {
        assert!(!dismiss_top());
        assert!(!dismiss_background());
    }
}
