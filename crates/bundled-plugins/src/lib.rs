//! The official plugins under `plugins/`, compiled to WebAssembly by this
//! crate's build script and embedded, so they work offline and out of the box.
//!
//! Set `KOPUZ_PLUGIN_WASM_DIR` to a directory of prebuilt `<package>.wasm`
//! files to skip the nested build, as a sandboxed packager would.

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

/// Every official plugin, manifest and component.
pub fn all() -> &'static [plugin_host::Bundled] {
    BUNDLED
}
