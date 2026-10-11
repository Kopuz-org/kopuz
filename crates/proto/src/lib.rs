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
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join("kopuz"))
        .unwrap_or_else(|| base.cache_dir().join("kopuz"));
    Some(dir.join("kopuzd.sock"))
}
