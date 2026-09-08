/// A playlist as the wire sees it: identity, name, cover, and the track refs
/// in order. The refs are library keys, so a client resolves them through
/// [`crate::LibraryApi::tracks_by_keys`] like any other list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistInfo {
    pub id: String,
    pub name: String,
    pub track_keys: Vec<String>,
    pub artwork: Option<crate::ArtworkTarget>,
}

/// A user-made grouping of playlists. Local organisation only -- no source
/// has a concept for it -- so the ids here are always local playlist ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistFolderInfo {
    pub id: String,
    pub name: String,
    pub playlist_ids: Vec<String>,
}

/// Everything the playlist views render from, in one read: a page that showed
/// playlists and folders separately would tear between the two.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistCatalog {
    pub playlists: Vec<PlaylistInfo>,
    pub folders: Vec<PlaylistFolderInfo>,
}

/// Where a reorder puts the moved track. `from` is where it is now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlaylistReorder {
    pub from: u32,
    pub to: u32,
}
