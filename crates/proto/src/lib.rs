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

pub mod convert;
pub mod status;
