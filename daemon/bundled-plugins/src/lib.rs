//! The sandboxed official plugins under `plugins/`, built and embedded.
//! `KOPUZ_PLUGIN_WASM_DIR` names prebuilt `<package>.wasm` files instead.

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

pub fn all() -> &'static [plugin_host::Bundled] {
    BUNDLED
}

/// The example plugins under `plugins/examples/`, for tests; empty unless the
/// crate is built with its `examples` feature.
pub fn examples() -> &'static [plugin_host::Bundled] {
    EXAMPLES
}
