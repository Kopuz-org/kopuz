use serde::Deserialize;

use super::LyricLine;
use super::{
    Lyrics, SERVER_LYRICS_TIMEOUT, has_usable_line_timing, lrc::parse_lrc, lrc_has_usable_timing,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct JellyfinLyricsResponse {
    #[serde(default)]
    lyrics: Vec<JellyfinLyricLine>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct JellyfinLyricLine {
    text: String,
    start: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SubsonicSongLyricsEnvelope {
    #[serde(rename = "subsonic-response")]
    response: SubsonicSongLyricsResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubsonicSongLyricsResponse {
    status: String,
    #[serde(default)]
    lyrics_list: Option<SubsonicLyricsList>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubsonicLyricsList {
    #[serde(default)]
    structured_lyrics: Vec<SubsonicStructuredLyric>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubsonicStructuredLyric {
    synced: bool,
    #[serde(default)]
    line: Vec<SubsonicLyricLine>,
}

#[derive(Debug, Deserialize)]
struct SubsonicLyricLine {
    #[serde(default)]
    start: Option<i64>,
    value: String,
}

#[derive(Debug, Deserialize)]
struct SubsonicPlainLyricsEnvelope {
    #[serde(rename = "subsonic-response")]
    response: SubsonicPlainLyricsResponse,
}

#[derive(Debug, Deserialize)]
struct SubsonicPlainLyricsResponse {
    status: String,
    #[serde(default)]
    lyrics: Option<SubsonicPlainLyricsData>,
}

#[derive(Debug, Deserialize)]
struct SubsonicPlainLyricsData {
    #[serde(default)]
    value: String,
}

pub(super) async fn fetch_jellyfin_lyrics(
    item_id: &str,
    server_url: &str,
    token: &str,
) -> Option<Lyrics> {
    let url = format!(
        "{}/Items/{}/Lyrics",
        server_url.trim_end_matches('/'),
        item_id
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("X-Emby-Token", token)
        .timeout(SERVER_LYRICS_TIMEOUT)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: JellyfinLyricsResponse = resp.json().await.ok()?;
    if data.lyrics.is_empty() {
        return None;
    }

    let has_timestamps = data.lyrics.iter().any(|l| l.start.is_some());
    if has_timestamps {
        let mut lines: Vec<LyricLine> = data
            .lyrics
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .filter_map(|l| {
                l.start.map(|ticks| LyricLine {
                    start_time: ticks as f64 / 10_000_000.0,
                    end_time: None,
                    text: l.text.clone(),
                    chunks: Vec::new(),
                    parent_line_index: None,
                    background: false,
                    opposite_turn: false,
                })
            })
            .collect();
        lines.sort_by(|a, b| {
            a.start_time
                .partial_cmp(&b.start_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if has_usable_line_timing(&lines) {
            return Some(Lyrics::Synced(lines));
        }
    }

    let text: String = data
        .lyrics
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.trim().is_empty() {
        Some(Lyrics::Plain(text))
    } else {
        None
    }
}

pub(super) async fn fetch_subsonic_lyrics(
    song_id: &str,
    server_url: &str,
    username: &str,
    password: &str,
    artist: &str,
    title: &str,
) -> Option<Lyrics> {
    let base_url = server_url.trim_end_matches('/');
    let client = reqwest::Client::new();
    let salt = "kopuzlyrics";
    let token = format!("{:x}", md5::compute(format!("{}{}", password, salt)));

    if let Some(lyrics) =
        subsonic_get_by_id(&client, base_url, username, &token, salt, song_id).await
    {
        return Some(lyrics);
    }

    subsonic_get_by_title(&client, base_url, username, &token, salt, artist, title).await
}

async fn subsonic_get_by_id(
    client: &reqwest::Client,
    base_url: &str,
    username: &str,
    token: &str,
    salt: &str,
    song_id: &str,
) -> Option<Lyrics> {
    let url = format!("{}/rest/getLyricsBySongId.view", base_url);
    let params = [
        ("u", username),
        ("t", token),
        ("s", salt),
        ("v", "1.16.1"),
        ("c", "kopuz"),
        ("f", "json"),
        ("id", song_id),
    ];

    let resp = client
        .get(&url)
        .query(&params)
        .timeout(SERVER_LYRICS_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let data: SubsonicSongLyricsEnvelope = resp.json().await.ok()?;
    if !data.response.status.eq_ignore_ascii_case("ok") {
        return None;
    }

    let structured = data
        .response
        .lyrics_list?
        .structured_lyrics
        .into_iter()
        .next()?;

    if structured.synced {
        let mut lines: Vec<LyricLine> = structured
            .line
            .iter()
            .filter(|l| !l.value.trim().is_empty())
            .filter_map(|l| {
                l.start.map(|ms| LyricLine {
                    start_time: ms as f64 / 1000.0,
                    end_time: None,
                    text: l.value.clone(),
                    chunks: Vec::new(),
                    parent_line_index: None,
                    background: false,
                    opposite_turn: false,
                })
            })
            .collect();
        lines.sort_by(|a, b| {
            a.start_time
                .partial_cmp(&b.start_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if has_usable_line_timing(&lines) {
            return Some(Lyrics::Synced(lines));
        }
    }

    let text: String = structured
        .line
        .iter()
        .map(|l| l.value.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.trim().is_empty() {
        Some(Lyrics::Plain(text))
    } else {
        None
    }
}

async fn subsonic_get_by_title(
    client: &reqwest::Client,
    base_url: &str,
    username: &str,
    token: &str,
    salt: &str,
    artist: &str,
    title: &str,
) -> Option<Lyrics> {
    let url = format!("{}/rest/getLyrics.view", base_url);
    let params = [
        ("u", username),
        ("t", token),
        ("s", salt),
        ("v", "1.16.1"),
        ("c", "kopuz"),
        ("f", "json"),
        ("artist", artist),
        ("title", title),
    ];

    let resp = client
        .get(&url)
        .query(&params)
        .timeout(SERVER_LYRICS_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let data: SubsonicPlainLyricsEnvelope = resp.json().await.ok()?;
    if !data.response.status.eq_ignore_ascii_case("ok") {
        return None;
    }

    let value = data.response.lyrics?.value;
    if value.trim().is_empty() {
        return None;
    }

    let parsed = if lrc_has_usable_timing(&value) {
        parse_lrc(&value)
    } else {
        Vec::new()
    };
    if has_usable_line_timing(&parsed) {
        Some(Lyrics::Synced(parsed))
    } else {
        Some(Lyrics::Plain(value))
    }
}
