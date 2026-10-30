//! The Kopuz wire contract: tonic/prost types generated from
//! `proto/kopuz.proto`, plus lossless conversions to and from the
//! in-process `api` types. The daemon serves proto at its boundary and
//! thinks in `api` types everywhere else; wire clients do the reverse.
//! Each `convert` submodule carries the round-trip test that guards its
//! own types: every `api` value must survive api -> proto -> api unchanged.

mod generated {
    #![allow(clippy::large_enum_variant)]
    tonic::include_proto!("kopuz.v1");
}
pub use generated::*;

/// The encoded file descriptor set, for gRPC server reflection (the
/// `grpcurl` story).
pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("kopuz");

/// The contract revision this build speaks; it lives in `api` because the
/// daemon reports it without otherwise knowing the wire exists.
pub use api::WIRE_REVISION;

pub mod convert;
#[cfg(windows)]
pub mod pipe;
pub mod status;

/// The default socket or named pipe served by the desktop app and `kopuzd`.
#[cfg(windows)]
pub fn default_socket_path() -> Option<std::path::PathBuf> {
    pipe::default_name().ok().map(std::path::PathBuf::from)
}

/// The default socket served by the desktop app and `kopuzd`.
#[cfg(not(windows))]
pub fn default_socket_path() -> Option<std::path::PathBuf> {
    let base = directories::BaseDirs::new()?;
    // Moves the default socket without making it remote; a relative one sits under the runtime dir, which a sandbox can name without the uid.
    if let Some(dir) = std::env::var_os("KOPUZ_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        let dir = std::path::PathBuf::from(dir);
        let dir = match dir.is_relative() {
            true => base.runtime_dir()?.join(dir),
            false => dir,
        };
        // The override replaces the flavour's own directory, so the flavour moves into the file name.
        let name = match config::flavor::Flavor::current() {
            config::flavor::Flavor::Stable => "kopuzd.sock",
            config::flavor::Flavor::Nightly => "kopuzd-nightly.sock",
        };
        return Some(dir.join(name));
    }
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join(config::flavor::Flavor::current().name()))
        .unwrap_or_else(|| {
            base.cache_dir()
                .join(config::flavor::Flavor::current().name())
        });
    Some(dir.join("kopuzd.sock"))
}
