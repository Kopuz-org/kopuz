//! Runs kopuz plugins: WebAssembly components built against the WIT package
//! in `wit/`.
//!
//! A plugin is a component plus a `kopuz-plugin.toml` manifest. The manifest
//! says which services it serves, what each supports, and which hosts it may
//! reach; the component implements the `source-plugin` world. A plugin has no
//! files, sockets or environment of its own: its only way out is the `http`
//! import, checked against the manifest on every request and redirect.
//!
//! This crate knows nothing of kopuz's models. The server crate converts at
//! the boundary and keeps the database on its own side of it.

mod host;
pub mod manifest;
pub mod market;
mod plugin;
mod registry;
mod session;

pub use plugin::{Origin, Plugin};
pub use registry::{Bundled, COMPONENT_FILE, MANIFEST_FILE, Registry};
pub use session::Session;

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "source-plugin",
        imports: { default: async },
        exports: { default: async },
    });
}

/// The WIT types a plugin exchanges with the host.
pub mod wit {
    pub use crate::bindings::kopuz::plugin::types::*;
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PluginError {
    /// The engine itself could not start.
    #[error("the plugin runtime is unavailable: {0}")]
    Runtime(String),
    /// The component is not one this host can run.
    #[error("the plugin is not a valid component: {0}")]
    Invalid(String),
    /// The plugin crashed mid-call.
    #[error("the plugin crashed: {0}")]
    Trapped(String),
    /// The plugin ran past the longest a call may take, and was stopped.
    #[error("the plugin did not answer in time")]
    TimedOut,
}

impl PluginError {
    /// As the source error a caller reports, for paths that have no other
    /// way to say the plugin failed.
    pub fn into_source_error(self) -> wit::SourceError {
        wit::SourceError::Backend(self.to_string())
    }
}
