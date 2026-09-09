//! Sources as a frontend sees them: what is configured, which one is active,
//! and what each can do.
//!
//! A frontend never constructs a media source. It reads these rows to decide
//! which affordances to render -- a delete button, a tag editor, a discover
//! tab -- and calls the mutating methods to change what is configured. No
//! credential ever appears here; the daemon holds them.

/// What kind of thing a source is, for grouping in a settings list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SourceKind {
    /// The legacy single local library.
    Local,
    /// One of the named local libraries.
    LocalLibrary,
    Server,
    #[default]
    Unknown,
}

/// How much of a playlist a source lets a client change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlaylistCapability {
    #[default]
    None,
    AddRemove,
    Reorder,
}

/// Whether artists are a view of the local library or the source's own
/// catalog. Decides, among other things, whether an artist with no photo may
/// borrow one of their album covers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ArtistPresentation {
    #[default]
    Library,
    Remote,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AlbumPresentation {
    #[default]
    Standard,
    /// Albums are catalog entries the library holds only partially, so a
    /// detail view asks the source for the full listing.
    Remote,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FavoritesSyncMode {
    #[default]
    Instant,
    Paginated,
}

/// What the active source supports. Every "should this button exist?" question
/// in a frontend is answered from here, so none of them branches on a service
/// name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceCapabilities {
    pub edit_tags: bool,
    pub delete_from_disk: bool,
    pub scan_folders: bool,
    pub folders: bool,
    pub sync: bool,
    pub downloads: bool,
    pub discover: bool,
    pub track_radio: bool,
    pub playlist_radio: bool,
    pub playlists: PlaylistCapability,
    pub artists: ArtistPresentation,
    pub albums: AlbumPresentation,
    pub favorites_sync: FavoritesSyncMode,
}

/// One configured source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    /// `None` for a local source.
    pub service: Option<config::MusicService>,
    pub active: bool,
    /// Whether the daemon holds usable credentials for it.
    pub authenticated: bool,
    pub capabilities: SourceCapabilities,
    pub url: Option<String>,
    /// Which browser its sign-in flow uses, where it has one.
    pub browser: Option<String>,
    pub anonymous: bool,
    pub storefront: Option<String>,
    pub language: Option<String>,
    /// Scan roots, for a local source.
    pub directories: Vec<String>,
}

/// A local source to create or update. `id` absent means create.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalSourceDraft {
    pub id: Option<String>,
    pub name: String,
    pub directories: Vec<String>,
}

/// A server to create or update. Credentials are provisioned separately, so
/// this carries none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerDraft {
    pub id: Option<String>,
    pub name: String,
    pub url: String,
    pub service: config::MusicService,
    pub browser: Option<String>,
    pub anonymous: bool,
    pub storefront: Option<String>,
    pub language: Option<String>,
}

/// Write-only: a secret a client obtained out of band. No response ever
/// contains these values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialProvision {
    pub server_id: String,
    pub secret: String,
    pub user_id: Option<String>,
    pub browser: Option<String>,
}

/// Write-only username/password sign-in, for servers that take one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceLoginRequest {
    pub server_id: String,
    pub username: String,
    pub password: String,
}

/// One entry when browsing a server's folder tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceFolderEntry {
    pub path: String,
    pub name: String,
}

/// A scrobbling or metadata service, which is configured per-account rather
/// than per-source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IntegrationKind {
    ListenBrainz,
    LastFm,
    LibreFm,
    #[default]
    Unknown,
}

/// Whether an integration is set up. Deliberately not the credentials.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IntegrationStatus {
    pub kind: IntegrationKind,
    pub configured: bool,
}

/// Write-only integration credentials. No response contains these values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntegrationProvision {
    pub kind: IntegrationKind,
    pub token: Option<String>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub session_key: Option<String>,
}
