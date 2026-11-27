//! Runs kopuz plugins: a `kopuz-plugin.toml` manifest plus a component built
//! against `wit/` in this crate, or a native plugin whose code the host supplies.
//! A plugin is a source, opened per signed-in connection, or an integration,
//! opened once from the fields the person filled in. A component's only ways
//! out are the `http` and `tcp` imports, checked against its manifest. This
//! crate knows nothing of kopuz's models.

pub mod folders;
mod host;
mod integration;
pub mod manifest;
pub mod market;
mod plugin;
mod registry;
mod session;

pub use host::{Decryption, Opener};
pub use integration::Integration;
pub use plugin::{Code, Origin, Plugin};
pub use registry::{
    Bundled, COMPONENT_FILE, MANIFEST_FILE, Pending, Registry, host_set, plugin_dirs,
};
pub use session::Session;

mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "source-plugin",
        imports: { default: async },
        exports: { default: async },
    });
}

// Its imports are the source world's, so one host implementation serves both.
mod integration_bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "integration-plugin",
        imports: { default: async },
        exports: { default: async },
        with: {
            "kopuz:plugin/types": crate::bindings::kopuz::plugin::types,
            "kopuz:plugin/http": crate::bindings::kopuz::plugin::http,
            "kopuz:plugin/log": crate::bindings::kopuz::plugin::log,
            "kopuz:plugin/tcp": crate::bindings::kopuz::plugin::tcp,
        },
    });
}

pub mod wit {
    pub use crate::bindings::kopuz::plugin::types::*;
    pub use crate::integration_bindings::exports::kopuz::plugin::integration::Setting;
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
