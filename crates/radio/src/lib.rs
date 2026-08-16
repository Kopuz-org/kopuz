//! Internet radio support for Kopuz: fetches and caches radio station manifests,
//! maps streams to tracks, and manages the station registry.

pub mod browser;
pub mod manifest;
pub mod mapping;
pub mod provider;
pub mod registry;
