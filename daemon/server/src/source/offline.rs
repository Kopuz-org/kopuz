use async_trait::async_trait;
use config::Source;
use db::Db;

use super::{
    AlbumType, ArtistView, AuthOutcome, Capabilities, FavoritesSync, LibraryActions, MediaSource,
    PlaylistOps, RadioSeeds, SourceError, StreamInfo,
};

pub(super) struct OfflineServerSource {
    pub(super) db: Db,
    pub(super) source: Source,
}

#[async_trait]
impl MediaSource for OfflineServerSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            browse_folders: false,
            external_devices: false,
            browser_playback: false,
            accounts: false,
            sync: false,
            downloads: false,
            uploads: false,
            storage_quota: false,
            discover: false,
            dont_recommend: false,
            music_videos: false,
            radio: RadioSeeds::NONE,
            play_queue: false,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
            account_avatar: false,
            stream_quality: false,
            explicit_flags: false,
            watch_history: false,
            complete_play_history: false,
            library_actions: LibraryActions::NONE,
        }
    }

    async fn add_to_playlist(&self, _: &str, _: &[String]) -> Result<Vec<String>, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn create_playlist(&self, _: &str, _: &[String]) -> Result<String, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn remove_from_playlist(
        &self,
        _: &str,
        _: &reader::Track,
        _: usize,
    ) -> Result<(), SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn resolve_stream(
        &self,
        _: &str,
        _: config::StreamQuality,
    ) -> Result<StreamInfo, SourceError> {
        Err(SourceError::Auth)
    }
    async fn validate(&self) -> AuthOutcome {
        AuthOutcome::Unreachable
    }
    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        Err(SourceError::Connectivity)
    }
    async fn push_favorite(&self, _: &str, _: bool) -> Result<(), SourceError> {
        Err(SourceError::Connectivity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_listing_names_only_the_artists_it_gives_an_id() {
        let dir = tempfile::tempdir().unwrap();
        let db = db::init(&dir.path().join("kopuz.db")).await.unwrap();
        let src = OfflineServerSource {
            db,
            source: Source::Server("srv".into()),
        };
        let track = |key: &str, credit: reader::ArtistCredit| reader::Track {
            id: reader::TrackId::Server {
                service: config::MusicService::Jellyfin,
                item_id: key.into(),
            },
            cover: None,
            album_id: String::new(),
            title: key.into(),
            artist: credit.name.clone(),
            album: String::new(),
            duration: 1,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![credit.name.clone()],
            replay_gain: config::ReplayGainInfo::default(),
            credits: vec![credit],
            counterpart: None,
            explicit: false,
            plays: None,
        };
        src.upsert_tracks(&[
            track(
                "t1",
                reader::ArtistCredit::linked("Ada feat. Boris", "ar-ada"),
            ),
            track("t2", reader::ArtistCredit::unlinked("Cleo")),
        ])
        .await
        .unwrap();

        let renamed = src
            .name_listed_artists(&[
                (reader::ArtistCredit::linked("Ada", "ar-ada"), None),
                (
                    reader::ArtistCredit::unlinked("Cleo Renamed"),
                    Some("u".into()),
                ),
            ])
            .await
            .unwrap();

        assert_eq!(renamed, 1);
        let names: Vec<String> = src
            .db
            .artists(&src.source)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.name)
            .collect();
        assert!(names.contains(&"Ada".to_string()), "{names:?}");
        assert!(
            names.contains(&"Cleo".to_string()),
            "a bare name renames nobody"
        );
    }
}
