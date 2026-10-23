//! Build a kopuz plugin: implement [`Source`] and export it with
//! [`export_source!`], then build for `wasm32-wasip2`. The crate builds
//! natively too, so a plugin's own logic can be unit tested.

pub mod cover;
pub mod http;
pub mod log;

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "source-plugin",
        pub_export_macro: true,
        export_macro_name: "__export_source_plugin",
        default_bindings_module: "kopuz_plugin_sdk::bindings",
        additional_derives: [PartialEq],
    });
}

pub use bindings::kopuz::plugin::types::{
    Album, ArtistCredit, ArtistListing, AuthOutcome, AuthSession, Check, CheckOutcome, CheckStep,
    Connection, LibrarySnapshot, PlaylistEntry, PlaylistMeta, RemotePlayQueue, ReplayGain,
    SearchResults, SourceError, StorageQuota, StreamInfo, StreamQuality, Track, TrackPage,
};

pub type Result<T, E = SourceError> = std::result::Result<T, E>;

impl SourceError {
    /// Plain text for a person; the generated `Display` is the debug form.
    pub fn message(&self) -> String {
        match self {
            SourceError::Unsupported(operation) => {
                format!("this source doesn't support {operation}")
            }
            SourceError::Connectivity => "the server did not answer".to_string(),
            SourceError::Auth => "the server refused the credentials".to_string(),
            SourceError::InvalidInput(message) | SourceError::Backend(message) => message.clone(),
        }
    }

    pub fn unsupported(operation: &str) -> Self {
        SourceError::Unsupported(operation.to_string())
    }

    pub fn from_status(status: u16, what: &str) -> Self {
        match status {
            401 | 403 => SourceError::Auth,
            _ => SourceError::Backend(format!("{what} failed: HTTP {status}")),
        }
    }
}

impl From<http::Error> for SourceError {
    fn from(error: http::Error) -> Self {
        if error.is_connectivity() {
            SourceError::Connectivity
        } else {
            SourceError::Backend(error.to_string())
        }
    }
}

impl From<String> for SourceError {
    fn from(message: String) -> Self {
        SourceError::Backend(message)
    }
}

impl StreamInfo {
    pub fn url(url: impl Into<String>) -> Self {
        StreamInfo {
            url: url.into(),
            hls: false,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
            content_length: None,
            loudness_db: None,
        }
    }
}

impl ArtistCredit {
    pub fn unlinked(name: impl Into<String>) -> Self {
        ArtistCredit {
            name: name.into(),
            id: None,
        }
    }

    pub fn linked(name: impl Into<String>, id: impl Into<String>) -> Self {
        ArtistCredit {
            name: name.into(),
            id: Some(id.into()),
        }
    }
}

// Generated from the WIT, where no derive can be added.
#[allow(clippy::derivable_impls)]
impl Default for ReplayGain {
    fn default() -> Self {
        ReplayGain {
            track_gain_db: None,
            track_peak: None,
            album_gain_db: None,
            album_peak: None,
        }
    }
}

#[allow(clippy::derivable_impls)]
impl Default for Track {
    fn default() -> Self {
        Track {
            item_id: String::new(),
            cover: None,
            album_id: String::new(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_secs: 0,
            sample_rate: 0,
            bitrate_kbps: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: Vec::new(),
            credits: Vec::new(),
            replay_gain: ReplayGain::default(),
            explicit: false,
        }
    }
}

impl TrackPage {
    pub fn last(tracks: Vec<Track>) -> Self {
        TrackPage { tracks, next: None }
    }
}

impl Check {
    pub fn of<T>(
        step: CheckStep,
        result: std::result::Result<T, String>,
        detail: impl FnOnce(T) -> String,
    ) -> Self {
        match result {
            Ok(value) => Check {
                step,
                outcome: CheckOutcome::Passed,
                detail: detail(value),
            },
            Err(error) => Check {
                step,
                outcome: CheckOutcome::Failed,
                detail: error,
            },
        }
    }
}

/// A media source. Every operation but opening, validating and resolving a
/// stream has the host's own default; the host never asks for one whose
/// capability the manifest leaves off.
pub trait Source: Sized + 'static {
    fn login(connection: Connection, username: &str, password: &str) -> Result<AuthSession> {
        let _ = (connection, username, password);
        Err(SourceError::unsupported("username and password sign-in"))
    }

    fn open(connection: Connection) -> Self;

    fn validate(&self) -> AuthOutcome;

    fn diagnose(&self) -> Result<Vec<Check>> {
        let result = match self.validate() {
            AuthOutcome::Valid => Ok(()),
            AuthOutcome::Expired => Err("the sign-in is no longer accepted".to_string()),
            AuthOutcome::Unreachable => Err("the server did not answer".to_string()),
        };
        Ok(vec![Check::of(CheckStep::Session, result, |()| {
            "accepted".to_string()
        })])
    }

    fn keepalive(&self) -> Result<()> {
        Ok(())
    }

    fn user_id(&self) -> Result<Option<String>> {
        Ok(None)
    }

    fn fetch_library(&self) -> Result<LibrarySnapshot> {
        Ok(LibrarySnapshot {
            albums: Vec::new(),
            tracks: Vec::new(),
            artists: Vec::new(),
        })
    }

    fn fetch_artists(&self) -> Result<Vec<ArtistListing>> {
        Ok(Vec::new())
    }

    fn fetch_album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        let _ = album_id;
        Err(SourceError::unsupported("album tracks"))
    }

    fn fetch_track(&self, item_id: &str) -> Result<Option<Track>> {
        let _ = item_id;
        Ok(None)
    }

    /// Unsupported leaves search to the host's synced library.
    fn search(&self, query: &str) -> Result<SearchResults> {
        let _ = query;
        Err(SourceError::unsupported("search"))
    }

    fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>> {
        Ok(Vec::new())
    }

    fn fetch_playlist_entries(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let _ = playlist_id;
        Ok(Vec::new())
    }

    fn fetch_playlist_page(&self, playlist_id: &str, cursor: Option<&str>) -> Result<TrackPage> {
        let _ = cursor;
        Ok(TrackPage::last(self.fetch_playlist_entries(playlist_id)?))
    }

    fn add_to_playlist(&self, playlist_id: &str, item_refs: &[String]) -> Result<Vec<String>> {
        let _ = (playlist_id, item_refs);
        Err(SourceError::unsupported("adding to playlists"))
    }

    fn create_playlist(&self, name: &str, item_refs: &[String]) -> Result<String> {
        let _ = (name, item_refs);
        Err(SourceError::unsupported("creating playlists"))
    }

    fn remove_from_playlist(
        &self,
        playlist_id: &str,
        entry: &PlaylistEntry,
        position: u32,
    ) -> Result<()> {
        let _ = (playlist_id, entry, position);
        Err(SourceError::unsupported("removing from playlists"))
    }

    fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered: &[PlaylistEntry],
        moved: &PlaylistEntry,
        new_index: u32,
    ) -> Result<()> {
        let _ = (playlist_id, ordered, moved, new_index);
        Err(SourceError::unsupported("playlist reorder"))
    }

    fn set_playlist_image(
        &self,
        playlist_id: &str,
        image: &[u8],
        content_type: &str,
    ) -> Result<()> {
        let _ = (playlist_id, image, content_type);
        Err(SourceError::unsupported("playlist images"))
    }

    fn delete_remote_playlist(&self, playlist_id: &str) -> Result<()> {
        let _ = playlist_id;
        Ok(())
    }

    /// Unsupported, with `push_favorite`, keeps favorites in kopuz alone.
    fn fetch_favorites(&self) -> Result<Vec<String>> {
        Err(SourceError::unsupported("favorites"))
    }

    fn fetch_favorites_page(&self, cursor: Option<&str>) -> Result<TrackPage> {
        let _ = cursor;
        Ok(TrackPage::last(Vec::new()))
    }

    fn push_favorite(&self, item_id: &str, on: bool) -> Result<()> {
        let _ = (item_id, on);
        Err(SourceError::unsupported("favorites"))
    }

    fn resolve_stream(&self, item_id: &str, quality: StreamQuality) -> Result<StreamInfo>;

    fn download_track(&self, item_id: &str) -> Result<Vec<u8>> {
        let _ = item_id;
        Err(SourceError::unsupported("track download"))
    }

    fn upload_track(&self, filename: &str, content: &[u8]) -> Result<()> {
        let _ = (filename, content);
        Err(SourceError::unsupported("music uploads"))
    }

    fn storage_quota(&self) -> Result<StorageQuota> {
        Err(SourceError::unsupported("storage quota"))
    }

    fn start_radio(&self, seed_ref: &str) -> Result<TrackPage> {
        let _ = seed_ref;
        Err(SourceError::unsupported("radio"))
    }

    fn start_playlist_radio(&self, playlist_ref: &str) -> Result<TrackPage> {
        let _ = playlist_ref;
        Err(SourceError::unsupported("playlist radio"))
    }

    fn more_radio(&self, cursor: &str) -> Result<TrackPage> {
        let _ = cursor;
        Err(SourceError::unsupported("radio continuation"))
    }

    fn save_play_queue(
        &self,
        item_ids: &[String],
        current_index: u32,
        position_ms: u64,
    ) -> Result<bool> {
        let _ = (item_ids, current_index, position_ms);
        Err(SourceError::unsupported("play queue sync"))
    }

    fn get_play_queue(&self) -> Result<Option<RemotePlayQueue>> {
        Err(SourceError::unsupported("play queue sync"))
    }

    fn scrobble_now_playing(&self, item_id: &str) -> Result<()> {
        let _ = item_id;
        Ok(())
    }

    fn scrobble(&self, item_id: &str) -> Result<()> {
        let _ = item_id;
        Ok(())
    }

    fn report_playback_start(&self, item_id: &str) -> Result<()> {
        let _ = item_id;
        Ok(())
    }

    fn report_playback_stopped(&self, item_id: &str, position_ticks: u64) -> Result<()> {
        let _ = (item_id, position_ticks);
        Ok(())
    }

    fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: u64,
        is_paused: bool,
    ) -> Result<()> {
        let _ = (item_id, position_ticks, is_paused);
        Ok(())
    }
}

/// Export a [`Source`] implementation, once, at the plugin crate's root.
#[macro_export]
macro_rules! export_source {
    ($source:ty) => {
        #[doc(hidden)]
        pub struct __KopuzPlugin;

        #[doc(hidden)]
        pub struct __KopuzSession($source);

        impl $crate::bindings::exports::kopuz::plugin::source::Guest for __KopuzPlugin {
            type Session = __KopuzSession;

            fn login(
                connection: $crate::Connection,
                username: ::std::string::String,
                password: ::std::string::String,
            ) -> $crate::Result<$crate::AuthSession> {
                <$source as $crate::Source>::login(connection, &username, &password)
            }
        }

        impl $crate::bindings::exports::kopuz::plugin::source::GuestSession for __KopuzSession {
            fn new(connection: $crate::Connection) -> Self {
                __KopuzSession(<$source as $crate::Source>::open(connection))
            }
            fn validate(&self) -> $crate::AuthOutcome {
                $crate::Source::validate(&self.0)
            }
            fn diagnose(&self) -> $crate::Result<::std::vec::Vec<$crate::Check>> {
                $crate::Source::diagnose(&self.0)
            }
            fn keepalive(&self) -> $crate::Result<()> {
                $crate::Source::keepalive(&self.0)
            }
            fn user_id(&self) -> $crate::Result<::std::option::Option<::std::string::String>> {
                $crate::Source::user_id(&self.0)
            }
            fn fetch_library(&self) -> $crate::Result<$crate::LibrarySnapshot> {
                $crate::Source::fetch_library(&self.0)
            }
            fn fetch_artists(&self) -> $crate::Result<::std::vec::Vec<$crate::ArtistListing>> {
                $crate::Source::fetch_artists(&self.0)
            }
            fn fetch_album_tracks(
                &self,
                album_id: ::std::string::String,
            ) -> $crate::Result<::std::vec::Vec<$crate::Track>> {
                $crate::Source::fetch_album_tracks(&self.0, &album_id)
            }
            fn fetch_track(
                &self,
                item_id: ::std::string::String,
            ) -> $crate::Result<::std::option::Option<$crate::Track>> {
                $crate::Source::fetch_track(&self.0, &item_id)
            }
            fn search(&self, query: ::std::string::String) -> $crate::Result<$crate::SearchResults> {
                $crate::Source::search(&self.0, &query)
            }
            fn fetch_playlists(&self) -> $crate::Result<::std::vec::Vec<$crate::PlaylistMeta>> {
                $crate::Source::fetch_playlists(&self.0)
            }
            fn fetch_playlist_entries(
                &self,
                playlist_id: ::std::string::String,
            ) -> $crate::Result<::std::vec::Vec<$crate::Track>> {
                $crate::Source::fetch_playlist_entries(&self.0, &playlist_id)
            }
            fn fetch_playlist_page(
                &self,
                playlist_id: ::std::string::String,
                cursor: ::std::option::Option<::std::string::String>,
            ) -> $crate::Result<$crate::TrackPage> {
                $crate::Source::fetch_playlist_page(&self.0, &playlist_id, cursor.as_deref())
            }
            fn add_to_playlist(
                &self,
                playlist_id: ::std::string::String,
                item_refs: ::std::vec::Vec<::std::string::String>,
            ) -> $crate::Result<::std::vec::Vec<::std::string::String>> {
                $crate::Source::add_to_playlist(&self.0, &playlist_id, &item_refs)
            }
            fn create_playlist(
                &self,
                name: ::std::string::String,
                item_refs: ::std::vec::Vec<::std::string::String>,
            ) -> $crate::Result<::std::string::String> {
                $crate::Source::create_playlist(&self.0, &name, &item_refs)
            }
            fn remove_from_playlist(
                &self,
                playlist_id: ::std::string::String,
                entry: $crate::PlaylistEntry,
                position: u32,
            ) -> $crate::Result<()> {
                $crate::Source::remove_from_playlist(&self.0, &playlist_id, &entry, position)
            }
            fn reorder_playlist(
                &self,
                playlist_id: ::std::string::String,
                ordered: ::std::vec::Vec<$crate::PlaylistEntry>,
                moved: $crate::PlaylistEntry,
                new_index: u32,
            ) -> $crate::Result<()> {
                $crate::Source::reorder_playlist(&self.0, &playlist_id, &ordered, &moved, new_index)
            }
            fn set_playlist_image(
                &self,
                playlist_id: ::std::string::String,
                image: ::std::vec::Vec<u8>,
                content_type: ::std::string::String,
            ) -> $crate::Result<()> {
                $crate::Source::set_playlist_image(&self.0, &playlist_id, &image, &content_type)
            }
            fn delete_remote_playlist(&self, playlist_id: ::std::string::String) -> $crate::Result<()> {
                $crate::Source::delete_remote_playlist(&self.0, &playlist_id)
            }
            fn fetch_favorites(&self) -> $crate::Result<::std::vec::Vec<::std::string::String>> {
                $crate::Source::fetch_favorites(&self.0)
            }
            fn fetch_favorites_page(
                &self,
                cursor: ::std::option::Option<::std::string::String>,
            ) -> $crate::Result<$crate::TrackPage> {
                $crate::Source::fetch_favorites_page(&self.0, cursor.as_deref())
            }
            fn push_favorite(&self, item_id: ::std::string::String, on: bool) -> $crate::Result<()> {
                $crate::Source::push_favorite(&self.0, &item_id, on)
            }
            fn resolve_stream(
                &self,
                item_id: ::std::string::String,
                quality: $crate::StreamQuality,
            ) -> $crate::Result<$crate::StreamInfo> {
                $crate::Source::resolve_stream(&self.0, &item_id, quality)
            }
            fn download_track(&self, item_id: ::std::string::String) -> $crate::Result<::std::vec::Vec<u8>> {
                $crate::Source::download_track(&self.0, &item_id)
            }
            fn upload_track(
                &self,
                filename: ::std::string::String,
                content: ::std::vec::Vec<u8>,
            ) -> $crate::Result<()> {
                $crate::Source::upload_track(&self.0, &filename, &content)
            }
            fn storage_quota(&self) -> $crate::Result<$crate::StorageQuota> {
                $crate::Source::storage_quota(&self.0)
            }
            fn start_radio(&self, seed_ref: ::std::string::String) -> $crate::Result<$crate::TrackPage> {
                $crate::Source::start_radio(&self.0, &seed_ref)
            }
            fn start_playlist_radio(
                &self,
                playlist_ref: ::std::string::String,
            ) -> $crate::Result<$crate::TrackPage> {
                $crate::Source::start_playlist_radio(&self.0, &playlist_ref)
            }
            fn more_radio(&self, cursor: ::std::string::String) -> $crate::Result<$crate::TrackPage> {
                $crate::Source::more_radio(&self.0, &cursor)
            }
            fn save_play_queue(
                &self,
                item_ids: ::std::vec::Vec<::std::string::String>,
                current_index: u32,
                position_ms: u64,
            ) -> $crate::Result<bool> {
                $crate::Source::save_play_queue(&self.0, &item_ids, current_index, position_ms)
            }
            fn get_play_queue(&self) -> $crate::Result<::std::option::Option<$crate::RemotePlayQueue>> {
                $crate::Source::get_play_queue(&self.0)
            }
            fn scrobble_now_playing(&self, item_id: ::std::string::String) -> $crate::Result<()> {
                $crate::Source::scrobble_now_playing(&self.0, &item_id)
            }
            fn scrobble(&self, item_id: ::std::string::String) -> $crate::Result<()> {
                $crate::Source::scrobble(&self.0, &item_id)
            }
            fn report_playback_start(&self, item_id: ::std::string::String) -> $crate::Result<()> {
                $crate::Source::report_playback_start(&self.0, &item_id)
            }
            fn report_playback_stopped(
                &self,
                item_id: ::std::string::String,
                position_ticks: u64,
            ) -> $crate::Result<()> {
                $crate::Source::report_playback_stopped(&self.0, &item_id, position_ticks)
            }
            fn report_playback_progress(
                &self,
                item_id: ::std::string::String,
                position_ticks: u64,
                is_paused: bool,
            ) -> $crate::Result<()> {
                $crate::Source::report_playback_progress(&self.0, &item_id, position_ticks, is_paused)
            }
        }

        $crate::bindings::__export_source_plugin!(__KopuzPlugin with_types_in $crate::bindings);
    };
}
