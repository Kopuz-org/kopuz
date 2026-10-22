//! A signed-in connection to a plugin's service. An instance runs one call at
//! a time and a sync can hold one for minutes, so a session keeps a small pool
//! and the next stream never waits behind it.

use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};
use wasmtime::component::ResourceAny;

use crate::plugin::{Instance, Plugin};
use crate::{PluginError, wit};

const MAX_INSTANCES: usize = 4;

/// Requests time out on their own; this stops a plugin that never returns.
/// A first sync of a large library is one call.
pub(crate) const CALL_LIMIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

struct Live {
    instance: Instance,
    session: ResourceAny,
}

pub struct Session {
    plugin: Arc<Plugin>,
    connection: wit::Connection,
    idle: Mutex<Vec<Live>>,
    slots: Semaphore,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("plugin", &self.plugin.id())
            .field("service", &self.connection.service)
            .finish_non_exhaustive()
    }
}

impl Session {
    pub fn new(plugin: Arc<Plugin>, connection: wit::Connection) -> Self {
        Self {
            plugin,
            connection,
            idle: Mutex::new(Vec::new()),
            slots: Semaphore::new(MAX_INSTANCES),
        }
    }

    pub fn plugin(&self) -> &Plugin {
        &self.plugin
    }

    pub fn service(&self) -> &str {
        &self.connection.service
    }

    async fn open(&self) -> Result<Live, PluginError> {
        let mut instance = self.plugin.instantiate(&self.connection.url).await?;
        let session = instance
            .bindings
            .kopuz_plugin_source()
            .session()
            .call_constructor(&mut instance.store, &self.connection)
            .await
            .map_err(|error| self.plugin.trapped(error))?;
        Ok(Live { instance, session })
    }
}

/// One method per session call. An instance that trapped is dropped, since
/// its memory may be half-written.
macro_rules! calls {
    ($(
        $(#[$meta:meta])*
        fn $name:ident => $call:ident ( $( $arg:ident : $ty:ty => $pass:expr ),* ) -> $ret:ty;
    )*) => {
        impl Session {
            $(
                $(#[$meta])*
                pub async fn $name(&self, $( $arg: $ty ),*) -> Result<$ret, PluginError> {
                    let _slot = self
                        .slots
                        .acquire()
                        .await
                        .map_err(|_| PluginError::Runtime("the session is closed".into()))?;
                    let idle = self.idle.lock().await.pop();
                    let mut live = match idle {
                        Some(live) => live,
                        None => self.open().await?,
                    };
                    let guest = live.instance.bindings.kopuz_plugin_source().session();
                    let call = guest.$call(&mut live.instance.store, live.session, $( $pass ),*);
                    match tokio::time::timeout(CALL_LIMIT, call).await {
                        Ok(Ok(value)) => {
                            self.idle.lock().await.push(live);
                            Ok(value)
                        }
                        Ok(Err(error)) => Err(self.plugin.trapped(error)),
                        Err(_) => {
                            tracing::error!(plugin = %self.plugin.id(), "plugin call timed out");
                            Err(PluginError::TimedOut)
                        }
                    }
                }
            )*
        }
    };
}

type Fallible<T> = Result<T, wit::SourceError>;

calls! {
    fn validate => call_validate() -> wit::AuthOutcome;
    fn diagnose => call_diagnose() -> Fallible<Vec<wit::Check>>;
    fn keepalive => call_keepalive() -> Fallible<()>;
    fn user_id => call_user_id() -> Fallible<Option<String>>;

    fn fetch_library => call_fetch_library() -> Fallible<wit::LibrarySnapshot>;
    fn fetch_artists => call_fetch_artists() -> Fallible<Vec<wit::ArtistListing>>;
    fn fetch_album_tracks => call_fetch_album_tracks(album_id: &str => album_id) -> Fallible<Vec<wit::Track>>;
    fn fetch_track => call_fetch_track(item_id: &str => item_id) -> Fallible<Option<wit::Track>>;
    fn search => call_search(query: &str => query) -> Fallible<wit::SearchResults>;

    fn fetch_playlists => call_fetch_playlists() -> Fallible<Vec<wit::PlaylistMeta>>;
    fn fetch_playlist_entries => call_fetch_playlist_entries(playlist_id: &str => playlist_id) -> Fallible<Vec<wit::Track>>;
    fn fetch_playlist_page => call_fetch_playlist_page(
        playlist_id: &str => playlist_id,
        cursor: Option<&str> => cursor
    ) -> Fallible<wit::TrackPage>;
    fn add_to_playlist => call_add_to_playlist(
        playlist_id: &str => playlist_id,
        item_refs: &[String] => item_refs
    ) -> Fallible<Vec<String>>;
    fn create_playlist => call_create_playlist(
        name: &str => name,
        item_refs: &[String] => item_refs
    ) -> Fallible<String>;
    fn remove_from_playlist => call_remove_from_playlist(
        playlist_id: &str => playlist_id,
        entry: &wit::PlaylistEntry => entry,
        position: u32 => position
    ) -> Fallible<()>;
    fn reorder_playlist => call_reorder_playlist(
        playlist_id: &str => playlist_id,
        ordered: &[wit::PlaylistEntry] => ordered,
        moved: &wit::PlaylistEntry => moved,
        new_index: u32 => new_index
    ) -> Fallible<()>;
    fn set_playlist_image => call_set_playlist_image(
        playlist_id: &str => playlist_id,
        image: &[u8] => image,
        content_type: &str => content_type
    ) -> Fallible<()>;
    fn delete_remote_playlist => call_delete_remote_playlist(playlist_id: &str => playlist_id) -> Fallible<()>;

    fn fetch_favorites => call_fetch_favorites() -> Fallible<Vec<String>>;
    fn fetch_favorites_page => call_fetch_favorites_page(cursor: Option<&str> => cursor) -> Fallible<wit::TrackPage>;
    fn push_favorite => call_push_favorite(item_id: &str => item_id, on: bool => on) -> Fallible<()>;

    fn resolve_stream => call_resolve_stream(
        item_id: &str => item_id,
        quality: wit::StreamQuality => quality
    ) -> Fallible<wit::StreamInfo>;
    fn download_track => call_download_track(item_id: &str => item_id) -> Fallible<Vec<u8>>;
    fn upload_track => call_upload_track(filename: &str => filename, content: &[u8] => content) -> Fallible<()>;
    fn storage_quota => call_storage_quota() -> Fallible<wit::StorageQuota>;

    fn start_radio => call_start_radio(seed_ref: &str => seed_ref) -> Fallible<wit::TrackPage>;
    fn start_playlist_radio => call_start_playlist_radio(playlist_ref: &str => playlist_ref) -> Fallible<wit::TrackPage>;
    fn more_radio => call_more_radio(cursor: &str => cursor) -> Fallible<wit::TrackPage>;

    fn save_play_queue => call_save_play_queue(
        item_ids: &[String] => item_ids,
        current_index: u32 => current_index,
        position_ms: u64 => position_ms
    ) -> Fallible<bool>;
    fn get_play_queue => call_get_play_queue() -> Fallible<Option<wit::RemotePlayQueue>>;

    fn scrobble_now_playing => call_scrobble_now_playing(item_id: &str => item_id) -> Fallible<()>;
    fn scrobble => call_scrobble(item_id: &str => item_id) -> Fallible<()>;
    fn report_playback_start => call_report_playback_start(item_id: &str => item_id) -> Fallible<()>;
    fn report_playback_stopped => call_report_playback_stopped(
        item_id: &str => item_id,
        position_ticks: u64 => position_ticks
    ) -> Fallible<()>;
    fn report_playback_progress => call_report_playback_progress(
        item_id: &str => item_id,
        position_ticks: u64 => position_ticks,
        is_paused: bool => is_paused
    ) -> Fallible<()>;
}
