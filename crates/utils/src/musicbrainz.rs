use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::time::Duration;

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

async fn search_recording_id(artist: &str, title: &str) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }

    let mut query = format!("recording:\"{}\"", escape_lucene(title));
    let artist = artist.trim();
    if !artist.is_empty() {
        query.push_str(&format!(" AND artist:\"{}\"", escape_lucene(artist)));
    }

    let url = format!(
        "https://musicbrainz.org/ws/2/recording?query={}&fmt=json&limit=1",
        utf8_percent_encode(&query, NON_ALPHANUMERIC)
    );

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let res = client
        .get(&url)
        .header(
            "User-Agent",
            concat!(
                "Kopuz/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/owlenz/kopuz)"
            ),
        )
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        return None;
    }

    let body = res.text().await.ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    json.get("recordings")?
        .as_array()?
        .first()?
        .get("id")?
        .as_str()
        .map(|s| s.to_string())
}

fn escape_lucene(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}
