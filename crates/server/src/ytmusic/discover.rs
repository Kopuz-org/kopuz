//! YouTube Music Home feed parser. The wire format was reverse-engineered
//! against a live response — see `yttools/discover-home-probe` and
//! `discover-continuation-probe` for the recordings. Three shelves come
//! back per page; the section-list-level continuation token feeds the
//! next three.

use std::path::PathBuf;

use reader::models::Track;
use serde_json::{Value, json};

use super::SOURCE_PREFIX;
use super::clients::{ORIGIN_YOUTUBE_MUSIC, WEB_REMIX};
use super::innertube::{http_client, sapisid_hash};
use super::search::{encode_url_tag, synthesize_album_id};

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoverHome {
    pub shelves: Vec<DiscoverShelf>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoverShelf {
    pub title: String,
    pub strapline: Option<String>,
    pub more_browse_id: Option<String>,
    pub items: Vec<DiscoverItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiscoverItem {
    Song(Track),
    Playlist {
        playlist_id: String,
        title: String,
        subtitle: String,
        thumbnail: Option<String>,
    },
    Album {
        browse_id: String,
        title: String,
        subtitle: String,
        thumbnail: Option<String>,
    },
    Artist {
        channel_id: String,
        name: String,
        thumbnail: Option<String>,
    },
    Mood {
        browse_id: String,
        title: String,
        thumbnail: Option<String>,
    },
}

pub async fn fetch_home(cookies: &str) -> Result<DiscoverHome, String> {
    let body = build_browse_body(Some("FEmusic_home"));
    let resp = post(
        &format!("{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/browse?prettyPrint=false"),
        &body,
        cookies,
    )
    .await?;
    Ok(parse_initial(&resp))
}

pub async fn fetch_continuation(token: &str, cookies: &str) -> Result<DiscoverHome, String> {
    let body = build_browse_body(None);
    let url = format!(
        "{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/browse?ctoken={token}&continuation={token}&prettyPrint=false"
    );
    let resp = post(&url, &body, cookies).await?;
    Ok(parse_continuation(&resp))
}

fn build_browse_body(browse_id: Option<&str>) -> Value {
    let client = WEB_REMIX;
    let mut body = json!({
        "context": {
            "client": {
                "clientName": client.client_name,
                "clientVersion": client.client_version,
                "hl": "en",
                "gl": "US",
                "userAgent": client.user_agent,
            },
            "user": { "lockedSafetyMode": false },
        },
    });
    if let Some(id) = browse_id {
        body["browseId"] = Value::String(id.to_string());
    }
    body
}

async fn post(url: &str, body: &Value, cookies: &str) -> Result<Value, String> {
    let client = WEB_REMIX;
    let auth = sapisid_hash(cookies, ORIGIN_YOUTUBE_MUSIC)
        .ok_or_else(|| "SAPISID missing".to_string())?;
    http_client()
        .post(url)
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"))
        .header("Cookie", cookies)
        .header("Authorization", auth)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("discover HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("discover HTTP: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("discover JSON: {e}"))
}

fn parse_initial(resp: &Value) -> DiscoverHome {
    let contents = resp
        .pointer(
            "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
        )
        .and_then(|v| v.as_array());
    let continuation = resp
        .pointer(
            "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/continuations/0/nextContinuationData/continuation",
        )
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    DiscoverHome {
        shelves: contents
            .map(|arr| arr.iter().filter_map(parse_shelf).collect())
            .unwrap_or_default(),
        continuation,
    }
}

fn parse_continuation(resp: &Value) -> DiscoverHome {
    let contents = resp
        .pointer("/continuationContents/sectionListContinuation/contents")
        .and_then(|v| v.as_array());
    let continuation = resp
        .pointer(
            "/continuationContents/sectionListContinuation/continuations/0/nextContinuationData/continuation",
        )
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    DiscoverHome {
        shelves: contents
            .map(|arr| arr.iter().filter_map(parse_shelf).collect())
            .unwrap_or_default(),
        continuation,
    }
}

fn parse_shelf(section: &Value) -> Option<DiscoverShelf> {
    let shelf = section.get("musicCarouselShelfRenderer")?;
    let header = shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer");
    let title = header
        .and_then(|h| h.pointer("/title/runs/0/text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if title.is_empty() {
        return None;
    }
    let strapline = header
        .and_then(|h| h.pointer("/strapline/runs/0/text"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let more_browse_id = header
        .and_then(|h| {
            h.pointer(
                "/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint/browseId",
            )
        })
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let items: Vec<DiscoverItem> = shelf
        .get("contents")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_tile).collect())
        .unwrap_or_default();

    if items.is_empty() {
        return None;
    }

    Some(DiscoverShelf {
        title,
        strapline,
        more_browse_id,
        items,
    })
}

fn parse_tile(item: &Value) -> Option<DiscoverItem> {
    let r = item.get("musicTwoRowItemRenderer")?;
    let title = r
        .pointer("/title/runs/0/text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if title.is_empty() {
        return None;
    }
    let subtitle = r
        .pointer("/subtitle/runs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let thumbnail = best_thumbnail(r).map(normalize_yt_thumbnail);

    if let Some(video_id) = r
        .pointer("/navigationEndpoint/watchEndpoint/videoId")
        .and_then(|v| v.as_str())
    {
        return Some(DiscoverItem::Song(build_song_track(
            video_id,
            &title,
            &subtitle,
            thumbnail.as_deref(),
        )));
    }

    if let Some(playlist_id) = r
        .pointer("/navigationEndpoint/watchPlaylistEndpoint/playlistId")
        .and_then(|v| v.as_str())
    {
        return Some(DiscoverItem::Playlist {
            playlist_id: playlist_id.to_string(),
            title,
            subtitle,
            thumbnail,
        });
    }

    if let Some(browse_id) = r
        .pointer("/navigationEndpoint/browseEndpoint/browseId")
        .and_then(|v| v.as_str())
    {
        if let Some(rest) = browse_id.strip_prefix("VL") {
            return Some(DiscoverItem::Playlist {
                playlist_id: rest.to_string(),
                title,
                subtitle,
                thumbnail,
            });
        }
        if browse_id.starts_with("MPRE") {
            return Some(DiscoverItem::Album {
                browse_id: browse_id.to_string(),
                title,
                subtitle,
                thumbnail,
            });
        }
        if browse_id.starts_with("UC") {
            return Some(DiscoverItem::Artist {
                channel_id: browse_id.to_string(),
                name: title,
                thumbnail,
            });
        }
        if browse_id.starts_with("FEmusic_") {
            return Some(DiscoverItem::Mood {
                browse_id: browse_id.to_string(),
                title,
                thumbnail,
            });
        }
    }

    None
}

fn build_song_track(
    video_id: &str,
    title: &str,
    subtitle: &str,
    thumbnail: Option<&str>,
) -> Track {
    // Subtitle for songs/videos is typically "Artist • N views" — take
    // the first run as the primary artist; everything after the first
    // dot is metadata that doesn't belong in the artist field.
    let primary_artist = subtitle
        .split('•')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let artists = if primary_artist.is_empty() {
        Vec::new()
    } else {
        vec![primary_artist.clone()]
    };
    let path = match thumbnail {
        Some(url) if !url.is_empty() => PathBuf::from(format!(
            "{SOURCE_PREFIX}:{video_id}:{}",
            encode_url_tag(url)
        )),
        _ => PathBuf::from(format!("{SOURCE_PREFIX}:{video_id}")),
    };
    let album_id = synthesize_album_id("", &primary_artist);
    Track {
        path,
        album_id,
        title: title.to_string(),
        artist: primary_artist,
        album: String::new(),
        duration: 0,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists,
    }
}

fn best_thumbnail(r: &Value) -> Option<String> {
    r.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0))
        })
        .and_then(|t| t.get("url").and_then(|u| u.as_str()))
        .map(|s| s.to_string())
}

fn normalize_yt_thumbnail(url: String) -> String {
    // Photo-CDN URLs end with =wNNN-hNNN-... and accept rewriting to a
    // bigger size. Mix-art URLs (music.youtube.com/image/mixart?r=…)
    // and any other token-style URL can't take that suffix; appending
    // it breaks the request.
    if let Some(idx) = url.rfind("=w")
        && url[idx + 2..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    {
        return format!("{}=w544-h544-l90-rj", &url[..idx]);
    }
    url
}
