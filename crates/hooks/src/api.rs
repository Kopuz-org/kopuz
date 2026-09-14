//! The daemon handle every hook reads through.
//!
//! Provided once by the app; whether it is a `LocalApi` over an in-process
//! core or a `GrpcApi` over a socket is not something a hook can tell.

use std::sync::Arc;

use dioxus::prelude::*;

pub fn use_api() -> Arc<dyn api::KopuzApi> {
    use_context::<Arc<dyn api::KopuzApi>>()
}

/// The same handle from an event handler, where hooks cannot run.
pub fn consume_api() -> Arc<dyn api::KopuzApi> {
    consume_context::<Arc<dyn api::KopuzApi>>()
}
