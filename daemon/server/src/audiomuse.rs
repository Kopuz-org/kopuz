//! AudioMuse-AI, the sonic analysis server beside a Navidrome library. Its song ids are the library's own track keys.

use std::time::Duration;

use serde::Deserialize;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const QUICK_TIMEOUT: Duration = Duration::from_secs(15);
const FINGERPRINT_TIMEOUT: Duration = Duration::from_secs(45);
/// The first text search loads the model, which takes tens of seconds.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(90);

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
