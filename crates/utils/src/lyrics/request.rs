#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsServerAuth {
    pub url: String,
    pub token: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsRequest {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration: u64,
    pub track_path: String,
    pub server: Option<LyricsServerAuth>,
    pub prefer_local: bool,
    pub enable_musixmatch: bool,
}

impl LyricsRequest {
    pub fn new(
        artist: impl Into<String>,
        title: impl Into<String>,
        album: impl Into<String>,
        duration: u64,
        track_path: impl Into<String>,
    ) -> Self {
        Self {
            artist: artist.into(),
            title: title.into(),
            album: album.into(),
            duration,
            track_path: track_path.into(),
            server: None,
            prefer_local: false,
            enable_musixmatch: false,
        }
    }

    pub fn with_server(
        mut self,
        url: Option<&str>,
        token: Option<&str>,
        user_id: Option<&str>,
    ) -> Self {
        self.server = url.map(|url| LyricsServerAuth {
            url: url.to_string(),
            token: token.map(ToString::to_string),
            user_id: user_id.map(ToString::to_string),
        });
        self
    }

    pub fn prefer_local(mut self, value: bool) -> Self {
        self.prefer_local = value;
        self
    }

    pub fn enable_musixmatch(mut self, value: bool) -> Self {
        self.enable_musixmatch = value;
        self
    }

    pub(crate) fn cache_key(&self) -> String {
        super::lyrics_cache_key(
            &self.artist,
            &self.title,
            &self.album,
            self.duration,
            &self.track_path,
            self.enable_musixmatch,
        )
    }
}
