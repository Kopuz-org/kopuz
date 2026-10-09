//! Downloading YouTube and YouTube Music audio without an external downloader.
//!
//! A link resolves through the same InnerTube client the YouTube Music source
//! browses with, and audio comes from the same stream resolver playback uses,
//! so a download works whenever playback does, signed in or anonymous.

use std::path::Path;
use std::time::{Duration, Instant};

use reader::Track;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::ytmusic::{
    YtStreamInfo, clients::WEB_REMIX, discover, innertube, playlists, search::synthesize_album_id,
};

/// What a pasted link points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Video(String),
    Playlist(String),
    /// A YouTube Music album page (`browse/MPREb_…`).
    Album(String),
}

impl Link {
    /// The canonical address of one video, for history rows and file tags.
    pub fn video_url(video_id: &str) -> String {
        format!("https://music.youtube.com/watch?v={video_id}")
    }
}

/// A YouTube video id: exactly 11 characters of the URL-safe alphabet.
fn is_video_id(candidate: &str) -> bool {
    candidate.len() == 11 && is_id_charset(candidate)
}

fn is_id_charset(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// What `input` links to, or `None` when it is not a YouTube link at all.
///
/// Covers what people have on their clipboard: `watch?v=`, `youtu.be`,
/// `shorts/`, `embed/`, `live/`, `playlist?list=`, a YouTube Music album page,
/// with or without a scheme, and a bare video id. A watch link carrying a
/// `list=` names the song that was playing, so the video wins over the list.
pub fn parse_link(input: &str) -> Option<Link> {
    let input = input.trim();
    if is_video_id(input) {
        return Some(Link::Video(input.to_string()));
    }

    let without_scheme = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
        .unwrap_or(input);
    let (host, rest) = without_scheme
        .split_once('/')
        .unwrap_or((without_scheme, ""));
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    let is_youtube = matches!(
        host.as_str(),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" | "youtu.be"
    );
    if !is_youtube {
        return None;
    }

    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path = path.trim_end_matches('/');
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.split('#').next().unwrap_or(value))
    };
    let first_segment = |tail: &str| tail.split('/').next().map(str::to_string);

    if host == "youtu.be" {
        return first_segment(path)
            .filter(|id| is_video_id(id))
            .map(Link::Video);
    }
    for prefix in ["shorts/", "embed/", "live/", "v/"] {
        if let Some(tail) = path.strip_prefix(prefix) {
            return first_segment(tail)
                .filter(|id| is_video_id(id))
                .map(Link::Video);
        }
    }
    if let Some(tail) = path.strip_prefix("browse/") {
        return first_segment(tail)
            .filter(|id| id.starts_with("MPREb_") && is_id_charset(id))
            .map(Link::Album);
    }
    if let Some(id) = param("v").filter(|id| is_video_id(id)) {
        return Some(Link::Video(id.to_string()));
    }
    param("list")
        .filter(|id| is_id_charset(id))
        .map(|id| Link::Playlist(id.to_string()))
}

/// Largest thumbnail in a `videoDetails.thumbnail.thumbnails` array. YouTube
/// orders them smallest first.
fn best_thumbnail(details: &Value) -> Option<String> {
    details
        .pointer("/thumbnail/thumbnails")?
        .as_array()?
        .iter()
        .filter_map(|entry| entry.get("url")?.as_str())
        .next_back()
        .map(str::to_string)
}

/// YouTube's auto-generated artist channels are named "<artist> - Topic".
fn clean_author(author: &str) -> String {
    let author = author.trim();
    author
        .strip_suffix(" - Topic")
        .unwrap_or(author)
        .to_string()
}

fn playability_reason(response: &Value) -> String {
    response
        .pointer("/playabilityStatus/reason")
        .or_else(|| response.pointer("/playabilityStatus/status"))
        .and_then(Value::as_str)
        .unwrap_or("no metadata in the response")
        .to_string()
}

/// Resolves links and streams with the signed-in YouTube Music session when
/// there is one, anonymously otherwise.
#[derive(Clone, Default)]
pub struct YoutubeDownloader {
    cookies: Option<String>,
}

impl YoutubeDownloader {
    pub fn new(cookies: Option<String>) -> Self {
        Self {
            cookies: cookies.filter(|cookies| !cookies.is_empty()),
        }
    }

    /// Songs on YouTube Music matching `query`, best match first.
    pub async fn search(&self, query: &str) -> Result<Vec<Track>, String> {
        crate::ytmusic::search::music_search_tracks(query, self.cookies.as_deref()).await
    }

    /// The tracks a link stands for, in order.
    pub async fn tracks(&self, link: &Link) -> Result<Vec<Track>, String> {
        let cookies = self.cookies.as_deref().unwrap_or("");
        let tracks = match link {
            Link::Video(id) => vec![self.video_track(id).await?],
            Link::Playlist(id) => playlists::get_playlist_entries(id, cookies).await?,
            Link::Album(id) => discover::fetch_album_tracks(id, cookies).await?,
        };
        if tracks.is_empty() {
            return Err("the link has no tracks".to_string());
        }
        Ok(tracks)
    }

    /// One video's metadata, from the same `/player` response playback reads,
    /// so a single link and a playlist entry come out as the same [`Track`].
    async fn video_track(&self, video_id: &str) -> Result<Track, String> {
        let response = innertube::player(
            WEB_REMIX,
            video_id,
            self.cookies.as_deref(),
            innertube::PlayerExtras::default(),
        )
        .await?;
        let details = response
            .get("videoDetails")
            .filter(|details| details.get("title").is_some())
            .ok_or_else(|| playability_reason(&response))?;
        let text = |key: &str| details.get(key).and_then(Value::as_str);
        let artist = text("author").map(clean_author).unwrap_or_default();

        Ok(Track {
            id: crate::ytmusic::yt_id(video_id),
            cover: best_thumbnail(details),
            album_id: synthesize_album_id("", &artist),
            title: text("title").unwrap_or_default().to_string(),
            artists: if artist.is_empty() {
                Vec::new()
            } else {
                vec![artist.clone()]
            },
            artist,
            album: String::new(),
            duration: text("lengthSeconds")
                .and_then(|seconds| seconds.parse().ok())
                .unwrap_or(0),
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            replay_gain: config::ReplayGainInfo::default(),
            credits: Vec::new(),
            explicit: false,
            plays: None,
        })
    }

    pub async fn stream(&self, video_id: &str) -> Result<YtStreamInfo, String> {
        crate::ytmusic::probe_stream(video_id, self.cookies.as_deref()).await
    }
}

/// Bytes written so far, and the size when the server said.
pub type Progress<'a> = &'a mut (dyn FnMut(u64, Option<u64>) + Send);

/// Write `stream` to `path`, stopping early once `cancelled` says so.
///
/// A range-safe URL with a known size is fetched in fixed slices with retries:
/// googlevideo throttles one long-running response to playback speed, while
/// separate range requests come back at full speed.
pub async fn fetch_stream(
    stream: &YtStreamInfo,
    path: &Path,
    cancelled: &(dyn Fn() -> bool + Send + Sync),
    progress: Progress<'_>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("the download client could not start: {error}"))?;
    let file = tokio::fs::File::create(path)
        .await
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    let mut writer = tokio::io::BufWriter::with_capacity(256 * 1024, file);

    match stream.content_length.filter(|total| *total > 0) {
        Some(total) if stream.range_safe => {
            fetch_ranges(&client, stream, total, &mut writer, cancelled, progress).await?
        }
        _ => fetch_sequential(&client, stream, &mut writer, cancelled, progress).await?,
    }
    writer
        .flush()
        .await
        .map_err(|error| format!("cannot write the audio: {error}"))
}

const CANCELLED: &str = "download cancelled";

async fn fetch_ranges(
    client: &reqwest::Client,
    stream: &YtStreamInfo,
    total: u64,
    writer: &mut (impl AsyncWriteExt + Unpin),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
    progress: Progress<'_>,
) -> Result<(), String> {
    const RANGE_SIZE: u64 = 512 * 1024;
    const RANGE_TIMEOUT: Duration = Duration::from_secs(45);
    const ATTEMPTS: u64 = 4;

    let mut start = 0;
    while start < total {
        let end = (start + RANGE_SIZE - 1).min(total - 1);
        let expected = end - start + 1;
        let mut last_error = String::new();
        let mut received = None;
        for attempt in 1..=ATTEMPTS {
            if cancelled() {
                return Err(CANCELLED.to_string());
            }
            let request = async {
                let response = client
                    .get(&stream.url)
                    .header(reqwest::header::USER_AGENT, &stream.user_agent)
                    .header(reqwest::header::ACCEPT_ENCODING, "identity")
                    .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(|error| error.without_url().to_string())?;
                if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                    return Err(format!("HTTP {}", response.status()));
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| error.without_url().to_string())?;
                if bytes.len() as u64 != expected {
                    return Err(format!("{} of {expected} bytes", bytes.len()));
                }
                Ok(bytes)
            };
            match tokio::time::timeout(RANGE_TIMEOUT, request).await {
                Ok(Ok(bytes)) => {
                    received = Some(bytes);
                    break;
                }
                Ok(Err(error)) => last_error = error,
                Err(_) => last_error = "timed out".to_string(),
            }
            tracing::debug!(start, end, attempt, error = %last_error, "retrying an audio range");
            tokio::time::sleep(Duration::from_millis(250 * attempt)).await;
        }
        let bytes = received
            .ok_or_else(|| format!("bytes {start}-{end} failed {ATTEMPTS} times: {last_error}"))?;
        writer
            .write_all(&bytes)
            .await
            .map_err(|error| format!("cannot write the audio: {error}"))?;
        start = end + 1;
        progress(start, Some(total));
    }
    Ok(())
}

async fn fetch_sequential(
    client: &reqwest::Client,
    stream: &YtStreamInfo,
    writer: &mut (impl AsyncWriteExt + Unpin),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
    progress: Progress<'_>,
) -> Result<(), String> {
    const STALL: Duration = Duration::from_secs(60);

    let mut response = client
        .get(&stream.url)
        .header(reqwest::header::USER_AGENT, &stream.user_agent)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let total = stream.content_length.or_else(|| response.content_length());
    let mut written = 0u64;
    let mut reported = Instant::now();
    loop {
        if cancelled() {
            return Err(CANCELLED.to_string());
        }
        let chunk = tokio::time::timeout(STALL, response.chunk())
            .await
            .map_err(|_| "the download stalled".to_string())?
            .map_err(|error| error.without_url().to_string())?;
        let Some(chunk) = chunk else { break };
        writer
            .write_all(&chunk)
            .await
            .map_err(|error| format!("cannot write the audio: {error}"))?;
        written += chunk.len() as u64;
        if reported.elapsed() >= Duration::from_millis(200) {
            progress(written, total);
            reported = Instant::now();
        }
    }
    if let Some(total) = total.filter(|total| written != *total) {
        return Err(format!("the stream ended at {written} of {total} bytes"));
    }
    progress(written, total);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_video_link_shape() {
        for link in [
            "dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDAMVM123&start_radio=1",
            "http://youtube.com/watch?app=desktop&v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?t=42",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "  music.youtube.com/watch?v=dQw4w9WgXcQ  ",
        ] {
            assert_eq!(
                parse_link(link),
                Some(Link::Video("dQw4w9WgXcQ".into())),
                "failed on {link}"
            );
        }
    }

    #[test]
    fn reads_playlists_and_albums() {
        assert_eq!(
            parse_link("https://music.youtube.com/playlist?list=OLAK5uy_abc-123"),
            Some(Link::Playlist("OLAK5uy_abc-123".into()))
        );
        assert_eq!(
            parse_link("https://www.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG"),
            Some(Link::Playlist("PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG".into()))
        );
        assert_eq!(
            parse_link("https://music.youtube.com/browse/MPREb_4pL8gzRtw1p"),
            Some(Link::Album("MPREb_4pL8gzRtw1p".into()))
        );
    }

    /// A song name must never read as a link, or typing one would download
    /// something else.
    #[test]
    fn rejects_anything_that_is_not_a_youtube_link() {
        for input in [
            "",
            "never gonna give you up",
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://soundcloud.com/artist/track",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/@channel",
            "https://music.youtube.com/browse/UCabc",
        ] {
            assert_eq!(parse_link(input), None, "failed on {input:?}");
        }
    }

    #[test]
    fn strips_the_topic_suffix() {
        assert_eq!(clean_author("Boards of Canada - Topic"), "Boards of Canada");
        assert_eq!(clean_author("  Aphex Twin  "), "Aphex Twin");
    }
}
