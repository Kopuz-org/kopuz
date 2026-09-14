//! What a managed settings file pins.
//!
//! A settings row renders locked when a config key is set by a file the app
//! must not write. The daemon reads those layers, so the list comes from it
//! rather than from the frontend opening the same files.

use dioxus::prelude::*;

use crate::api::use_api;

/// The keys a managed settings file owns. Empty when nothing is managed.
#[derive(Clone, Copy)]
pub struct LockedKeys(pub Signal<Vec<String>>);

impl LockedKeys {
    pub fn is_locked(&self, key: &str) -> bool {
        self.0.read().iter().any(|locked| locked == key)
    }

    pub fn any(&self) -> bool {
        !self.0.read().is_empty()
    }
}

/// Provide the locked-key list to the tree. Read once: a managed file is
/// managed for the life of the process.
pub fn use_locked_keys_provider() -> LockedKeys {
    let api = use_api();
    let keys = use_context_provider(|| LockedKeys(Signal::new(Vec::new())));
    use_future(move || {
        let api = api.clone();
        let mut signal = keys.0;
        async move {
            if let Ok(view) = api.config().await {
                signal.set(view.locked_keys);
            }
        }
    });
    keys
}
