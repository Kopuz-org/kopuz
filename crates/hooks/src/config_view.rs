//! What a managed settings file pins, as the daemon's view says; the app never opens those layers itself.

use config::SettingKey;
use dioxus::prelude::*;

/// The keys a managed settings file owns. Empty when nothing is managed.
#[derive(Clone, Copy)]
pub struct LockedKeys(pub Signal<Vec<SettingKey>>);

impl LockedKeys {
    pub fn is_locked(&self, key: SettingKey) -> bool {
        self.0.read().contains(&key)
    }

    /// For a row that names its key as stored, as a daemon field spec does.
    pub fn is_name_locked(&self, name: &str) -> bool {
        self.0.read().iter().any(|locked| locked.name() == name)
    }

    pub fn any(&self) -> bool {
        !self.0.read().is_empty()
    }
}
