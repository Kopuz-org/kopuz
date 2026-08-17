//! The [`MediaSource`] a plugin backs.
//!
//! This is the whole adapter layer: it owns the id namespacing in both
//! directions and the mapping between the plugin's tables and the app's model
//! types, and nothing else. Every uniform database operation is inherited from
//! the trait's defaults by supplying [`source`](MediaSource::source) and
//! [`db`](MediaSource::db) locally, so no database call is ever handed to Lua.
//!
//! Two rules the rest of the file follows:
//!
//! * Ids the app persists (tracks, albums) are namespaced `"<plugin_id>/<ref>"`
//!   so two plugins can never share a `TrackId::Server` uid. Ids that only ever
//!   round-trip in memory (artists, playlists, shelf tokens) stay opaque.
//! * An operation the plugin does not implement degrades to the same default a
//!   built-in source would have had, rather than surfacing as an error. That is
//!   what lets a twenty-line plugin be useful.

use std::sync::Arc;

use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;

use crate::plugin::{self, PluginInstance, export};
use crate::ytmusic::discover::{DiscoverHome, DiscoverItem, DiscoverShelf, YtArtist};

use super::{
    AuthOutcome, Capabilities, FavoritesPage, FavoritesSync, LibrarySnapshot, MediaSource,
    PlaylistMeta, PlaylistPage, RemoteAlbum, SourceError, StreamInfo,
};

pub(super) struct PluginSource {
    db: Db,
    source: Source,
    plugin_id: String,
    map: Mapper,
}

impl PluginSource {
    pub(super) fn new(db: Db, source: Source, plugin_id: String) -> Self {
        Self {
            db,
            source,
            map: Mapper {
                plugin_id: plugin_id.clone(),
            },
            plugin_id,
        }
    }

    /// The live Lua state, loading the plugin if this is its first use.
    async fn instance(&self) -> Result<Arc<PluginInstance>, SourceError> {
        plugin::registry()
            .instance(&self.plugin_id)
            .await
            .map_err(SourceError::from)
    }
}

/// The id namespacing and table mapping, split from [`PluginSource`] so both
/// halves stay small and this one is testable without a database handle.
struct Mapper {
    plugin_id: String,
}

impl Mapper {
    /// Strip this plugin's namespace off an id the app handed us. An id that
    /// belongs to a different plugin is rejected rather than silently
    /// forwarded: that is how a queue restored from another source would
    /// otherwise end up asking the wrong backend for bytes.
    fn strip(&self, id: &str) -> Result<String, SourceError> {
        match plugin::split_item_id(id) {
            Some((owner, rest)) if owner == self.plugin_id => Ok(rest.to_string()),
            Some((owner, _)) => Err(SourceError::InvalidInput(format!(
                "{id} belongs to plugin {owner}, not {}",
                self.plugin_id
            ))),
            // Ids minted before the source was a plugin, or refs the plugin
            // itself returned unprefixed. Pass them through untouched.
            None => Ok(id.to_string()),
        }
    }

    fn strip_all(&self, ids: &[String]) -> Result<Vec<String>, SourceError> {
        ids.iter().map(|id| self.strip(id)).collect()
    }

    fn qualify(&self, item_ref: &str) -> String {
        plugin::namespace_item_id(&self.plugin_id, item_ref)
    }

    fn track(&self, t: plugin::PluginTrack) -> reader::Track {
        reader::Track {
            id: reader::TrackId::Server {
                service: MusicService::Plugin,
                item_id: self.qualify(&t.item_id),
            },
            cover: t.cover,
            album_id: if t.album_id.is_empty() {
                String::new()
            } else {
                self.qualify(&t.album_id)
            },
            title: t.title,
            artist: t.artist,
            album: t.album,
            duration: t.duration_secs,
            khz: t.khz,
            bitrate: t.bitrate,
            track_number: t.track_number,
            disc_number: t.disc_number,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: t.playlist_item_id,
            artists: t.artists,
        }
    }

    fn tracks(&self, items: Vec<plugin::PluginTrack>) -> Vec<reader::Track> {
        items.into_iter().map(|t| self.track(t)).collect()
    }

    fn album(&self, a: plugin::PluginAlbum) -> reader::Album {
        reader::Album {
            id: self.qualify(&a.album_id),
            title: a.title,
            artist: a.artist,
            genre: a.genre,
            year: a.year.unwrap_or(0),
            cover_path: a.cover.map(std::path::PathBuf::from),
            manual_cover: false,
        }
    }

    fn remote_album(&self, detail: plugin::PluginAlbumDetail) -> RemoteAlbum {
        RemoteAlbum {
            browse_id: self.qualify(&detail.album.album_id),
            title: detail.album.title,
            artist: (!detail.album.artist.is_empty()).then_some(detail.album.artist),
            year: detail.album.year.map(|y| y.to_string()),
            thumbnail: detail.album.cover,
            audio_playlist_id: detail.play_ref,
            tracks: self.tracks(detail.tracks),
        }
    }

    fn shelf(&self, s: plugin::PluginShelf) -> DiscoverShelf {
        DiscoverShelf {
            title: s.title,
            strapline: s.strapline,
            more_browse_id: s.more_ref,
            is_song_list: s.is_song_list,
            items: s.items.into_iter().map(|i| self.shelf_item(i)).collect(),
        }
    }

    fn shelf_item(&self, item: plugin::PluginShelfItem) -> DiscoverItem {
        match item {
            plugin::PluginShelfItem::Song { track } => {
                DiscoverItem::Song(Box::new(self.track(*track)))
            }
            plugin::PluginShelfItem::Album {
                album_id,
                title,
                subtitle,
                cover,
            } => DiscoverItem::Album {
                browse_id: self.qualify(&album_id),
                title,
                subtitle,
                thumbnail: cover,
            },
            plugin::PluginShelfItem::Artist {
                artist_id,
                name,
                image,
            } => DiscoverItem::Artist {
                channel_id: artist_id,
                name,
                thumbnail: image,
            },
            plugin::PluginShelfItem::Playlist {
                playlist_id,
                title,
                subtitle,
                cover,
            } => DiscoverItem::Playlist {
                playlist_id,
                title,
                subtitle,
                thumbnail: cover,
            },
            plugin::PluginShelfItem::Category { id, title, cover } => DiscoverItem::Mood {
                browse_id: id,
                title,
                thumbnail: cover,
            },
        }
    }

    fn discover(&self, result: plugin::DiscoverResult) -> DiscoverHome {
        DiscoverHome {
            shelves: result.shelves.into_iter().map(|s| self.shelf(s)).collect(),
            continuation: result.next,
        }
    }

    fn artist_images(&self, images: Vec<plugin::ArtistImage>) -> Vec<(String, String)> {
        images.into_iter().map(|i| (i.name, i.image)).collect()
    }
}

/// Degrade an operation the plugin does not implement to the default a source
/// without it would have produced. Only [`SourceError::Unsupported`] is
/// swallowed; a real failure still surfaces.
fn or_default<T: Default>(result: Result<T, SourceError>) -> Result<T, SourceError> {
    match result {
        Err(SourceError::Unsupported(_)) => Ok(T::default()),
        other => other,
    }
}

#[async_trait]
impl MediaSource for PluginSource {
    fn source(&self) -> &Source {
        &self.source
    }

    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        // Sync must be read off a loaded plugin, but this is called from render
        // paths that cannot await. The cached handshake answers once the plugin
        // has been contacted; until then the baseline is what makes the first
        // contact happen, since the sync task is what loads a fresh plugin.
        let caps = plugin::registry()
            .cached_capabilities(&self.plugin_id)
            .unwrap_or(Capabilities {
                sync: true,
                favorites_sync: FavoritesSync::Paginated,
                ..Capabilities::default()
            });
        Capabilities {
            // Offline downloads do not route through `resolve_stream`, so there
            // is no way to fetch a plugin's bytes for them. Forced off rather
            // than trusted, so a plugin declaring it cannot offer a dead
            // affordance (see docs/plugins.md).
            downloads: false,
            ..caps
        }
    }

    fn web_url(&self, track: &reader::Track) -> Option<String> {
        let item_id = track.id.key();
        let (_, item_ref) = plugin::split_item_id(&item_id)?;
        Some(
            plugin::registry()
                .cached_web_url_template(&self.plugin_id)?
                .replace("{id}", item_ref),
        )
    }

    // --- required remote-reaching ops --------------------------------------

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        let item_ref = self.map.strip(item_id)?;
        let result: plugin::StreamResult = self
            .instance()
            .await?
            .call(export::RESOLVE_STREAM, item_ref)
            .await?;
        if result.url.trim().is_empty() {
            return Err(SourceError::Backend(format!(
                "plugin {} returned an empty stream URL",
                self.plugin_id
            )));
        }
        Ok(StreamInfo {
            url: result.url,
            // No container hint: that selects the buffered-GET path, which is
            // the one that works for any format a plugin might serve.
            format: None,
            user_agent: result.user_agent,
            duration_secs: result.duration_secs,
            bitrate: result.bitrate,
            content_length: result.content_length,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        let instance = match self.instance().await {
            Ok(instance) => instance,
            Err(e) => {
                tracing::debug!(plugin = %self.plugin_id, error = %e, "plugin unreachable");
                return AuthOutcome::Unreachable;
            }
        };
        let outcome = instance
            .call::<_, plugin::AuthState>(export::VALIDATE, ())
            .await
            .map_err(SourceError::from);
        match outcome {
            Ok(state) => state.into(),
            Err(SourceError::Auth) => AuthOutcome::Expired,
            Err(SourceError::Unsupported(_)) => {
                // A plugin with nothing to authenticate against loaded and ran
                // its handshake, which is all "valid" means for it.
                AuthOutcome::Valid
            }
            Err(e) => {
                tracing::debug!(plugin = %self.plugin_id, error = %e, "plugin validate failed");
                AuthOutcome::Unreachable
            }
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let ids: Vec<String> = or_default(
            self.instance()
                .await?
                .call(export::FETCH_FAVORITES, ())
                .await
                .map_err(SourceError::from),
        )?;
        Ok(ids.iter().map(|id| self.map.qualify(id)).collect())
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        let item_ref = self.map.strip(item_id)?;
        self.instance()
            .await?
            .call_unit(export::PUSH_FAVORITE, (item_ref, on))
            .await
            .map_err(SourceError::from)
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        let refs = self.map.strip_all(item_refs)?;
        let landed: Vec<String> = self
            .instance()
            .await?
            .call(export::ADD_TO_PLAYLIST, (playlist_id.to_string(), refs))
            .await?;
        let landed: Vec<String> = landed.iter().map(|id| self.map.qualify(id)).collect();
        super::mirror_added(self.db(), self.source(), playlist_id, &landed).await?;
        Ok(landed)
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        let refs = self.map.strip_all(item_refs)?;
        let id: String = self
            .instance()
            .await?
            .call(export::CREATE_PLAYLIST, (name.to_string(), refs))
            .await?;
        super::mirror_created(self.db(), self.source(), &id, name, item_refs).await?;
        Ok(id)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        position: usize,
    ) -> Result<(), SourceError> {
        let item_ref = self.map.strip(&track.id.key())?;
        self.instance()
            .await?
            .call_unit(
                export::REMOVE_FROM_PLAYLIST,
                (
                    playlist_id.to_string(),
                    item_ref,
                    track.playlist_item_id.clone(),
                    position,
                ),
            )
            .await?;
        self.db()
            .remove_playlist_tracks(self.source(), playlist_id, &[track.id.key().into_owned()])
            .await
            .map_err(SourceError::from)
    }

    // --- capability-gated ops ----------------------------------------------

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_refs: &[String],
        moved: &reader::Track,
        new_index: usize,
    ) -> Result<(), SourceError> {
        let ordered = self.map.strip_all(ordered_refs)?;
        let item_ref = self.map.strip(&moved.id.key())?;
        self.instance()
            .await?
            .call_unit(
                export::REORDER_PLAYLIST,
                (
                    playlist_id.to_string(),
                    ordered,
                    item_ref,
                    moved.playlist_item_id.clone(),
                    new_index,
                ),
            )
            .await?;
        self.db()
            .set_playlist_tracks(self.source(), playlist_id, ordered_refs)
            .await
            .map_err(SourceError::from)
    }

    async fn start_radio(&self, seed_ref: &str) -> Result<Vec<reader::Track>, SourceError> {
        let seed = self.map.strip(seed_ref)?;
        let tracks: Vec<plugin::PluginTrack> = self
            .instance()
            .await?
            .call(export::START_RADIO, seed)
            .await?;
        Ok(self.map.tracks(tracks))
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let result: Result<plugin::SearchResult, SourceError> = self
            .instance()
            .await?
            .call(export::SEARCH, (q.to_string(), plugin::SEARCH_LIMIT))
            .await
            .map_err(SourceError::from);
        match result {
            Ok(result) => Ok((
                self.map.tracks(result.tracks),
                result
                    .albums
                    .into_iter()
                    .map(|a| self.map.album(a))
                    .collect(),
            )),
            // A plugin with no catalog search still has a synced library, and
            // the trait's default searches exactly that.
            Err(SourceError::Unsupported(_)) => {
                let tracks = self.db().search_corpus(self.source()).await?;
                let albums = self.db().albums(self.source()).await?;
                Ok(super::search_filter(&q.to_lowercase(), tracks, albums))
            }
            Err(e) => Err(e),
        }
    }

    async fn discover_home(&self) -> Result<DiscoverHome, SourceError> {
        let result: plugin::DiscoverResult = self
            .instance()
            .await?
            .call(export::DISCOVER_HOME, ())
            .await?;
        Ok(self.map.discover(result))
    }

    async fn discover_continuation(&self, token: &str) -> Result<DiscoverHome, SourceError> {
        let result: plugin::DiscoverResult = self
            .instance()
            .await?
            .call(export::DISCOVER_CONTINUATION, token.to_string())
            .await?;
        Ok(self.map.discover(result))
    }

    async fn fetch_album_tracks(&self, browse_id: &str) -> Result<Vec<reader::Track>, SourceError> {
        let album_ref = self.map.strip(browse_id)?;
        let tracks: Vec<plugin::PluginTrack> = self
            .instance()
            .await?
            .call(export::FETCH_ALBUM_TRACKS, album_ref)
            .await?;
        Ok(self.map.tracks(tracks))
    }

    async fn fetch_album(&self, browse_id: &str) -> Result<RemoteAlbum, SourceError> {
        let album_ref = self.map.strip(browse_id)?;
        let detail: plugin::PluginAlbumDetail = self
            .instance()
            .await?
            .call(export::FETCH_ALBUM, album_ref)
            .await?;
        Ok(self.map.remote_album(detail))
    }

    async fn fetch_album_by_ref(&self, id: &str) -> Result<Option<RemoteAlbum>, SourceError> {
        let album_ref = self.map.strip(id)?;
        let detail: Option<plugin::PluginAlbumDetail> = self
            .instance()
            .await?
            .call(export::FETCH_ALBUM_BY_REF, album_ref)
            .await?;
        Ok(detail.map(|d| self.map.remote_album(d)))
    }

    async fn fetch_album_by_meta(
        &self,
        title: &str,
        artist: &str,
    ) -> Result<Option<RemoteAlbum>, SourceError> {
        let detail: Option<plugin::PluginAlbumDetail> = self
            .instance()
            .await?
            .call(
                export::FETCH_ALBUM_BY_META,
                (title.to_string(), artist.to_string()),
            )
            .await?;
        Ok(detail.map(|d| self.map.remote_album(d)))
    }

    async fn fetch_playlist_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<(Vec<reader::Track>, Option<String>), SourceError> {
        let page = self
            .fetch_playlist_entries_page(playlist_id, cursor)
            .await?;
        Ok((page.tracks, page.next))
    }

    async fn resolve_artist_channel_id(&self, query: &str) -> Result<Option<String>, SourceError> {
        self.instance()
            .await?
            .call(export::RESOLVE_ARTIST_ID, query.to_string())
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_album_browse_id(
        &self,
        album: &str,
        artist: &str,
    ) -> Result<Option<String>, SourceError> {
        let id: Option<String> = self
            .instance()
            .await?
            .call(
                export::RESOLVE_ALBUM_ID,
                (album.to_string(), artist.to_string()),
            )
            .await?;
        Ok(id.map(|id| self.map.qualify(&id)))
    }

    async fn fetch_artist(&self, channel_id: &str) -> Result<YtArtist, SourceError> {
        let page: plugin::PluginArtistPage = self
            .instance()
            .await?
            .call(export::FETCH_ARTIST, channel_id.to_string())
            .await?;
        Ok(YtArtist {
            channel_id: page.artist_id,
            name: page.name,
            subscribers: page.subtitle,
            description: page.description,
            banner_thumbnail: page.banner,
            shuffle_playlist_id: page.shuffle_ref,
            sections: page
                .shelves
                .into_iter()
                .map(|s| self.map.shelf(s))
                .collect(),
        })
    }

    // --- remote reads -------------------------------------------------------

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let result: plugin::LibraryResult = or_default(
            self.instance()
                .await?
                .call(export::FETCH_LIBRARY, ())
                .await
                .map_err(SourceError::from),
        )?;
        Ok(LibrarySnapshot {
            albums: result
                .albums
                .into_iter()
                .map(|a| self.map.album(a))
                .collect(),
            tracks: self.map.tracks(result.tracks),
            artist_images: self.map.artist_images(result.artist_images),
        })
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        let metas: Vec<plugin::PluginPlaylistMeta> = or_default(
            self.instance()
                .await?
                .call(export::FETCH_PLAYLISTS, ())
                .await
                .map_err(SourceError::from),
        )?;
        Ok(metas
            .into_iter()
            .map(|p| PlaylistMeta {
                id: p.playlist_id,
                name: p.name,
                image_tag: p.image,
            })
            .collect())
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        let mut all = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .fetch_playlist_entries_page(playlist_id, cursor)
                .await?;
            all.extend(page.tracks);
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(all),
            }
        }
    }

    async fn fetch_playlist_entries_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<PlaylistPage, SourceError> {
        let page: plugin::TrackPage = or_default(
            self.instance()
                .await?
                .call(
                    export::FETCH_PLAYLIST_ENTRIES_PAGE,
                    (playlist_id.to_string(), cursor),
                )
                .await
                .map_err(SourceError::from),
        )?;
        Ok(PlaylistPage {
            tracks: self.map.tracks(page.tracks),
            next: page.next,
        })
    }

    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        let page: plugin::TrackPage = or_default(
            self.instance()
                .await?
                .call(export::FETCH_FAVORITES_PAGE, cursor)
                .await
                .map_err(SourceError::from),
        )?;
        Ok(FavoritesPage {
            tracks: self.map.tracks(page.tracks),
            next: page.next,
        })
    }

    async fn fetch_artist_images(&self) -> Result<Vec<(String, String)>, SourceError> {
        let images: Vec<plugin::ArtistImage> = or_default(
            self.instance()
                .await?
                .call(export::FETCH_ARTIST_IMAGES, ())
                .await
                .map_err(SourceError::from),
        )?;
        Ok(self.map.artist_images(images))
    }

    async fn fetch_artist_image(&self, name: &str) -> Result<Option<String>, SourceError> {
        or_default(
            self.instance()
                .await?
                .call(export::FETCH_ARTIST_IMAGE, name.to_string())
                .await
                .map_err(SourceError::from),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Mapper {
        Mapper {
            plugin_id: "example".into(),
        }
    }

    #[test]
    fn track_ids_are_namespaced() {
        let map = fixture();
        let track = map.track(plugin::PluginTrack {
            item_id: "t1".into(),
            title: "Title".into(),
            artist: "Artist".into(),
            artists: vec!["Artist".into()],
            album: "Album".into(),
            album_id: "a1".into(),
            cover: Some("directurl:https://example.test/a.jpg".into()),
            duration_secs: 210,
            khz: 44_100,
            bitrate: 320,
            track_number: Some(3),
            disc_number: None,
            playlist_item_id: None,
        });
        assert_eq!(
            track.id,
            reader::TrackId::Server {
                service: MusicService::Plugin,
                item_id: "example/t1".into()
            }
        );
        assert_eq!(track.album_id, "example/a1");
        assert_eq!(track.id.uid(), "plugin:example/t1");
    }

    #[test]
    fn empty_album_ids_stay_empty() {
        let map = fixture();
        let track = map.track(plugin::PluginTrack {
            item_id: "t1".into(),
            title: "T".into(),
            artist: String::new(),
            artists: Vec::new(),
            album: String::new(),
            album_id: String::new(),
            cover: None,
            duration_secs: 0,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            playlist_item_id: None,
        });
        assert!(
            track.album_id.is_empty(),
            "no album id must not become a bare prefix"
        );
    }

    #[test]
    fn strip_round_trips_and_rejects_other_plugins() {
        let map = fixture();
        assert_eq!(map.strip("example/t1").expect("own id"), "t1");
        // Unprefixed ids pass through: legacy rows and plugin-authored refs.
        assert_eq!(map.strip("bare").expect("bare id"), "bare");
        assert!(matches!(
            map.strip("other/t1"),
            Err(SourceError::InvalidInput(_))
        ));
    }

    #[test]
    fn artist_images_become_name_url_pairs() {
        let map = fixture();
        let pairs = map.artist_images(vec![plugin::ArtistImage {
            name: "Artist".into(),
            image: "https://example.test/a.jpg".into(),
        }]);
        assert_eq!(
            pairs,
            vec![(
                "Artist".to_string(),
                "https://example.test/a.jpg".to_string()
            )]
        );
    }

    #[test]
    fn unsupported_degrades_to_the_default() {
        let unsupported: Result<Vec<String>, _> = Err(SourceError::unsupported("x"));
        assert_eq!(
            or_default(unsupported).expect("degrades"),
            Vec::<String>::new()
        );
        let real: Result<Vec<String>, _> = Err(SourceError::Connectivity);
        assert!(or_default(real).is_err(), "a real failure still surfaces");
    }
}
