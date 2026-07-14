use serde::{Deserialize, Serialize};

/// Where a track/playlist/favorite comes from, and what the app is currently
/// sourcing from: the local library, or a specific media server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum Source {
    #[default]
    Local,
    Server(String),
}

impl Source {
    /// The `source` column value: `"local"` or the server id.
    pub fn as_str(&self) -> &str {
        match self {
            Source::Local => "local",
            Source::Server(id) => id.as_str(),
        }
    }

    /// Build from a stored `source` column value.
    pub fn from_column(s: &str) -> Self {
        if s == "local" {
            Source::Local
        } else {
            Source::Server(s.to_owned())
        }
    }

    /// The server id, if this is a server source.
    pub fn server_id(&self) -> Option<&str> {
        match self {
            Source::Server(id) => Some(id),
            Source::Local => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum MusicService {
    #[default]
    Jellyfin,
    #[serde(alias = "Navidrome")]
    Subsonic,
    Custom,
    YtMusic,
    SoundCloud,
    Tidal,
}

impl MusicService {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Jellyfin => "Jellyfin",
            Self::Subsonic => "Subsonic",
            Self::Custom => "Custom",
            Self::YtMusic => "YouTube Music",
            Self::SoundCloud => "SoundCloud",
            Self::Tidal => "TIDAL",
        }
    }

    /// Backends that authenticate via a browser sign-in window (OAuth/cookies)
    /// rather than a URL + username/password form.
    pub fn uses_browser_signin(&self) -> bool {
        matches!(self, Self::YtMusic | Self::SoundCloud)
    }

    /// Backends that sign in via the OAuth device flow — kopuz opens the
    /// default browser on an approval page and polls for the grant. No URL,
    /// no password form, no isolated browser profile.
    pub fn uses_device_signin(&self) -> bool {
        matches!(self, Self::Tidal)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MusicServer {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub service: MusicService,
    pub access_token: Option<String>,
    pub user_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    /// For browser sign-in services: which Chromium-family browser was used.
    #[serde(default)]
    pub yt_browser: Option<Browser>,
    /// For `MusicService::YtMusic` only: anonymous mode.
    #[serde(default)]
    pub yt_anonymous: bool,
}

impl MusicServer {
    pub fn new(name: String, url: String) -> Self {
        Self::new_with_service(name, url, MusicService::Jellyfin)
    }

    pub fn new_with_service(name: String, url: String, service: MusicService) -> Self {
        Self {
            name,
            url: url.trim_end_matches('/').to_string(),
            service,
            access_token: None,
            user_id: None,
            id: Some(uuid::Uuid::new_v4().to_string()),
            yt_browser: None,
            yt_anonymous: false,
        }
    }

    pub fn yt_browser(&self) -> Option<Browser> {
        self.yt_browser
    }
}

impl Default for MusicServer {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            service: MusicService::Jellyfin,
            access_token: None,
            user_id: None,
            id: None,
            yt_browser: None,
            yt_anonymous: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Browser {
    Chrome,
    Chromium,
    Brave,
    Edge,
    Vivaldi,
}

impl Browser {
    pub const ALL: &'static [Browser] = &[
        Browser::Chrome,
        Browser::Chromium,
        Browser::Brave,
        Browser::Edge,
        Browser::Vivaldi,
    ];

    /// The stable id used in URL routes, settings UI option values,
    /// libsecret lookups, etc.
    pub fn id(self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::Chromium => "chromium",
            Browser::Brave => "brave",
            Browser::Edge => "edge",
            Browser::Vivaldi => "vivaldi",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Browser::Chrome => "Chrome",
            Browser::Chromium => "Chromium",
            Browser::Brave => "Brave",
            Browser::Edge => "Edge",
            Browser::Vivaldi => "Vivaldi",
        }
    }

    pub fn from_id(s: &str) -> Option<Browser> {
        Browser::ALL
            .iter()
            .copied()
            .find(|browser| browser.id() == s)
    }
}

impl std::fmt::Display for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

pub type JellyfinServer = MusicServer;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedServer {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub service: MusicService,
    /// Persisted browser choice for browser sign-in servers.
    #[serde(default)]
    pub yt_browser: Option<Browser>,
    /// Persisted anonymous-mode flag.
    #[serde(default)]
    pub yt_anonymous: bool,
}

impl SavedServer {
    pub fn new(name: String, url: String, service: MusicService) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            url: url.trim_end_matches('/').to_string(),
            service,
            yt_browser: None,
            yt_anonymous: false,
        }
    }

    pub fn from_music_server(server: &MusicServer) -> Self {
        Self {
            id: server
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: server.name.clone(),
            url: server.url.clone(),
            service: server.service,
            yt_browser: server.yt_browser,
            yt_anonymous: server.yt_anonymous,
        }
    }

    pub fn matches(&self, server: &MusicServer) -> bool {
        if let Some(sid) = server.id.as_ref()
            && sid == &self.id
        {
            return true;
        }
        self.url == server.url && self.service == server.service
    }
}
