//! AudioMuse-AI picks, resolved against the active source's library.

use std::sync::{Arc, Weak};

use api::{ApiError, ErrorCode, ErrorReason};
use server::audiomuse::{AudioMuse, AudioMuseError, Score};
use server::source::RadioPage;

use super::{CatalogService, db_error};
use crate::session::{MixAdvice, MixPlanner, MixRefusal};

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
        AudioMuseError::BadUrl => {
            ApiError::invalid_input(error.to_string()).because(ErrorReason::InvalidServerUrl)
        }
        AudioMuseError::Status(_)
        | AudioMuseError::Malformed
        | AudioMuseError::NotOffered
        | AudioMuseError::NoTransition => {
            ApiError::internal(error.to_string()).because(ErrorReason::IntegrationFailed)
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

impl CatalogService {
    /// The session's mix planner. Weak, because the catalog holds the session.
    pub(crate) fn mix_planner(self: &Arc<Self>) -> Arc<dyn MixPlanner> {
        Arc::new(AudioMuseMixes(Arc::downgrade(self)))
    }
}

/// Smart-crossfade advice from AudioMuse-AI, with the login the catalog holds.
struct AudioMuseMixes(Weak<CatalogService>);

#[async_trait::async_trait]
impl MixPlanner for AudioMuseMixes {
    async fn advise(&self, from: &str, to: &str) -> Result<MixAdvice, MixRefusal> {
        let client = self
            .0
            .upgrade()
            .and_then(|catalog| catalog.audiomuse().ok())
            .ok_or(MixRefusal::NotSetUp)?;

        let (plan, from_score, to_score) = tokio::join!(
            transition_plan(&client, from, to),
            client.score(from),
            client.score(to),
        );
        let tempos = tempo(from_score).zip(tempo(to_score));
        Ok(MixAdvice {
            plan: plan?,
            tempos,
        })
    }
}

/// The transition AudioMuse-AI plans within the songs' fade points; none when a song has no points or no room.
async fn transition_plan(
    client: &AudioMuse,
    from: &str,
    to: &str,
) -> Result<Option<server::audiomuse::TransitionPlan>, MixRefusal> {
    let ids = [from.to_string(), to.to_string()];
    let points = match client.smart_fade_points(&ids).await {
        Err(AudioMuseError::NotOffered) => return Err(MixRefusal::NotOffered),
        Err(error) => {
            tracing::debug!(%error, "no fade points for the next mix");
            return Ok(None);
        }
        Ok(points) => points,
    };
    let point = |id: &str| points.iter().find(|point| point.id == id);
    let (Some(out), Some(r#in)) = (
        point(from).and_then(|point| point.fade_out),
        point(to).and_then(|point| point.fade_in),
    ) else {
        return Ok(None);
    };

    match client.transition(from, to, out, r#in).await {
        Ok(plan) => Ok(Some(plan)),
        Err(AudioMuseError::NotOffered) => Err(MixRefusal::NotOffered),
        Err(error) => {
            tracing::debug!(%error, "no transition for the next mix");
            Ok(None)
        }
    }
}

fn tempo(score: Result<Option<Score>, AudioMuseError>) -> Option<f32> {
    match score {
        Ok(score) => score?.tempo,
        Err(error) => {
            tracing::debug!(%error, "no tempo for the next mix");
            None
        }
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

    #[test]
    fn every_audiomuse_failure_has_a_reason_to_say_it_in_the_viewers_language() {
        for error in [
            AudioMuseError::Rejected,
            AudioMuseError::Unreachable,
            AudioMuseError::BadUrl,
            AudioMuseError::Status(500),
            AudioMuseError::Malformed,
            AudioMuseError::NotOffered,
            AudioMuseError::NoTransition,
        ] {
            assert!(audiomuse_error(error).reason.is_some());
        }
    }

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
