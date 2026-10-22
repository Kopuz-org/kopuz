//! A [`MediaSource`] whose remote half runs in a sandboxed plugin. The
//! database stays here: what a call changed remotely is mirrored once the
//! plugin reports it landed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use config::{MusicService, Source};
use db::Db;
use plugin_host::manifest;
use plugin_host::{PluginError, Session, wit};

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, Check, CheckOutcome, CheckStep,
    FavoritesPage, FavoritesSync, LibraryActions, LibrarySnapshot, MediaSource, PlaylistMeta,
    PlaylistOps, PlaylistPage, RadioPage, RadioSeeds, RemotePlayQueue, SourceError, StorageQuota,
    StreamInfo, mirror_added, mirror_created,
};

pub(super) struct PluginSource {
    db: Db,
    source: Source,
    service: MusicService,
    session: Arc<Session>,
    capabilities: Capabilities,
}

impl PluginSource {
    pub(super) fn new(
        db: Db,
        source: Source,
        service: MusicService,
        connection: wit::Connection,
    ) -> Option<Self> {
        let slug = connection.service.clone();
        let session = crate::plugins::session(connection)?;
        let capabilities = session
            .plugin()
            .manifest()
            .service(&slug)
            .map(|declared| capabilities(&declared.capabilities))?;
        Some(Self {
            db,
            source,
            service,
            session,
            capabilities,
        })
    }

    fn track(&self, track: wit::Track) -> reader::Track {
        track_from(self.service, track)
    }

    fn tracks(&self, tracks: Vec<wit::Track>) -> Vec<reader::Track> {
        tracks.into_iter().map(|track| self.track(track)).collect()
    }
}

fn answer<T>(result: Result<Result<T, wit::SourceError>, PluginError>) -> Result<T, SourceError> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(source_error(error)),
        Err(error) => Err(SourceError::Backend(error.to_string())),
    }
}

pub(crate) fn source_error(error: wit::SourceError) -> SourceError {
    match error {
        wit::SourceError::Unsupported(operation) => SourceError::Unsupported(intern(operation)),
        wit::SourceError::Connectivity => SourceError::Connectivity,
        wit::SourceError::Auth => SourceError::Auth,
        wit::SourceError::InvalidInput(message) => SourceError::InvalidInput(message),
        wit::SourceError::Backend(message) => SourceError::Backend(message),
    }
}

/// `Unsupported` takes a `&'static str`; a plugin's few names leak once each.
fn intern(operation: String) -> &'static str {
    static NAMES: OnceLock<Mutex<std::collections::HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(name) = names.get(operation.as_str()) {
        return name;
    }
    let name: &'static str = Box::leak(operation.into_boxed_str());
    names.insert(name);
    name
}

pub(crate) fn track_from(service: MusicService, track: wit::Track) -> reader::Track {
    reader::Track {
        id: reader::TrackId::Server {
            service,
            item_id: track.item_id,
        },
        cover: track.cover,
        album_id: track.album_id,
        title: track.title,
        artist: track.artist,
        album: track.album,
        duration: track.duration_secs,
        khz: track.sample_rate,
        bitrate: track.bitrate_kbps.min(u16::MAX as u32) as u16,
        track_number: track.track_number,
        disc_number: track.disc_number,
        musicbrainz_release_id: track.musicbrainz_release_id,
        musicbrainz_recording_id: track.musicbrainz_recording_id,
        musicbrainz_track_id: track.musicbrainz_track_id,
        playlist_item_id: track.playlist_item_id,
        artists: track.artists,
        credits: track.credits.into_iter().map(credit_from).collect(),
        replay_gain: config::ReplayGainInfo {
            track_gain_db: track.replay_gain.track_gain_db,
            track_peak: track.replay_gain.track_peak,
            album_gain_db: track.replay_gain.album_gain_db,
            album_peak: track.replay_gain.album_peak,
            loudness_db: None,
        },
        explicit: track.explicit,
        plays: None,
        counterpart: None,
    }
}

fn credit_from(credit: wit::ArtistCredit) -> reader::ArtistCredit {
    match credit.id {
        Some(id) => reader::ArtistCredit::linked(credit.name, id),
        None => reader::ArtistCredit::unlinked(credit.name),
    }
}

fn album_from(album: wit::Album) -> reader::Album {
    reader::Album {
        id: album.id,
        title: album.title,
        artist: album.artist,
        genre: album.genre,
        year: album.year,
        cover_path: album.cover.map(PathBuf::from),
        manual_cover: false,
        artist_key: album.artist_id.clone(),
        artist_id: album.artist_id,
    }
}

fn listing_from(listing: wit::ArtistListing) -> (reader::ArtistCredit, Option<String>) {
    (credit_from(listing.credit), listing.photo)
}

fn entry_of(entry: &reader::PlaylistEntry) -> wit::PlaylistEntry {
    wit::PlaylistEntry {
        key: entry.key.clone(),
        playlist_item_id: entry.item_id.clone(),
    }
}

fn entry_of_track(track: &reader::Track) -> wit::PlaylistEntry {
    wit::PlaylistEntry {
        key: track.id.key().into_owned(),
        playlist_item_id: track.playlist_item_id.clone(),
    }
}

fn check_from(check: wit::Check) -> Check {
    Check {
        step: match check.step {
            wit::CheckStep::Reach => CheckStep::Reach,
            wit::CheckStep::Session => CheckStep::Session,
            wit::CheckStep::SignedInPlayback => CheckStep::SignedInPlayback,
            wit::CheckStep::AnonymousPlayback => CheckStep::AnonymousPlayback,
            wit::CheckStep::PlaybackToken => CheckStep::PlaybackToken,
        },
        outcome: match check.outcome {
            wit::CheckOutcome::Passed => CheckOutcome::Passed,
            wit::CheckOutcome::Failed => CheckOutcome::Failed,
            wit::CheckOutcome::Skipped => CheckOutcome::Skipped,
        },
        detail: check.detail,
    }
}

fn quality_of(quality: config::StreamQuality) -> wit::StreamQuality {
    match quality {
        config::StreamQuality::Low => wit::StreamQuality::Low,
        config::StreamQuality::Normal => wit::StreamQuality::Normal,
        config::StreamQuality::High => wit::StreamQuality::High,
    }
}

fn radio_page(page: wit::TrackPage, service: MusicService) -> RadioPage {
    RadioPage {
        tracks: page
            .tracks
            .into_iter()
            .map(|track| track_from(service, track))
            .collect(),
        more: page.next,
    }
}

pub(crate) fn capabilities(declared: &manifest::Capabilities) -> Capabilities {
    let seeds = |seed| declared.radio.contains(&seed);
    let action = |action| declared.library_actions.contains(&action);
    Capabilities {
        edit_tags: declared.edit_tags,
        delete_from_disk: declared.delete_from_disk,
        scan_folders: declared.scan_folders,
        folders: declared.folders,
        browse_folders: declared.browse_folders,
        external_devices: declared.external_devices,
        browser_playback: declared.browser_playback,
        accounts: declared.accounts,
        sync: declared.sync,
        downloads: declared.downloads,
        uploads: declared.uploads,
        storage_quota: declared.storage_quota,
        discover: declared.discover,
        dont_recommend: declared.dont_recommend,
        music_videos: declared.music_videos,
        radio: RadioSeeds {
            track: seeds(manifest::RadioSeed::Track),
            playlist: seeds(manifest::RadioSeed::Playlist),
            search: seeds(manifest::RadioSeed::Search),
        },
        play_queue: declared.play_queue,
        playlists: match declared.playlists {
            manifest::PlaylistOps::None => PlaylistOps::None,
            manifest::PlaylistOps::AddRemove => PlaylistOps::AddRemove,
            manifest::PlaylistOps::Reorder => PlaylistOps::Reorder,
        },
        artist_view: match declared.artist_view {
            manifest::ArtistView::Library => ArtistView::Library,
            manifest::ArtistView::Remote => ArtistView::Remote,
        },
        albums: AlbumType::Standard,
        favorites_sync: match declared.favorites_sync {
            manifest::FavoritesSync::Instant => FavoritesSync::Instant,
            manifest::FavoritesSync::Paginated => FavoritesSync::Paginated,
        },
        account_avatar: declared.account_avatar,
        stream_quality: declared.stream_quality,
        explicit_flags: declared.explicit_flags,
        watch_history: declared.watch_history,
        complete_play_history: declared.complete_play_history,
        library_actions: LibraryActions {
            rate: action(manifest::LibraryAction::Rate),
            follow: action(manifest::LibraryAction::Follow),
            save: action(manifest::LibraryAction::Save),
            remove_from_history: action(manifest::LibraryAction::RemoveFromHistory),
            playlist_details: action(manifest::LibraryAction::PlaylistDetails),
        },
    }
}

fn image_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
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
        self.capabilities
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        let added = answer(self.session.add_to_playlist(playlist_id, item_refs).await)?;
        mirror_added(&self.db, &self.source, playlist_id, &added).await?;
        Ok(added)
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        let id = answer(self.session.create_playlist(name, item_refs).await)?;
        mirror_created(&self.db, &self.source, &id, name, item_refs).await?;
        Ok(id)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        position: usize,
    ) -> Result<(), SourceError> {
        let position_u32 = u32::try_from(position)
            .map_err(|_| SourceError::InvalidInput("playlist position out of range".into()))?;
        answer(
            self.session
                .remove_from_playlist(playlist_id, &entry_of_track(track), position_u32)
                .await,
        )?;
        self.remove_playlist_entry(playlist_id, position).await
    }

    async fn resolve_stream(
        &self,
        item_id: &str,
        quality: config::StreamQuality,
    ) -> Result<StreamInfo, SourceError> {
        let stream = answer(
            self.session
                .resolve_stream(item_id, quality_of(quality))
                .await,
        )?;
        Ok(StreamInfo {
            url: if stream.hls {
                format!("__SC_HLS:{}", stream.url)
            } else {
                stream.url
            },
            format: None,
            user_agent: stream.user_agent,
            duration_secs: stream.duration_secs,
            bitrate: stream.bitrate,
            content_length: stream.content_length,
            loudness_db: stream.loudness_db,
        })
    }

    async fn download_track(
        &self,
        item_id: &str,
        _progress: Option<crate::stream::stream_buffer::BufferProgressCallback>,
    ) -> Result<Vec<u8>, SourceError> {
        answer(self.session.download_track(item_id).await)
    }

    async fn validate(&self) -> AuthOutcome {
        match self.session.validate().await {
            Ok(wit::AuthOutcome::Valid) => AuthOutcome::Valid,
            Ok(wit::AuthOutcome::Expired) => AuthOutcome::Expired,
            Ok(wit::AuthOutcome::Unreachable) | Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn diagnose(&self) -> Vec<Check> {
        match answer(self.session.diagnose().await) {
            Ok(checks) => checks.into_iter().map(check_from).collect(),
            Err(error) => vec![Check {
                step: CheckStep::Reach,
                outcome: CheckOutcome::Failed,
                detail: error.to_string(),
            }],
        }
    }

    async fn storage_quota(&self) -> Result<StorageQuota, SourceError> {
        let quota = answer(self.session.storage_quota().await)?;
        Ok(StorageQuota {
            used_bytes: quota.used_bytes,
            quota_bytes: quota.quota_bytes,
            remaining_bytes: quota.remaining_bytes,
        })
    }

    async fn upload_track(&self, filename: String, content: Vec<u8>) -> Result<(), SourceError> {
        answer(self.session.upload_track(&filename, &content).await)
    }

    /// Without favorites of its own, the remote set is kopuz's, so a
    /// reconcile changes nothing.
    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        match answer(self.session.fetch_favorites().await) {
            Err(SourceError::Unsupported(_)) => self
                .db
                .favorites(self.source.as_str())
                .await
                .map_err(SourceError::from),
            result => result,
        }
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        match answer(self.session.push_favorite(item_id, on).await) {
            Err(SourceError::Unsupported(_)) => Ok(()),
            result => result,
        }
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered: &[reader::PlaylistEntry],
        moved: &reader::Track,
        new_index: usize,
    ) -> Result<(), SourceError> {
        let new_index_u32 = u32::try_from(new_index)
            .map_err(|_| SourceError::InvalidInput("playlist position out of range".into()))?;
        let entries: Vec<wit::PlaylistEntry> = ordered.iter().map(entry_of).collect();
        answer(
            self.session
                .reorder_playlist(playlist_id, &entries, &entry_of_track(moved), new_index_u32)
                .await,
        )?;
        self.db
            .set_playlist_tracks(&self.source, playlist_id, ordered)
            .await
            .map_err(SourceError::from)
    }

    async fn delete_remote_playlist(&self, playlist_id: &str) -> Result<(), SourceError> {
        answer(self.session.delete_remote_playlist(playlist_id).await)
    }

    async fn start_radio(&self, seed_ref: &str) -> Result<RadioPage, SourceError> {
        let page = answer(self.session.start_radio(seed_ref).await)?;
        Ok(radio_page(page, self.service))
    }

    async fn start_playlist_radio(&self, playlist_ref: &str) -> Result<RadioPage, SourceError> {
        let page = answer(self.session.start_playlist_radio(playlist_ref).await)?;
        Ok(radio_page(page, self.service))
    }

    async fn more_radio(&self, cursor: &str) -> Result<RadioPage, SourceError> {
        let page = answer(self.session.more_radio(cursor).await)?;
        Ok(radio_page(page, self.service))
    }

    async fn save_play_queue(
        &self,
        queue: &[reader::Track],
        current_index: usize,
        position_ms: u64,
    ) -> Result<bool, SourceError> {
        let mut ids = Vec::with_capacity(queue.len());
        for track in queue {
            match &track.id {
                reader::TrackId::Server { service, item_id } if *service == self.service => {
                    ids.push(item_id.clone())
                }
                // A queue the server cannot name every entry of would come back short.
                _ => return Ok(false),
            }
        }
        let current_index = u32::try_from(current_index).unwrap_or(u32::MAX);
        answer(
            self.session
                .save_play_queue(&ids, current_index, position_ms)
                .await,
        )
    }

    async fn get_play_queue(&self) -> Result<Option<RemotePlayQueue>, SourceError> {
        let Some(queue) = answer(self.session.get_play_queue().await)? else {
            return Ok(None);
        };
        Ok(Some(RemotePlayQueue {
            tracks: self.tracks(queue.tracks),
            current_id: queue.current_id,
            position_ms: queue.position_ms,
            changed_by: queue.changed_by,
        }))
    }

    async fn fetch_track(&self, item_id: &str) -> Result<Option<reader::Track>, SourceError> {
        Ok(answer(self.session.fetch_track(item_id).await)?.map(|track| self.track(track)))
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        match answer(self.session.search(query).await) {
            Ok(results) => Ok((
                self.tracks(results.tracks),
                results.albums.into_iter().map(album_from).collect(),
            )),
            Err(SourceError::Unsupported(_)) => {
                let tracks = self.db.search_corpus(&self.source).await?;
                let albums = self.db.albums(&self.source).await?;
                Ok(super::search::filter(&q, tracks, albums))
            }
            Err(error) => Err(error),
        }
    }

    async fn fetch_album_tracks(&self, browse_id: &str) -> Result<Vec<reader::Track>, SourceError> {
        Ok(self.tracks(answer(self.session.fetch_album_tracks(browse_id).await)?))
    }

    async fn fetch_library(&self) -> Result<LibrarySnapshot, SourceError> {
        let snapshot = answer(self.session.fetch_library().await)?;
        Ok(LibrarySnapshot {
            albums: snapshot.albums.into_iter().map(album_from).collect(),
            tracks: self.tracks(snapshot.tracks),
            artists: snapshot.artists.into_iter().map(listing_from).collect(),
        })
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        Ok(self.tracks(answer(
            self.session.fetch_playlist_entries(playlist_id).await,
        )?))
    }

    async fn fetch_playlist_entries_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<PlaylistPage, SourceError> {
        let page = answer(
            self.session
                .fetch_playlist_page(playlist_id, cursor.as_deref())
                .await,
        )?;
        Ok(PlaylistPage {
            tracks: self.tracks(page.tracks),
            next: page.next,
            header: None,
        })
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        Ok(answer(self.session.fetch_playlists().await)?
            .into_iter()
            .map(|playlist| PlaylistMeta {
                id: playlist.id,
                name: playlist.name,
                image_tag: playlist.image_tag,
            })
            .collect())
    }

    async fn fetch_artists(
        &self,
    ) -> Result<Vec<(reader::ArtistCredit, Option<String>)>, SourceError> {
        Ok(answer(self.session.fetch_artists().await)?
            .into_iter()
            .map(listing_from)
            .collect())
    }

    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        let page = answer(self.session.fetch_favorites_page(cursor.as_deref()).await)?;
        Ok(FavoritesPage {
            tracks: self.tracks(page.tracks),
            next: page.next,
        })
    }

    async fn scrobble_now_playing(&self, item_id: &str) -> Result<(), SourceError> {
        answer(self.session.scrobble_now_playing(item_id).await)
    }

    async fn scrobble(&self, item_id: &str) -> Result<(), SourceError> {
        answer(self.session.scrobble(item_id).await)
    }

    async fn keepalive(&self) -> Result<(), SourceError> {
        answer(self.session.keepalive().await)
    }

    async fn report_playback_start(&self, item_id: &str) -> Result<(), SourceError> {
        answer(self.session.report_playback_start(item_id).await)
    }

    async fn report_playback_stopped(
        &self,
        item_id: &str,
        position_ticks: u64,
    ) -> Result<(), SourceError> {
        answer(
            self.session
                .report_playback_stopped(item_id, position_ticks)
                .await,
        )
    }

    async fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: u64,
        is_paused: bool,
    ) -> Result<(), SourceError> {
        answer(
            self.session
                .report_playback_progress(item_id, position_ticks, is_paused)
                .await,
        )
    }

    /// The upload is best-effort; the local cover is what the person sees.
    async fn set_playlist_cover(
        &self,
        playlist_id: &str,
        name: &str,
        image_path: &Path,
        image_tag: Option<&str>,
    ) -> Result<(), SourceError> {
        if let Ok(bytes) = tokio::fs::read(image_path).await {
            let content_type = image_content_type(image_path);
            match answer(
                self.session
                    .set_playlist_image(playlist_id, &bytes, content_type)
                    .await,
            ) {
                Ok(()) | Err(SourceError::Unsupported(_)) => {}
                Err(error) => {
                    tracing::debug!(%error, %playlist_id, "playlist image upload failed");
                }
            }
        }
        let cover = image_path.to_string_lossy();
        self.db
            .upsert_playlist_meta(&self.source, playlist_id, name, Some(&cover), image_tag)
            .await
            .map_err(SourceError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_track_is_filed_under_the_session_service() {
        let track = track_from(
            MusicService::Custom,
            wit::Track {
                item_id: "s1".into(),
                cover: Some("custom:al-1".into()),
                album_id: "custom:al-1".into(),
                title: "Song".into(),
                artist: "Ada".into(),
                album: "Album".into(),
                duration_secs: 200,
                sample_rate: 44_100,
                bitrate_kbps: 100_000,
                track_number: Some(3),
                disc_number: None,
                musicbrainz_release_id: None,
                musicbrainz_recording_id: None,
                musicbrainz_track_id: None,
                playlist_item_id: None,
                artists: vec!["Ada".into()],
                credits: vec![wit::ArtistCredit {
                    name: "Ada".into(),
                    id: Some("ar-1".into()),
                }],
                replay_gain: wit::ReplayGain {
                    track_gain_db: Some(-6.0),
                    track_peak: None,
                    album_gain_db: None,
                    album_peak: None,
                },
                explicit: false,
            },
        );
        assert_eq!(track.id.uid(), "custom:s1");
        assert_eq!(track.bitrate, u16::MAX, "an oversized bitrate saturates");
        assert_eq!(track.credits[0].key.as_deref(), Some("ar-1"));
        assert_eq!(track.replay_gain.track_gain_db, Some(-6.0));
    }

    #[test]
    fn an_unsupported_operation_keeps_its_name() {
        let error = source_error(wit::SourceError::Unsupported("radio".into()));
        assert_eq!(error, SourceError::Unsupported("radio"));
        let again = source_error(wit::SourceError::Unsupported("radio".into()));
        let (SourceError::Unsupported(first), SourceError::Unsupported(second)) = (error, again)
        else {
            unreachable!()
        };
        assert!(std::ptr::eq(first, second), "one name is leaked once");
    }
}
