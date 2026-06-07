use std::path::PathBuf;

use reader::models::Track;
use serde_json::{Value, json};

use super::SOURCE_PREFIX;
use super::clients::WEB_REMIX;
use super::innertube::sapisid_hash;
use super::search::{encode_url_tag, synthesize_album_id};

const ORIGIN: &str = "https://music.youtube.com";

pub async fn start_mix(seed_video_id: &str, cookies: &str) -> Result<Vec<Track>, String> {
    let playlist_id = format!("RDAMVM{seed_video_id}");
    let client = WEB_REMIX;
    let body = json!({
        "enablePersistentPlaylistPanel": true,
        "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
        "videoId": seed_video_id,
        "playlistId": playlist_id,
        "params": "wAEB",
        "isAudioOnly": true,
        "context": {
            "client": {
                "clientName": client.client_name,
                "clientVersion": client.client_version,
                "hl": "en",
                "gl": "US",
            },
            "user": { "lockedSafetyMode": false },
        },
    });

    // Mix endpoint works without auth (anonymous radio for any public
    // video). Skip Cookie + SAPISID when cookies is empty so anon
    // YT mode can still hit Start-Radio.
    let cookies_opt = if cookies.is_empty() { None } else { Some(cookies) };
    let mut req = super::innertube::http_client().clone()
        .post(format!("{ORIGIN}/youtubei/v1/next?prettyPrint=false"))
        .header("Content-Type", "application/json")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("Origin", ORIGIN)
        .header("Referer", format!("{ORIGIN}/"));
    if let Some(c) = cookies_opt {
        let auth = sapisid_hash(c, ORIGIN)
            .ok_or_else(|| "SAPISID missing".to_string())?;
        req = req.header("Cookie", c).header("Authorization", auth);
    }
    let resp: Value = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("next HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("next HTTP: {e}"))?
        .json()
        .await
        .map_err(|e| format!("next JSON: {e}"))?;

    Ok(walk_queue(&resp))
}

fn walk_queue(resp: &Value) -> Vec<Track> {
    // Iterate the watchNext tabs by tabRenderer presence rather than
    // assuming the queue lives at tabs[0]. YT A/B-tests the tab order
    // (Up next vs Lyrics vs Related) and the positional dive
    // silently returned an empty queue whenever the queue tab wasn't
    // first — kills 'next song' and the radio button.
    let tabs = resp
        .pointer(
            "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs",
        )
        .and_then(|v| v.as_array());
    let Some(tabs) = tabs else {
        return Vec::new();
    };
    let items = tabs.iter().find_map(|tab| {
        tab.get("tabRenderer")
            .and_then(|t| t.get("content"))
            .and_then(|c| c.get("musicQueueRenderer"))
            .and_then(|q| q.get("content"))
            .and_then(|c| c.get("playlistPanelRenderer"))
            .and_then(|p| p.get("contents"))
            .and_then(|v| v.as_array())
    });
    let Some(items) = items else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for item in items {
        let row = item
            .get("playlistPanelVideoRenderer")
            .or_else(|| item.pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer"));
        let Some(row) = row else {
            continue;
        };
        if let Some(track) = parse_queue_row(row) {
            out.push(track);
        }
    }
    out
}

fn parse_queue_row(row: &Value) -> Option<Track> {
    let video_id = row.get("videoId").and_then(|v| v.as_str())?.to_string();
    let mvt = row
        .pointer("/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
        .and_then(|v| v.as_str());
    if !matches!(
        mvt,
        Some("MUSIC_VIDEO_TYPE_ATV" | "MUSIC_VIDEO_TYPE_OMV" | "MUSIC_VIDEO_TYPE_UGC" | "MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC")
    ) {
        return None;
    }
    let has_album = matches!(
        mvt,
        Some("MUSIC_VIDEO_TYPE_ATV" | "MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC")
    );

    let title = row
        .pointer("/title/runs/0/text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let byline: Vec<String> = row
        .pointer("/longBylineText/runs")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                .filter(|s| !matches!(*s, " • " | " & " | ", "))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    // For songs (has_album): byline = [artist, album, year-or-views, likes]
    // For videos:            byline = [artist, views, likes]
    let primary_artist = byline.first().cloned().unwrap_or_default();
    let artists = if primary_artist.is_empty() {
        Vec::new()
    } else {
        vec![primary_artist.clone()]
    };
    let album = if has_album {
        byline.get(1).cloned().unwrap_or_default()
    } else {
        String::new()
    };

    let duration = row
        .pointer("/lengthText/runs/0/text")
        .and_then(|v| v.as_str())
        .and_then(parse_mm_ss)
        .unwrap_or(0);

    let thumbnail = row
        .pointer("/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.iter().max_by_key(|t| t.get("width").and_then(|w| w.as_u64()).unwrap_or(0)))
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(normalize_yt_thumbnail);

    let path = match thumbnail {
        Some(ref url) if !url.is_empty() => PathBuf::from(format!(
            "{SOURCE_PREFIX}:{video_id}:{}",
            encode_url_tag(url)
        )),
        _ => PathBuf::from(format!("{SOURCE_PREFIX}:{video_id}")),
    };
    let album_id = synthesize_album_id(&album, &primary_artist);

    Some(Track {
        path,
        album_id,
        title,
        artist: primary_artist,
        album,
        duration,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists,
    })
}

fn normalize_yt_thumbnail(url: &str) -> String {
    // See discover.rs for the rationale: only rewrite when the URL
    // already carries a `=wNNN` size suffix; otherwise the suffix
    // glues onto mixart / query-style URLs and 404s.
    if let Some(idx) = url.rfind("=w")
        && url[idx + 2..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    {
        return format!("{}=w544-h544-l90-rj", &url[..idx]);
    }
    url.to_string()
}

fn parse_mm_ss(s: &str) -> Option<u64> {
    let mut parts = s.split(':').rev();
    let secs: u64 = parts.next()?.parse().ok()?;
    let mins: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let hours: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some(hours * 3600 + mins * 60 + secs)
}
