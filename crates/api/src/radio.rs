//! Internet radio: the stations a client can browse, pin and play.
//!
//! Stream URLs never appear here. A client plays a station by naming it and
//! one of its streams, and the daemon resolves the rest -- which is what
//! keeps a registry's URL rewriting, redirects and playlist unwrapping out of
//! every frontend.

/// One way to listen to a station, usually a bitrate or a codec.
///
/// `name` may be a translation key rather than display text, because station
/// metadata is authored in the registry and localized by the client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RadioStreamInfo {
    pub id: String,
    pub name: String,
    /// An icon name the registry authored, for a client that draws icons.
    pub icon: Option<String>,
}

/// A station as a browser renders it. `name` and `description` may be
/// translation keys, for the same reason as above.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RadioStationInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub streams: Vec<RadioStreamInfo>,
    /// Whether the station shows in the client's own selected list, which is
    /// what [`crate::LibraryApi::pin_radio_station`] toggles.
    pub pinned: bool,
    pub artwork: Option<crate::ArtworkRef>,
    /// An icon name the registry authored, for a client that draws icons.
    pub icon: String,
}
