//! AudioMuse-AI, the sonic analysis server beside a Navidrome library. Its song ids are the library's own track keys.

use std::time::Duration;

use serde::Deserialize;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const QUICK_TIMEOUT: Duration = Duration::from_secs(15);
const FINGERPRINT_TIMEOUT: Duration = Duration::from_secs(45);
/// The first text search loads the model, which takes tens of seconds.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(90);
/// The transition endpoint downloads and analyses both songs before it answers.
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(90);
const BAD_REQUEST: u16 = 400;
const NOT_FOUND: u16 = 404;
const UNAVAILABLE: u16 = 503;

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
    /// 404: the instance is older than the API (before v3.7.4); 503: "Search by Recording" is disabled.
    #[error("this AudioMuse-AI does not offer the call")]
    NotOffered,
    /// 400: the two songs leave no room for a transition.
    #[error("AudioMuse-AI found no room for a transition")]
    NoTransition,
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
    tempo: Option<f32>,
    energy: Option<f32>,
}

/// What AudioMuse heard in one song: genre and style tags, and the six moods, each strongest first as it sent them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    pub tags: Vec<(String, f32)>,
    pub moods: Vec<(String, f32)>,
    /// Beats per minute.
    pub tempo: Option<f32>,
    pub energy: Option<f32>,
}

#[derive(Deserialize)]
struct RawFade {
    item_id: String,
    fade_in_seconds: Option<f64>,
    fade_out_seconds: Option<f64>,
    duration_seconds: Option<f64>,
}

#[derive(Deserialize)]
struct FadeAnswer {
    results: Vec<RawFade>,
}

/// Where a song can fade in and out, in seconds; None for a song without a neural fingerprint.
#[derive(Debug, Clone, PartialEq)]
pub struct FadePoints {
    pub id: String,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
    pub duration: Option<f64>,
}

/// How AudioMuse mixes one song into the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    BeatMix,
    BeatRise,
    /// Also what a kind this client does not know falls back to.
    Crossfade,
}

/// Crossover frequencies of the three EQ bands, in Hz.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BandFilters {
    pub high_hz: f64,
    pub mid_hz: f64,
    pub mid_q: f64,
    pub bass_hz: f64,
}

/// Automation curves, one value per `step` from the plan's `start`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Curves {
    /// Playback rate, 1.0 is unchanged.
    pub speed: Vec<f64>,
    /// Linear gain, 0.0 to 1.0.
    pub volume: Vec<f64>,
    pub high_db: Vec<f64>,
    pub mid_db: Vec<f64>,
    pub bass_db: Vec<f64>,
}

/// The song that plays out.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OutgoingDeck {
    /// Level correction AudioMuse suggests. Kopuz ignores it: ReplayGain already levels songs.
    #[serde(default)]
    pub gain: Option<f64>,
    #[serde(flatten)]
    pub curves: Curves,
}

/// The song that comes in.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IncomingDeck {
    /// Level correction AudioMuse suggests. Kopuz ignores it: ReplayGain already levels songs.
    #[serde(default)]
    pub gain: Option<f64>,
    /// Where in the song it starts, in seconds.
    pub start: f64,
    #[serde(flatten)]
    pub curves: Curves,
    /// Cutoff curve in Hz; None for no high-pass.
    #[serde(default)]
    pub highpass_hz: Option<Vec<f64>>,
    /// Where in the song it carries on once the transition ends, in seconds.
    #[serde(default)]
    pub continue_from: Option<f64>,
}

/// A transition between two songs; all times in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionPlan {
    pub kind: TransitionKind,
    /// Where in the outgoing song it begins.
    pub start: f64,
    pub duration: f64,
    pub middle: f64,
    /// Time between two curve samples; always above zero.
    pub step: f64,
    pub filters: BandFilters,
    pub from: OutgoingDeck,
    pub to: IncomingDeck,
}

#[derive(Deserialize)]
struct RawTransition {
    #[serde(rename = "type")]
    kind: String,
    start: f64,
    duration: f64,
    middle: f64,
    step: f64,
    filters: BandFilters,
    from: OutgoingDeck,
    to: IncomingDeck,
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
        tempo: raw.tempo,
        energy: raw.energy,
    })
}

fn fade_points(body: &[u8]) -> Result<Vec<FadePoints>, AudioMuseError> {
    let found: FadeAnswer = serde_json::from_slice(body).map_err(|_| AudioMuseError::Malformed)?;
    Ok(found
        .results
        .into_iter()
        .map(|row| FadePoints {
            id: row.item_id,
            fade_in: row.fade_in_seconds,
            fade_out: row.fade_out_seconds,
            duration: row.duration_seconds,
        })
        .collect())
}

/// The lengths of a deck's curves; they must all match and not be empty.
fn deck_curves(curves: &Curves, highpass: Option<&Vec<f64>>) -> Vec<usize> {
    let mut lengths = vec![
        curves.speed.len(),
        curves.volume.len(),
        curves.high_db.len(),
        curves.mid_db.len(),
        curves.bass_db.len(),
    ];
    lengths.extend(highpass.map(Vec::len));
    lengths
}

fn parse_transition(body: &[u8]) -> Result<TransitionPlan, AudioMuseError> {
    let raw: RawTransition = serde_json::from_slice(body).map_err(|_| AudioMuseError::Malformed)?;
    let decks = [
        deck_curves(&raw.from.curves, None),
        deck_curves(&raw.to.curves, raw.to.highpass_hz.as_ref()),
    ];
    let aligned = decks
        .iter()
        .all(|lengths| lengths[0] > 0 && lengths.iter().all(|length| *length == lengths[0]));
    if !(raw.step > 0.0 && raw.step.is_finite() && aligned) {
        return Err(AudioMuseError::Malformed);
    }

    let kind = match raw.kind.as_str() {
        "beat_mix" => TransitionKind::BeatMix,
        "beat_rise" => TransitionKind::BeatRise,
        _ => TransitionKind::Crossfade,
    };
    Ok(TransitionPlan {
        kind,
        start: raw.start,
        duration: raw.duration,
        middle: raw.middle,
        step: raw.step,
        filters: raw.filters,
        from: raw.from,
        to: raw.to,
    })
}

/// 404 and 503 from the recording-search API mean the instance does not offer it.
fn not_offered(answer: Result<Vec<u8>, AudioMuseError>) -> Result<Vec<u8>, AudioMuseError> {
    match answer {
        Err(AudioMuseError::Status(NOT_FOUND | UNAVAILABLE)) => Err(AudioMuseError::NotOffered),
        answer => answer,
    }
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

    /// Fade-in and fade-out points for songs, in the order of `ids`.
    pub async fn smart_fade_points(
        &self,
        ids: &[String],
    ) -> Result<Vec<FadePoints>, AudioMuseError> {
        let request = self
            .client
            .post(format!("{}/api/recording_search/smart_fade", self.base))
            .json(&serde_json::json!({ "item_ids": ids, "keep_order": true }));
        fade_points(&not_offered(self.send(request).await)?)
    }

    /// How to mix `from_id` into `to_id`, given the seconds the outgoing song may use (`out`) and
    /// the incoming one may use (`r#in`) for it.
    pub async fn transition(
        &self,
        from_id: &str,
        to_id: &str,
        out: f64,
        r#in: f64,
    ) -> Result<TransitionPlan, AudioMuseError> {
        let request = self
            .client
            .get(format!("{}/api/recording_search/transition", self.base))
            .timeout(TRANSITION_TIMEOUT)
            .query(&[("from_id", from_id), ("to_id", to_id)])
            .query(&[("out", out), ("in", r#in)]);
        match not_offered(self.send(request).await) {
            Err(AudioMuseError::Status(BAD_REQUEST)) => Err(AudioMuseError::NoTransition),
            answer => parse_transition(&answer?),
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

    const PLAN: &str = r#"{"type":"beat_mix","start":190.2,"duration":15.4,"middle":7.7,"step":0.05,
        "filters":{"high_hz":4000,"mid_hz":1000,"mid_q":0.7,"bass_hz":200},
        "from":{"gain":0.8,"speed":[1.0,1.0],"volume":[1.0,0.5],"high_db":[0,0],"mid_db":[0,0],"bass_db":[0,-3],"new":1},
        "to":{"gain":1.1,"start":30.1,"speed":[1.0,1.0],"volume":[0.0,0.5],"high_db":[0,0],"mid_db":[0,0],"bass_db":[0,0],"highpass_hz":null,"continue_from":45.5},
        "info":{"tempo_from":124.0,"confidence":"high","later":{"x":1}}}"#;

    #[test]
    fn a_score_carries_tempo_and_energy_when_sent() {
        let body =
            br#"{"item_id":"a","tempo":125.0,"energy":0.2006,"key":"D#","mood_vector":"a:0.1"}"#;
        let score = parse_score(body).unwrap();
        assert_eq!(score.tempo, Some(125.0));
        assert_eq!(score.energy, Some(0.2006));

        let nulls = parse_score(br#"{"tempo":null,"energy":null,"key":null}"#).unwrap();
        assert_eq!((nulls.tempo, nulls.energy), (None, None));
    }

    #[test]
    fn fade_points_keep_order_and_allow_nulls() {
        let body = br#"{"count":2,"results":[
            {"item_id":"a","fade_in_seconds":12.5,"fade_out_seconds":201.3,"duration_seconds":215.0,"extra":1},
            {"item_id":"b","fade_in_seconds":null,"fade_out_seconds":null,"duration_seconds":null}]}"#;
        assert_eq!(
            fade_points(body).unwrap(),
            [
                FadePoints {
                    id: "a".into(),
                    fade_in: Some(12.5),
                    fade_out: Some(201.3),
                    duration: Some(215.0),
                },
                FadePoints {
                    id: "b".into(),
                    fade_in: None,
                    fade_out: None,
                    duration: None,
                },
            ]
        );
        assert!(matches!(fade_points(b"[]"), Err(AudioMuseError::Malformed)));
    }

    #[test]
    fn a_transition_plan_is_read_and_unknown_fields_are_ignored() {
        let plan = parse_transition(PLAN.as_bytes()).unwrap();
        assert_eq!(plan.kind, TransitionKind::BeatMix);
        assert_eq!(
            (plan.start, plan.duration, plan.middle, plan.step),
            (190.2, 15.4, 7.7, 0.05)
        );
        assert_eq!(plan.filters.bass_hz, 200.0);
        assert_eq!(plan.from.gain, Some(0.8));
        assert_eq!(plan.from.curves.bass_db, [0.0, -3.0]);
        assert_eq!(plan.to.start, 30.1);
        assert_eq!(plan.to.curves.volume, [0.0, 0.5]);
        assert_eq!(plan.to.highpass_hz, None);
        assert_eq!(plan.to.continue_from, Some(45.5));
    }

    #[test]
    fn a_transition_kind_maps_and_an_unknown_one_is_a_crossfade() {
        let kind = |name: &str| {
            let body = PLAN.replace("beat_mix", name);
            parse_transition(body.as_bytes()).unwrap().kind
        };
        assert_eq!(kind("beat_rise"), TransitionKind::BeatRise);
        assert_eq!(kind("crossfade"), TransitionKind::Crossfade);
        assert_eq!(kind("something_new"), TransitionKind::Crossfade);
    }

    #[test]
    fn a_highpass_curve_and_absent_optionals_are_read() {
        let body = PLAN
            .replace(r#""highpass_hz":null"#, r#""highpass_hz":[100.0,200.0]"#)
            .replace(r#""gain":1.1,"#, "")
            .replace(r#","continue_from":45.5"#, "");
        let plan = parse_transition(body.as_bytes()).unwrap();
        assert_eq!(plan.to.highpass_hz, Some(vec![100.0, 200.0]));
        assert_eq!((plan.to.gain, plan.to.continue_from), (None, None));
    }

    #[test]
    fn a_transition_with_bad_curves_or_step_is_malformed() {
        let malformed = |body: String| {
            assert!(matches!(
                parse_transition(body.as_bytes()),
                Err(AudioMuseError::Malformed)
            ));
        };
        malformed(PLAN.replace(
            r#""speed":[1.0,1.0],"volume":[1.0,0.5]"#,
            r#""speed":[1.0],"volume":[1.0,0.5]"#,
        ));
        malformed(PLAN.replace(r#""volume":[0.0,0.5]"#, r#""volume":[]"#));
        malformed(PLAN.replace(r#""highpass_hz":null"#, r#""highpass_hz":[1.0]"#));
        malformed(PLAN.replace(r#""step":0.05"#, r#""step":0"#));
        malformed(PLAN.replace(r#""step":0.05"#, r#""step":-1.0"#));
        malformed(PLAN.replace(r#""middle":7.7,"#, ""));
        malformed("<html></html>".into());
    }

    /// A real v3.7.4 answer; its `info` carries fields this client does not model.
    #[test]
    fn a_real_transition_answer_is_read() {
        let body = include_bytes!("../tests/fixtures/audiomuse-transition.json");
        let plan = parse_transition(body).unwrap();
        assert_eq!(plan.kind, TransitionKind::Crossfade);
        assert!((plan.step - 0.1).abs() < 0.001);
        assert_eq!(plan.from.curves.speed.len(), 103);
        assert_eq!(plan.to.curves.bass_db.len(), 103);
        assert_eq!(plan.to.highpass_hz, None);
        assert!(plan.to.continue_from.is_some());
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
