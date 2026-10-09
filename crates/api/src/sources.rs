//! Sources as a frontend sees them: what is configured, which one is active,
//! and what each can do.
//!
//! A frontend never constructs a media source, and never names one. It reads
//! these rows to decide which affordances to render -- a delete button, a tag
//! editor, a discover tab -- renders the field lists the daemon publishes to
//! configure them, and calls the mutating methods to change what is
//! configured. No credential ever appears here; the daemon holds them.

use crate::schema::{FieldSpec, FieldValue, Icon, Problem, Text};

/// How much of a playlist a source lets a client change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlaylistCapability {
    #[default]
    None,
    AddRemove,
    Reorder,
}

/// Whether artists are a view of the tracks held or the source's own
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

/// A page of the source's catalog a frontend can link to, such as its home
/// feed, its charts or a library tab. Opened with
/// [`crate::CatalogItemKind::Page`] and `id`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageEntry {
    pub id: String,
    pub label: Text,
    pub icon: Icon,
}

/// A way to narrow a search, such as to songs or to podcasts. `id` is what
/// [`crate::SearchRequest::filter`] takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilter {
    pub id: String,
    pub label: Text,
}

/// What the active source supports. Every "should this button exist?" question
/// in a frontend is answered from here, so none of them branches on a service
/// name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceCapabilities {
    pub edit_tags: bool,
    pub delete_from_disk: bool,
    pub scan_folders: bool,
    /// Playlists live in folders.
    pub folders: bool,
    /// Its library is a directory tree to be picked from, not a catalog.
    pub browse_folders: bool,
    pub sync: bool,
    pub downloads: bool,
    pub discover: bool,
    /// It takes a "stop recommending this" signal for a track.
    pub dont_recommend: bool,
    pub track_radio: bool,
    pub playlist_radio: bool,
    /// Playing a search result starts a track radio from it rather than
    /// queueing the results.
    pub search_radio: bool,
    /// It plays on devices of its own, which a client can list and move to.
    pub external_devices: bool,
    /// It plays through a browser on the host, not the engine.
    pub browser_playback: bool,
    /// It offers several formats per track and picks one by the
    /// `stream_quality` setting.
    pub stream_quality: bool,
    /// Its listings mark explicit tracks, so `skip_explicit` has something to skip.
    pub explicit_flags: bool,
    /// It keeps a watch history of the account's plays, which
    /// `pause_watch_history` stops it being told about.
    pub watch_history: bool,
    pub playlists: PlaylistCapability,
    pub artists: ArtistPresentation,
    pub albums: AlbumPresentation,
    pub favorites_sync: FavoritesSyncMode,
    /// The catalog pages it offers, in the order to list them.
    pub pages: Vec<PageEntry>,
    /// The filters its search takes, in the order to offer them. Empty when
    /// its search is only ever the unfiltered one.
    pub search_filters: Vec<SearchFilter>,
}

/// What making a source usable takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SignInKind {
    /// Nothing: no account is involved, or one is signed in already.
    #[default]
    None,
    /// A username and a password, which the client collects.
    Password,
    /// A browser the daemon drives; the client only asks for it.
    Browser,
}

/// A service the daemon can be pointed at, and the form for adding one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceInfo {
    pub id: String,
    pub name: Text,
    pub icon: Icon,
    /// A colour to tint its rows with.
    pub accent: String,
    pub experimental: bool,
    pub fields: Vec<FieldSpec>,
}

/// Which service a configured source speaks, as much as a client needs: what
/// to call it and how to draw it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceRef {
    pub id: String,
    pub name: Text,
    pub icon: Icon,
    pub accent: String,
}

/// One configured source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub service: ServiceRef,
    pub active: bool,
    /// Whether the daemon holds usable credentials for it.
    pub authenticated: bool,
    /// What signing it in would take, where it is not signed in already.
    pub sign_in: SignInKind,
    /// What signing in *again* would take, whatever it holds now. Stored
    /// credentials go stale, and `sign_in` reads `None` once a source has any,
    /// so it cannot answer for a source that needs re-authenticating.
    pub reauth: SignInKind,
    pub capabilities: SourceCapabilities,
    /// The line under its name: an address, or whatever else identifies it.
    pub detail: Option<String>,
    /// Usable without an account.
    pub anonymous: bool,
    /// Its options, with the values it currently has.
    pub settings: Vec<FieldSpec>,
    /// Reachability is worth watching: the library lives across the network.
    pub needs_network: bool,
    /// Every install has it, so it cannot be deleted.
    pub permanent: bool,
    /// What the daemon's last probe found; `None` until it has probed this source.
    pub state: Option<crate::SourceState>,
    /// The picture of the account it is signed in as, for a service whose accounts have one.
    pub avatar: Option<crate::ArtworkRef>,
}

/// A source to create or update, as the answers to a [`ServiceInfo`]'s form.
/// `secrets` are write-only and never come back.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SourceDraft {
    pub id: Option<String>,
    pub name: String,
    pub service: String,
    pub values: Vec<FieldValue>,
    pub secrets: Vec<FieldValue>,
}

impl std::fmt::Debug for SourceDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceDraft")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("service", &self.service)
            .field("values", &self.values)
            .field(
                "secrets",
                &format_args!("<{} redacted>", self.secrets.len()),
            )
            .finish()
    }
}

/// What a draft would do if it were saved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftCheck {
    pub sign_in: SignInKind,
    pub problems: Vec<Problem>,
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

/// How an integration is connected, once its fields are filled in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConnectKind {
    /// Nothing to press: filling the fields in is the whole of it.
    #[default]
    None,
    /// The daemon opens a sign-in page and keeps what it returns.
    WebSignIn,
}

/// Something configured per account rather than per source: a scrobbler, a
/// metadata service, a presence integration. Same shape as a service, so the
/// same renderer draws it; the credentials are never in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntegrationInfo {
    pub id: String,
    pub name: Text,
    pub icon: Icon,
    pub configured: bool,
    pub connect: ConnectKind,
    pub fields: Vec<FieldSpec>,
}
