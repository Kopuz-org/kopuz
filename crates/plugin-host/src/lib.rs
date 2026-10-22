//! Runs kopuz plugins: a `kopuz-plugin.toml` manifest plus a component built
//! against `wit/`, or a native plugin whose code the host supplies. A
//! component's only way out is the `http` import, checked against its
//! manifest. This crate knows nothing of kopuz's models.

mod host;
pub mod manifest;
pub mod market;
mod plugin;
mod registry;
mod session;

pub use plugin::{Code, Origin, Plugin};
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

pub mod wit {
    pub use crate::bindings::kopuz::plugin::types::*;
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PluginError {
    #[error("the plugin runtime is unavailable: {0}")]
    Runtime(String),
    #[error("the plugin is not a valid component: {0}")]
    Invalid(String),
    #[error("the plugin crashed: {0}")]
    Trapped(String),
    #[error("the plugin did not answer in time")]
    TimedOut,
}

impl PluginError {
    pub fn into_source_error(self) -> wit::SourceError {
        wit::SourceError::Backend(self.to_string())
    }
}
