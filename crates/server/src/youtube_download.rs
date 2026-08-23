//! Native YouTube Music search and stream resolution for the download tool.
//!
//! This deliberately reuses the same InnerTube, authentication, BotGuard, and
//! decipher paths as playback. The UI never shells out to a site downloader or
//! handles YouTube credentials itself.

use config::{AppConfig, MusicService};
use reader::Track;
use serde_json::Value;

use crate::ytmusic::{
    YtStreamInfo, clients::WEB_REMIX, innertube, search::music_search_tracks,
    search::synthesize_album_id,
};

/// A YouTube video id: exactly 11 characters of the URL-safe alphabet.
fn is_video_id(candidate: &str) -> bool {
    candidate.len() == 11
        && candidate
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The video id inside anything a user might paste, or `None` if there isn't one.
///
/// Covers the shapes people actually have on their clipboard: `watch?v=`, the
/// `youtu.be` short form, `shorts/`, `embed/`, `live/`, with or without a
/// scheme, and a bare id. Playlist and timestamp parameters are ignored rather
/// than rejected, since a link copied mid-playback always carries them.
pub fn parse_video_id(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if is_video_id(input) {
        return Some(input.to_string());
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
    if host == "youtu.be" {
        return path
            .split('/')
            .next()
            .filter(|id| is_video_id(id))
            .map(str::to_string);
    }

    for prefix in ["shorts/", "embed/", "live/", "v/"] {
        if let Some(tail) = path.strip_prefix(prefix) {
            return tail
                .split('/')
                .next()
                .filter(|id| is_video_id(id))
                .map(str::to_string);
        }
    }

    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, value)| *key == "v" && is_video_id(value))
        .map(|(_, value)| value.to_string())
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
    author
        .trim()
        .strip_suffix(" - Topic")
        .unwrap_or(author.trim())
        .to_string()
}

fn playability_reason(response: &Value) -> String {
    response
        .pointer("/playabilityStatus/reason")
        .and_then(Value::as_str)
        .or_else(|| {
            response
                .pointer("/playabilityStatus/status")
                .and_then(Value::as_str)
        })
        .unwrap_or("no metadata in the response")
        .to_string()
}

#[derive(Clone, Default)]
pub struct YoutubeDownloadClient {
    cookies: Option<String>,
}

impl YoutubeDownloadClient {
    /// Use the active YouTube Music session when one is available. The native
    /// endpoints also support anonymous search and public-track downloads, so
    /// this client remains useful while the local library is active.
    pub fn from_config(config: &AppConfig) -> Self {
        let cookies = config
            .server
            .as_ref()
            .filter(|server| server.service == MusicService::YtMusic)
            .and_then(|server| server.access_token.as_deref())
            .filter(|token| !token.is_empty())
            .map(str::to_owned);
        Self { cookies }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Track>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        music_search_tracks(query, self.cookies.as_deref()).await
    }

    /// The track behind a pasted link, for downloading one specific video
    /// instead of picking from search results. Metadata comes from the same
    /// `/player` response playback uses, so a link and a search hit produce the
    /// same [`Track`] shape and go down the same download path.
    pub async fn track_from_link(&self, link: &str) -> Result<Track, String> {
        let video_id =
            parse_video_id(link).ok_or_else(|| "not a YouTube track link".to_string())?;
        let response = innertube::player(
            WEB_REMIX,
            &video_id,
            self.cookies.as_deref(),
            innertube::PlayerExtras::default(),
        )
        .await?;
        let details = response
            .get("videoDetails")
            .filter(|details| details.get("title").is_some())
            .ok_or_else(|| playability_reason(&response))?;

        let title = details
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let artist = details
            .get("author")
            .and_then(Value::as_str)
            .map(clean_author)
            .unwrap_or_default();
        let duration = details
            .get("lengthSeconds")
            .and_then(Value::as_str)
            .and_then(|seconds| seconds.parse().ok())
            .unwrap_or(0);

        Ok(Track {
            id: crate::ytmusic::yt_id(video_id),
            cover: best_thumbnail(details),
            album_id: synthesize_album_id("", &artist),
            title,
            artist: artist.clone(),
            album: String::new(),
            duration,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: if artist.is_empty() {
                Vec::new()
            } else {
                vec![artist]
            },
        })
    }

    pub async fn resolve_stream(&self, video_id: &str) -> Result<YtStreamInfo, String> {
        let video_id = video_id.trim();
        if video_id.is_empty() {
            return Err("track has no YouTube video id".to_string());
        }
        crate::ytmusic::probe_stream(video_id, self.cookies.as_deref()).await
    }
}

#[cfg(test)]
mod tests {
    use super::{clean_author, parse_video_id};

    #[test]
    fn reads_every_link_shape() {
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
                parse_video_id(link).as_deref(),
                Some("dQw4w9WgXcQ"),
                "failed on {link}"
            );
        }
    }

    /// A plain search phrase must never be mistaken for a link, or typing a
    /// song name would silently start downloading something else.
    #[test]
    fn rejects_anything_that_is_not_a_link() {
        for input in [
            "",
            "never gonna give you up",
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://soundcloud.com/artist/track",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/@channel",
        ] {
            assert_eq!(parse_video_id(input), None, "failed on {input:?}");
        }
    }

    #[test]
    fn strips_the_topic_suffix() {
        assert_eq!(clean_author("Boards of Canada - Topic"), "Boards of Canada");
        assert_eq!(clean_author("  Aphex Twin  "), "Aphex Twin");
    }
}
