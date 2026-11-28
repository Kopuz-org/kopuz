//! AudioMuse-AI, the sonic analysis server beside a Navidrome library. Its song ids are the library's own track keys.

use std::time::Duration;

use serde::Deserialize;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const QUICK_TIMEOUT: Duration = Duration::from_secs(15);
const FINGERPRINT_TIMEOUT: Duration = Duration::from_secs(45);
/// The first text search loads the model, which takes tens of seconds.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(90);
const NOT_FOUND: u16 = 404;

/// Messages never carry the address or the token.
#[derive(Debug, thiserror::Error)]
pub enum AudioMuseError {
    #[error("the AudioMuse-AI address is not a http or https URL")]
    BadUrl,
    #[error("AudioMuse-AI could not be reached")]
    Unreachable,
    #[error("AudioMuse-AI did not accept the token")]
    Rejected,
    #[error("AudioMuse-AI answered with status {0}")]
    Status(u16),
    #[error("AudioMuse-AI sent an answer that could not be read")]
    Malformed,
}

#[derive(Clone)]
pub struct AudioMuse {
    client: reqwest::Client,
    base: String,
    token: String,
}

#[derive(Deserialize)]
struct Hit {
    item_id: String,
}

#[derive(Deserialize)]
struct Described {
    results: Vec<Hit>,
}

#[derive(Deserialize)]
struct SimilarTrack {
    item_id: String,
    distance: f32,
}

#[derive(Deserialize)]
struct ArtistHit {
    artist: String,
    /// None for a credit such as "A feat. B", which no library artist can match.
    artist_id: Option<String>,
    divergence: f32,
    track_count: Option<u32>,
}

/// An artist close in sound to another one.
#[derive(Debug, Clone, PartialEq)]
pub struct SimilarArtist {
    pub id: String,
    pub name: String,
    /// Smaller is closer.
    pub divergence: f32,
    /// How many of the artist's songs AudioMuse-AI analysed; None from a version that does not say.
    pub tracks: Option<u32>,
}

#[derive(Deserialize)]
struct RawScore {
    mood_vector: Option<String>,
    other_features: Option<String>,
}

/// What AudioMuse heard in one song: genre and style tags, and the six moods, each strongest first as it sent them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    pub tags: Vec<(String, f32)>,
    pub moods: Vec<(String, f32)>,
}

#[derive(Deserialize)]
struct Version {
    app_version: String,
}

/// The song ids of a text search answer, in AudioMuse's order.
fn described_ids(body: &[u8]) -> Result<Vec<String>, AudioMuseError> {
    let found: Described = serde_json::from_slice(body).map_err(|_| AudioMuseError::Malformed)?;
    Ok(found.results.into_iter().map(|hit| hit.item_id).collect())
}

fn fingerprint_ids(body: &[u8]) -> Result<Vec<String>, AudioMuseError> {
    let found: Vec<Hit> = serde_json::from_slice(body).map_err(|_| AudioMuseError::Malformed)?;
    Ok(found.into_iter().map(|hit| hit.item_id).collect())
}

/// `"metal:0.647,heavy metal:0.542"` as label and value pairs; a piece that is not one is skipped.
fn labelled(text: &str) -> Vec<(String, f32)> {
    text.split(',')
        .filter_map(|piece| {
            let (label, value) = piece.rsplit_once(':')?;
            let label = label.trim();
            if label.is_empty() {
                return None;
            }
            Some((label.to_string(), value.trim().parse().ok()?))
        })
        .collect()
}

fn similar_track_ids(body: &[u8]) -> Result<Vec<(String, f32)>, AudioMuseError> {
    let found: Vec<SimilarTrack> =
        serde_json::from_slice(body).map_err(|_| AudioMuseError::Malformed)?;
    Ok(found
        .into_iter()
        .map(|hit| (hit.item_id, hit.distance))
        .collect())
}

fn similar_artist_rows(body: &[u8]) -> Result<Vec<SimilarArtist>, AudioMuseError> {
    let found: Vec<ArtistHit> =
        serde_json::from_slice(body).map_err(|_| AudioMuseError::Malformed)?;
    Ok(found
        .into_iter()
        .filter_map(|hit| {
            Some(SimilarArtist {
                id: hit.artist_id?,
                name: hit.artist,
                divergence: hit.divergence,
                tracks: hit.track_count,
            })
        })
        .collect())
}

fn parse_score(body: &[u8]) -> Result<Score, AudioMuseError> {
    let raw: RawScore = serde_json::from_slice(body).map_err(|_| AudioMuseError::Malformed)?;
    Ok(Score {
        tags: labelled(raw.mood_vector.as_deref().unwrap_or_default()),
        moods: labelled(raw.other_features.as_deref().unwrap_or_default()),
    })
}

impl AudioMuse {
    pub fn new(url: &str, token: &str) -> Result<Self, AudioMuseError> {
        let url = reqwest::Url::parse(url.trim()).map_err(|_| AudioMuseError::BadUrl)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(AudioMuseError::BadUrl);
        }
        let client = utils::http::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(QUICK_TIMEOUT)
            .build()
            .map_err(|_| AudioMuseError::Unreachable)?;
        Ok(Self {
            client,
            base: url.as_str().trim_end_matches('/').to_string(),
            token: token.trim().to_string(),
        })
    }

    /// Sends the request and returns the body of a successful answer.
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Vec<u8>, AudioMuseError> {
        let response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|error| {
                tracing::debug!(error = %error.without_url(), "audiomuse request failed");
                AudioMuseError::Unreachable
            })?;
        let status = response.status();
        if matches!(status.as_u16(), 401 | 403) {
            return Err(AudioMuseError::Rejected);
        }
        if !status.is_success() {
            return Err(AudioMuseError::Status(status.as_u16()));
        }
        let body = response
            .bytes()
            .await
            .map_err(|_| AudioMuseError::Unreachable)?;
        Ok(body.to_vec())
    }

    pub async fn health(&self) -> Result<(), AudioMuseError> {
        self.send(self.client.get(format!("{}/api/health", self.base)))
            .await
            .map(drop)
    }

    pub async fn version(&self) -> Result<String, AudioMuseError> {
        let body = self
            .send(self.client.get(format!("{}/api/version", self.base)))
            .await?;
        let version: Version =
            serde_json::from_slice(&body).map_err(|_| AudioMuseError::Malformed)?;
        Ok(version.app_version)
    }

    /// Song ids whose sound matches a free-text description.
    pub async fn describe(&self, query: &str, limit: usize) -> Result<Vec<String>, AudioMuseError> {
        let request = self
            .client
            .post(format!("{}/api/clap/search", self.base))
            .timeout(DESCRIBE_TIMEOUT)
            .json(&serde_json::json!({ "query": query, "limit": limit }));
        described_ids(&self.send(request).await?)
    }

    /// Song ids that fit the listening profile AudioMuse holds for its configured user.
    pub async fn fingerprint(&self, count: usize) -> Result<Vec<String>, AudioMuseError> {
        let request = self
            .client
            .get(format!("{}/api/sonic_fingerprint/generate", self.base))
            .timeout(FINGERPRINT_TIMEOUT)
            .query(&[("n", count)]);
        fingerprint_ids(&self.send(request).await?)
    }

    /// The `count` songs closest in sound to one song, with their distance (smaller is closer); none for a song it has not analysed.
    pub async fn similar_tracks(
        &self,
        id: &str,
        count: usize,
    ) -> Result<Vec<(String, f32)>, AudioMuseError> {
        let request = self
            .client
            .get(format!("{}/api/similar_tracks", self.base))
            .query(&[("item_id", id)])
            .query(&[("n", count)]);
        match self.send(request).await {
            Err(AudioMuseError::Status(NOT_FOUND)) => Ok(Vec::new()),
            answer => similar_track_ids(&answer?),
        }
    }

    /// The `count` artists closest in sound to one artist.
    pub async fn similar_artists(
        &self,
        artist_id: &str,
        count: usize,
    ) -> Result<Vec<SimilarArtist>, AudioMuseError> {
        let request = self
            .client
            .get(format!("{}/api/similar_artists", self.base))
            .query(&[("artist_id", artist_id)])
            .query(&[("n", count)]);
        match self.send(request).await {
            Err(AudioMuseError::Status(NOT_FOUND)) => Ok(Vec::new()),
            answer => similar_artist_rows(&answer?),
        }
    }

    /// `None` for a song it has not analysed.
    pub async fn score(&self, id: &str) -> Result<Option<Score>, AudioMuseError> {
        let request = self
            .client
            .get(format!("{}/external/get_score", self.base))
            .query(&[("id", id)]);
        match self.send(request).await {
            Err(AudioMuseError::Status(NOT_FOUND)) => Ok(None),
            answer => parse_score(&answer?).map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_search_answer_gives_its_ids_in_order() {
        let body = br#"{"count":2,"query":"calm piano","results":[
            {"item_id":"b2","title":"Two","author":"Ada","album":"X","similarity":0.9,"mood_vector":"a:0.1","other_features":"f"},
            {"item_id":"a1","title":"One","author":"Bo","album":"Y","similarity":0.8}]}"#;
        assert_eq!(described_ids(body).unwrap(), ["b2", "a1"]);
    }

    #[test]
    fn a_fingerprint_answer_gives_its_ids_in_order() {
        let body = br#"[{"item_id":"s9","title":"T","distance":0.1,"top_genre":"jazz"},{"item_id":"s3","distance":0.2}]"#;
        assert_eq!(fingerprint_ids(body).unwrap(), ["s9", "s3"]);
    }

    #[test]
    fn labelled_values_keep_their_order_and_skip_what_is_not_one() {
        assert_eq!(
            labelled("metal:0.647, heavy metal:0.542,broken,:0.3,x:nan?"),
            [
                ("metal".to_string(), 0.647),
                ("heavy metal".to_string(), 0.542)
            ]
        );
        assert!(labelled("").is_empty());
    }

    #[test]
    fn a_score_splits_into_tags_and_moods() {
        let body = br#"{"item_id":"a","mood_vector":"metal:0.6,rock:0.4","other_features":"danceable:0.59,sad:0.56","tempo":null,"key":null}"#;
        let score = parse_score(body).unwrap();
        assert_eq!(
            score.tags,
            [("metal".to_string(), 0.6), ("rock".to_string(), 0.4)]
        );
        assert_eq!(
            score.moods,
            [("danceable".to_string(), 0.59), ("sad".to_string(), 0.56)]
        );

        let bare = parse_score(br#"{"item_id":"a","mood_vector":null}"#).unwrap();
        assert_eq!(bare, Score::default());
    }

    #[test]
    fn similar_answers_give_ids_with_their_distance() {
        let tracks = br#"[{"item_id":"t1","title":"T","distance":0.12,"mood_vector":"a:1"},{"item_id":"t2","distance":0.3}]"#;
        assert_eq!(
            similar_track_ids(tracks).unwrap(),
            [("t1".to_string(), 0.12), ("t2".to_string(), 0.3)]
        );
        let artists =
            br#"[{"artist":"Boris","artist_id":"ar-2","divergence":0.5,"track_count":9}]"#;
        let boris = SimilarArtist {
            id: "ar-2".into(),
            name: "Boris".into(),
            divergence: 0.5,
            tracks: Some(9),
        };
        assert_eq!(
            similar_artist_rows(artists).unwrap(),
            std::slice::from_ref(&boris)
        );
        // A credit such as "A feat. B" has no artist id of its own.
        let credited = br#"[{"artist":"A feat. B","artist_id":null,"divergence":0.1,"track_count":1},{"artist":"Boris","artist_id":"ar-2","divergence":0.5}]"#;
        assert_eq!(
            similar_artist_rows(credited).unwrap(),
            [SimilarArtist {
                tracks: None,
                ..boris
            }]
        );
        assert!(matches!(
            similar_track_ids(b"<html></html>"),
            Err(AudioMuseError::Malformed)
        ));
    }

    #[test]
    fn an_answer_of_another_shape_is_malformed() {
        assert!(matches!(
            described_ids(b"<html>not found</html>"),
            Err(AudioMuseError::Malformed)
        ));
        assert!(matches!(
            fingerprint_ids(br#"{"results":[]}"#),
            Err(AudioMuseError::Malformed)
        ));
    }

    #[test]
    fn only_an_http_address_is_taken() {
        assert!(AudioMuse::new("http://audiomuse.example:8000/", "t").is_ok());
        assert!(matches!(
            AudioMuse::new("ftp://audiomuse.example", "t"),
            Err(AudioMuseError::BadUrl)
        ));
        assert!(matches!(
            AudioMuse::new("nonsense", "t"),
            Err(AudioMuseError::BadUrl)
        ));
    }
}
