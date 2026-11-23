//! AudioMuse-AI picks, resolved against the active source's library.

use api::{ApiError, ErrorCode, ErrorReason};
use server::audiomuse::{AudioMuse, AudioMuseError};
use server::source::RadioPage;

use super::{CatalogService, db_error};

pub(super) mod sound;

/// Songs a text search asks for.
const DESCRIBE_LIMIT: usize = 50;
/// Songs a listening-profile pick asks for.
const FINGERPRINT_COUNT: usize = 50;

fn audiomuse_error(error: AudioMuseError) -> ApiError {
    match error {
        AudioMuseError::Rejected => {
            ApiError::invalid_input(error.to_string()).because(ErrorReason::CredentialRejected)
        }
        AudioMuseError::Unreachable => {
            ApiError::new(ErrorCode::SourceUnreachable, error.to_string())
                .because(ErrorReason::IntegrationUnreachable)
        }
        AudioMuseError::BadUrl | AudioMuseError::Status(_) | AudioMuseError::Malformed => {
            ApiError::internal(error.to_string())
        }
    }
}

impl CatalogService {
    pub fn attach_credentials(&self, credentials: crate::CredentialStore) {
        let _ = self.credentials.set(credentials);
    }

    /// The address and token held for AudioMuse-AI, `None` while either is unset.
    fn audiomuse_login(&self) -> Option<(String, String)> {
        let held = self.credentials.get()?.snapshot();
        if held.audiomuse_url.trim().is_empty() || held.audiomuse_token.trim().is_empty() {
            return None;
        }
        Some((held.audiomuse_url, held.audiomuse_token))
    }

    fn audiomuse(&self) -> Result<AudioMuse, ApiError> {
        let Some((url, token)) = self.audiomuse_login() else {
            return Err(ApiError::unsupported("AudioMuse-AI is not set up"));
        };
        AudioMuse::new(&url, &token).map_err(audiomuse_error)
    }

    pub async fn described_radio(&self, query: &str) -> Result<RadioPage, ApiError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(ApiError::invalid_input("describe the music to look for"));
        }
        let ids = self
            .audiomuse()?
            .describe(query, DESCRIBE_LIMIT)
            .await
            .map_err(audiomuse_error)?;
        Ok(RadioPage::last(self.in_library(ids).await?))
    }

    pub async fn fingerprint_radio(&self) -> Result<RadioPage, ApiError> {
        let ids = self
            .audiomuse()?
            .fingerprint(FINGERPRINT_COUNT)
            .await
            .map_err(audiomuse_error)?;
        Ok(RadioPage::last(self.in_library(ids).await?))
    }

    async fn in_library(&self, ids: Vec<String>) -> Result<Vec<reader::Track>, ApiError> {
        in_library(&self.db, &self.session.sources().active, &ids).await
    }
}

/// AudioMuse's song ids are track keys; those the library lacks drop out and the order stays.
async fn in_library(
    db: &db::Db,
    source: &config::Source,
    ids: &[String],
) -> Result<Vec<reader::Track>, ApiError> {
    let tracks = db.tracks_by_keys(source, ids).await.map_err(db_error)?;
    if tracks.is_empty() {
        return Err(ApiError::not_found(
            "nothing AudioMuse-AI found is in this library",
        ));
    }
    Ok(tracks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str) -> reader::Track {
        reader::Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(key)),
            cover: None,
            album_id: String::new(),
            title: key.to_string(),
            artist: String::new(),
            album: String::new(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            bit_depth: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            credits: Vec::new(),
            artists: Vec::new(),
            replay_gain: config::ReplayGainInfo::default(),
            explicit: false,
            plays: None,
            counterpart: None,
        }
    }

    fn ids(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|key| key.to_string()).collect()
    }

    #[tokio::test]
    async fn unknown_ids_drop_out_and_the_rest_keep_audiomuses_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = db::init(&dir.path().join("muse.db")).await.expect("db");
        let source = config::Source::default();
        db.upsert_tracks(&source, &[track("/a"), track("/b"), track("/c")])
            .await
            .expect("tracks");

        let found = in_library(&db, &source, &ids(&["/c", "/gone", "/a"]))
            .await
            .expect("two are known");

        let titles: Vec<&str> = found.iter().map(|track| track.title.as_str()).collect();
        assert_eq!(titles, ["/c", "/a"]);
    }

    #[tokio::test]
    async fn nothing_in_the_library_is_an_error_not_an_empty_queue() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = db::init(&dir.path().join("muse.db")).await.expect("db");

        let error = in_library(&db, &config::Source::default(), &ids(&["/gone"]))
            .await
            .expect_err("no match");

        assert_eq!(error.code, ErrorCode::NotFound);
    }
}
