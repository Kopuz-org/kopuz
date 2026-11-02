//! What a managed settings layer pins, as the daemon's view says; the app never opens those layers itself.

use api::{LockSource, SettingLock};
use config::SettingKey;
use dioxus::prelude::*;

/// The keys a managed settings layer owns, each with the layer that pins it. Empty when nothing is managed.
#[derive(Clone, Copy)]
pub struct LockedKeys(pub Signal<Vec<SettingLock>>);

impl LockedKeys {
    pub fn is_locked(&self, key: SettingKey) -> bool {
        self.0.read().iter().any(|lock| lock.key == key)
    }

    /// For a row that names its key as stored, as a daemon field spec does.
    pub fn is_name_locked(&self, name: &str) -> bool {
        self.lock_of_name(name).is_some()
    }

    /// The layer pinning the key stored under `name`, for a row to cite.
    pub fn lock_of_name(&self, name: &str) -> Option<LockSource> {
        self.0
            .read()
            .iter()
            .find(|lock| lock.key.name() == name)
            .map(|lock| lock.by.clone())
    }

    pub fn any(&self) -> bool {
        !self.0.read().is_empty()
    }
}
