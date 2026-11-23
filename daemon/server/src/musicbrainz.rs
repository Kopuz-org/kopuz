//! MusicBrainz lookups: the page a track has there, found from what its
//! tags name. Daemon-side, because finding one leaves the machine.

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::time::Duration;

/// A shareable MusicBrainz page for a track: its release when the tags name
/// one, else the recording a search resolves to.
pub async fn track_page_url(release_id: Option<&str>, artist: &str, title: &str) -> Option<String> {
    if let Some(id) = release_id {
        let id = id.trim();
        if !id.is_empty() {
            return Some(format!("https://musicbrainz.org/release/{id}"));
        }
    }

    let recording_id = search_recording_id(artist, title).await?;
    Some(format!("https://musicbrainz.org/recording/{recording_id}"))
}

/// The Lucene query naming one recording.
fn recording_query(artist: &str, title: &str) -> String {
    let mut query = format!("recording:\"{}\"", escape_lucene(title.trim()));
    let artist = artist.trim();
    if !artist.is_empty() {
        query.push_str(&format!(" AND artist:\"{}\"", escape_lucene(artist)));
    }
    query
}

fn escape_lucene(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

pub async fn search_recording_id(artist: &str, title: &str) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }

    let url = format!(
        "https://musicbrainz.org/ws/2/recording?query={}&fmt=json&limit=1",
        utf8_percent_encode(&recording_query(artist, title), NON_ALPHANUMERIC)
    );

    let json = get_json(&url).await.ok()?;
    json.get("recordings")?
        .as_array()?
        .first()?
        .get("id")?
        .as_str()
        .map(|s| s.to_string())
}

const USER_AGENT: &str = concat!(
    "Kopuz/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/owlenz/kopuz)"
);

const SIMILAR_ALGORITHM: &str =
    "session_based_days_7500_session_300_contribution_5_threshold_10_limit_100_filter_True_skip_30";

async fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let client = utils::http::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    response.json().await.map_err(|error| error.to_string())
}

async fn artist_id(name: &str) -> Result<Option<String>, String> {
    let query = format!("artist:\"{}\"", escape_lucene(name.trim()));
    let url = format!(
        "https://musicbrainz.org/ws/2/artist/?query={}&fmt=json&limit=1",
        utf8_percent_encode(&query, NON_ALPHANUMERIC)
    );
    let json = get_json(&url).await?;
    Ok(json
        .get("artists")
        .and_then(|artists| artists.as_array()?.first().cloned())
        .filter(|artist| artist.get("score").and_then(|score| score.as_u64()) >= Some(90))
        .and_then(|artist| artist.get("id")?.as_str().map(str::to_string)))
}

pub async fn similar_artists(name: &str, limit: usize) -> Result<Vec<String>, String> {
    let Some(id) = artist_id(name).await? else {
        return Ok(Vec::new());
    };
    let url = format!(
        "https://labs.api.listenbrainz.org/similar-artists/json?artist_mbids={id}&algorithm={SIMILAR_ALGORITHM}"
    );
    let json = get_json(&url).await?;
    Ok(similar_names(&json, limit))
}

fn similar_names(json: &serde_json::Value, limit: usize) -> Vec<String> {
    json.as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| artist.get("name")?.as_str())
        .map(str::to_string)
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similar_artists_are_read_in_order_up_to_the_limit() {
        let json = serde_json::json!([
            {"artist_mbid": "a", "name": "Nirvana", "score": 8788},
            {"artist_mbid": "b", "name": "System of a Down", "score": 7783},
            {"artist_mbid": "c", "score": 7000},
            {"artist_mbid": "d", "name": "Red Hot Chili Peppers", "score": 7752}
        ]);
        assert_eq!(similar_names(&json, 2), ["Nirvana", "System of a Down"]);
        assert_eq!(
            similar_names(&serde_json::json!({}), 5),
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    #[ignore = "asks MusicBrainz and ListenBrainz; run by hand"]
    async fn live_similar_artists() {
        let found = similar_artists("Metallica", 5).await.expect("answer");
        assert_eq!(found.len(), 5, "{found:?}");
    }
}
