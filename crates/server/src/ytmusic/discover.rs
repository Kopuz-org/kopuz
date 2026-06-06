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

/// Verified against /tmp/yt-album-MPREb_*.json via yttools/album-probe.
///
/// YT InnerTube returns polymorphic arrays where each entry is keyed by
/// which renderer it is (e.g. `{musicResponsiveHeaderRenderer: {...}}`
/// vs `{musicShelfRenderer: {...}}`). Positional indexing into those
/// arrays breaks the moment YT reorders, so every lookup here iterates
/// and dispatches on the renderer key.
///
/// Album browse shape:
///   /contents/twoColumnBrowseResultsRenderer/tabs[i]/tabRenderer
///     /content/sectionListRenderer/contents[j]/musicResponsiveHeaderRenderer
///   …holds title, straplineTextOne (artist + UC… browseEndpoint),
///   subtitle (kind + year), thumbnail, and buttons[] containing a
///   musicPlayButtonRenderer with the album's OLAK5uy_… audio playlist.
///
///   /contents/singleColumnBrowseResultsRenderer/tabs[i]/tabRenderer
///     /content/sectionListRenderer/contents[j]/musicShelfRenderer/contents
///   …each entry is a `musicResponsiveListItemRenderer` with flexColumns:
///     [0] title + watchEndpoint{videoId, playlistId}
///     [1] empty (text: {}) for single-artist albums — fall back to the
///         strapline artist; the per-row column doesn't exist
///     [2] play-count label, never artist
///   plus fixedColumns[0] = "mm:ss" duration and index.runs[0] = track #.
///
/// Track rows carry no thumbnail of their own, so we stamp the header
/// cover onto every track for jellyfin_image to pick up.
pub struct YtAlbum {
    pub browse_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<String>,
    pub thumbnail: Option<String>,
    pub audio_playlist_id: Option<String>,
    pub tracks: Vec<Track>,
}

pub async fn fetch_album_tracks(browse_id: &str, cookies: &str) -> Result<Vec<Track>, String> {
    fetch_album(browse_id, cookies).await.map(|a| a.tracks)
}

pub async fn fetch_album(browse_id: &str, cookies: &str) -> Result<YtAlbum, String> {
    let body = build_browse_body(Some(browse_id));
    let resp = post(
        &format!("{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/browse?prettyPrint=false"),
        &body,
        cookies,
    )
    .await?;
    Ok(parse_album(browse_id, &resp))
}

fn parse_album(browse_id: &str, resp: &Value) -> YtAlbum {
    let sections = album_section_contents(resp);
    let header = find_album_header(resp, &sections);

    let title = header
        .and_then(|h| h.pointer("/title/runs/0/text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let artist = pick_album_artist(header);
    let year = pick_album_year(header);
    let thumbnail = best_album_thumbnail(header).map(normalize_yt_thumbnail);
    let audio_playlist_id_header = header.and_then(find_audio_playlist_id);

    let mut tracks = Vec::new();
    let mut audio_pid_from_rows: Option<String> = None;
    for section in &sections {
        let Some(items) = section
            .get("musicShelfRenderer")
            .and_then(|s| s.get("contents"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        for item in items {
            let Some(row) = item.get("musicResponsiveListItemRenderer") else {
                continue;
            };
            if audio_pid_from_rows.is_none()
                && let Some(pid) = row
                    .pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/watchEndpoint/playlistId")
                    .and_then(|v| v.as_str())
            {
                audio_pid_from_rows = Some(pid.to_string());
            }
            if let Some(track) =
                parse_album_row(row, &title, artist.as_deref(), thumbnail.as_deref())
            {
                tracks.push(track);
            }
        }
    }

    YtAlbum {
        browse_id: browse_id.to_string(),
        title,
        artist,
        year,
        thumbnail,
        audio_playlist_id: audio_playlist_id_header.or(audio_pid_from_rows),
        tracks,
    }
}

/// Every `sectionListRenderer.contents` array reachable from the album
/// response — iterates `tabs[]` looking for `tabRenderer` rather than
/// indexing positionally, and merges in `secondaryContents` (where the
/// track shelf actually lives in the new two-column layout).
fn album_section_contents(resp: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    for tab_root in [
        resp.pointer("/contents/twoColumnBrowseResultsRenderer/tabs"),
        resp.pointer("/contents/singleColumnBrowseResultsRenderer/tabs"),
    ]
    .into_iter()
    .flatten()
    .filter_map(|v| v.as_array())
    {
        for tab in tab_root {
            let Some(contents) = tab
                .get("tabRenderer")
                .and_then(|t| t.get("content"))
                .and_then(|c| c.get("sectionListRenderer"))
                .and_then(|s| s.get("contents"))
                .and_then(|v| v.as_array())
            else {
                continue;
            };
            out.extend(contents.iter());
        }
    }
    if let Some(sec) = resp
        .pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents")
        .and_then(|v| v.as_array())
    {
        out.extend(sec.iter());
    }
    out
}

fn find_album_header<'a>(resp: &'a Value, sections: &[&'a Value]) -> Option<&'a Value> {
    for section in sections {
        if let Some(h) = section.get("musicResponsiveHeaderRenderer") {
            return Some(h);
        }
        if let Some(h) = section.get("musicDetailHeaderRenderer") {
            return Some(h);
        }
    }
    // Legacy layout puts the header object at the response root.
    if let Some(header_obj) = resp.pointer("/header").and_then(|v| v.as_object()) {
        for (key, value) in header_obj {
            if key.ends_with("HeaderRenderer") {
                return Some(value);
            }
        }
    }
    None
}

fn pick_album_artist(header: Option<&Value>) -> Option<String> {
    let header = header?;
    // New layout splits these: straplineTextOne is the artist (with a
    // UC… browseEndpoint), subtitle is "<Kind> • <Year>" with no artist.
    let from_strapline = header
        .pointer("/straplineTextOne/runs")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                .map(|s| s.trim())
                .find(|s| !s.is_empty() && *s != "•")
                .map(|s| s.to_string())
        });
    if from_strapline.is_some() {
        return from_strapline;
    }
    // Legacy layout crammed "<Kind> • <Artist> • <Year>" into subtitle.
    let arr = header.pointer("/subtitle/runs").and_then(|v| v.as_array())?;
    for r in arr {
        let text = r.get("text").and_then(|v| v.as_str())?;
        let t = text.trim();
        if t.is_empty() || t == "•" {
            continue;
        }
        if t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if matches!(
            t,
            "Album" | "Single" | "EP" | "Song" | "Video" | "Audio" | "Playlist"
        ) {
            continue;
        }
        return Some(t.to_string());
    }
    None
}

fn pick_album_year(header: Option<&Value>) -> Option<String> {
    let header = header?;
    for ptr in ["/subtitle/runs", "/secondSubtitle/runs"] {
        if let Some(arr) = header.pointer(ptr).and_then(|v| v.as_array()) {
            for r in arr {
                if let Some(t) = r.get("text").and_then(|v| v.as_str()) {
                    let t = t.trim();
                    if t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

fn find_audio_playlist_id(header: &Value) -> Option<String> {
    let buttons = header.get("buttons").and_then(|v| v.as_array())?;
    for button in buttons {
        if let Some(pid) = button
            .get("musicPlayButtonRenderer")
            .and_then(|p| p.pointer("/playNavigationEndpoint/watchEndpoint/playlistId"))
            .and_then(|v| v.as_str())
        {
            return Some(pid.to_string());
        }
    }
    None
}

fn best_album_thumbnail(header: Option<&Value>) -> Option<String> {
    let header = header?;
    for ptr in [
        "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/croppedSquareThumbnailRenderer/thumbnail/thumbnails",
    ] {
        if let Some(arr) = header.pointer(ptr).and_then(|v| v.as_array()) {
            let best = arr
                .iter()
                .max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0))
                .and_then(|t| t.get("url").and_then(|u| u.as_str()))
                .map(|s| s.to_string());
            if best.is_some() {
                return best;
            }
        }
    }
    None
}

fn parse_album_row(
    row: &Value,
    album_title: &str,
    album_artist: Option<&str>,
    album_thumbnail: Option<&str>,
) -> Option<Track> {
    let video_id = row
        .pointer("/playlistItemData/videoId")
        .and_then(|v| v.as_str())?
        .to_string();
    let title = row
        .pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if title.is_empty() {
        return None;
    }
    // flexColumns[1] is empty (`text: {}` — no runs at all) for
    // single-artist album rows in the new layout. For multi-artist or
    // features-credited tracks it does carry runs. Prefer it when set,
    // otherwise fall back to the header artist.
    let row_artist = row
        .pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let primary_artist = row_artist
        .or_else(|| album_artist.map(|s| s.to_string()))
        .unwrap_or_default();
    let duration = row
        .pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text/runs/0/text")
        .and_then(|v| v.as_str())
        .and_then(parse_mm_ss)
        .unwrap_or(0);
    let track_number = row
        .pointer("/index/runs/0/text")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u32>().ok());

    let artists = if primary_artist.is_empty() {
        Vec::new()
    } else {
        vec![primary_artist.clone()]
    };
    let path = match album_thumbnail {
        Some(url) if !url.is_empty() => PathBuf::from(format!(
            "{SOURCE_PREFIX}:{video_id}:{}",
            encode_url_tag(url)
        )),
        _ => PathBuf::from(format!("{SOURCE_PREFIX}:{video_id}")),
    };
    let album_id = synthesize_album_id(album_title, &primary_artist);
    Some(Track {
        path,
        album_id,
        title,
        artist: primary_artist,
        album: album_title.to_string(),
        duration,
        khz: 0,
        bitrate: 0,
        track_number,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists,
    })
}

fn parse_mm_ss(s: &str) -> Option<u64> {
    let mut parts = s.split(':').rev();
    let secs: u64 = parts.next()?.parse().ok()?;
    let mins: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let hours: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some(hours * 3600 + mins * 60 + secs)
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
