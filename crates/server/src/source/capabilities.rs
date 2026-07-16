use async_trait::async_trait;
use config::Source;

use super::{
    AuthOutcome, Capabilities, FavoritesPage, LibrarySnapshot, MediaSource, PlaylistMeta,
    PlaylistPage, RemoteAlbum, SourceError, StreamInfo,
};

pub trait SourceIdentity {
    fn source(&self) -> &Source;
    fn capabilities(&self) -> Capabilities;
}

impl<T> SourceIdentity for T
where
    T: MediaSource + ?Sized,
{
    fn source(&self) -> &Source {
        <T as MediaSource>::source(self)
    }

    fn capabilities(&self) -> Capabilities {
        <T as MediaSource>::capabilities(self)
    }
}

#[async_trait]
pub trait PlayableSource: SourceIdentity + Send + Sync {
    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError>;
    async fn validate(&self) -> AuthOutcome;
    async fn web_url(&self, track: &reader::Track) -> Option<String>;
}

#[async_trait]
impl<T> PlayableSource for T
where
    T: MediaSource + ?Sized,
{
    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        <T as MediaSource>::resolve_stream(self, item_id).await
    }

    async fn validate(&self) -> AuthOutcome {
        <T as MediaSource>::validate(self).await
    }

    async fn web_url(&self, track: &reader::Track) -> Option<String> {
        <T as MediaSource>::web_url(self, track).await
    }
}

#[async_trait]
pub trait PlaylistSource: SourceIdentity + Send + Sync {
    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError>;

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError>;

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        position: usize,
    ) -> Result<(), SourceError>;

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        moved: &reader::Track,
        new_index: usize,
    ) -> Result<(), SourceError>;

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError>;

    async fn fetch_playlist_entries_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<PlaylistPage, SourceError>;

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError>;
}

#[async_trait]
impl<T> PlaylistSource for T
where
    T: MediaSource + ?Sized,
{
    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        <T as MediaSource>::add_to_playlist(self, playlist_id, item_refs).await
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        <T as MediaSource>::create_playlist(self, name, item_refs).await
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        position: usize,
    ) -> Result<(), SourceError> {
        <T as MediaSource>::remove_from_playlist(self, playlist_id, track, position).await
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        moved: &reader::Track,
        new_index: usize,
    ) -> Result<(), SourceError> {
        <T as MediaSource>::reorder_playlist(self, playlist_id, ordered_refs, moved, new_index)
            .await
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        <T as MediaSource>::fetch_playlist_entries(self, playlist_id).await
    }

    async fn fetch_playlist_entries_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<PlaylistPage, SourceError> {
        <T as MediaSource>::fetch_playlist_entries_page(self, playlist_id, cursor).await
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        <T as MediaSource>::fetch_playlists(self).await
    }
}

#[async_trait]
pub trait CatalogSource: SourceIdentity + Send + Sync {
    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError>;

    async fn discover_home(&self) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError>;

    async fn discover_continuation(
        &self,
        token: &str,
    ) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError>;

    async fn fetch_album(&self, browse_id: &str) -> Result<RemoteAlbum, SourceError>;

    async fn fetch_album_by_ref(&self, id: &str) -> Result<Option<RemoteAlbum>, SourceError>;

    async fn fetch_artist(
        &self,
        channel_id: &str,
    ) -> Result<crate::ytmusic::discover::YtArtist, SourceError>;
}

#[async_trait]
impl<T> CatalogSource for T
where
    T: MediaSource + ?Sized,
{
    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        <T as MediaSource>::search(self, query).await
    }

    async fn discover_home(&self) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        <T as MediaSource>::discover_home(self).await
    }

    async fn discover_continuation(
        &self,
        token: &str,
    ) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        <T as MediaSource>::discover_continuation(self, token).await
    }

    async fn fetch_album(&self, browse_id: &str) -> Result<RemoteAlbum, SourceError> {
        <T as MediaSource>::fetch_album(self, browse_id).await
    }

    async fn fetch_album_by_ref(&self, id: &str) -> Result<Option<RemoteAlbum>, SourceError> {
        <T as MediaSource>::fetch_album_by_ref(self, id).await
    }

    async fn fetch_artist(
        &self,
        channel_id: &str,
    ) -> Result<crate::ytmusic::discover::YtArtist, SourceError> {
        <T as MediaSource>::fetch_artist(self, channel_id).await
    }
}

#[async_trait]
pub trait LibrarySource: SourceIdentity + Send + Sync {
    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError>;
    async fn album_tracks(&self, album_id: &str) -> Result<Vec<reader::Track>, SourceError>;
    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError>;
    async fn fetch_artist_image(&self, name: &str) -> Result<Option<String>, SourceError>;
}

#[async_trait]
impl<T> LibrarySource for T
where
    T: MediaSource + ?Sized,
{
    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        <T as MediaSource>::fetch_library(self).await
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<reader::Track>, SourceError> {
        <T as MediaSource>::album_tracks(self, album_id).await
    }

    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError> {
        <T as MediaSource>::fetch_artist_images(self).await
    }

    async fn fetch_artist_image(&self, name: &str) -> Result<Option<String>, SourceError> {
        <T as MediaSource>::fetch_artist_image(self, name).await
    }
}

#[async_trait]
pub trait FavoritesSource: SourceIdentity + Send + Sync {
    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError>;
    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError>;
    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError>;
    async fn favorites(&self) -> Result<Vec<String>, SourceError>;
    async fn is_favorite(&self, ref_: &str) -> bool;
}

#[async_trait]
impl<T> FavoritesSource for T
where
    T: MediaSource + ?Sized,
{
    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        <T as MediaSource>::fetch_favorites(self).await
    }

    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        <T as MediaSource>::fetch_favorites_page(self, cursor).await
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        <T as MediaSource>::push_favorite(self, item_id, on).await
    }

    async fn favorites(&self) -> Result<Vec<String>, SourceError> {
        <T as MediaSource>::favorites(self).await
    }

    async fn is_favorite(&self, ref_: &str) -> bool {
        <T as MediaSource>::is_favorite(self, ref_).await
    }
}
