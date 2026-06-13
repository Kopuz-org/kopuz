//! Write-side InnerTube endpoints — like/unlike a video, add/remove from
//! a playlist. All require the user's cookies + SAPISIDHASH auth.

use serde_json::{Value, json};

use super::clients::{ORIGIN_YOUTUBE_MUSIC, WEB_REMIX};
use super::innertube::sapisid_hash;

async fn post(endpoint: &str, body: Value, cookies: &str) -> Result<Value, String> {
    let client = WEB_REMIX;
    let auth =
        sapisid_hash(cookies, ORIGIN_YOUTUBE_MUSIC).ok_or_else(|| "SAPISID missing".to_string())?;
    let resp = super::innertube::http_client()
        .clone()
        .post(format!(
            "{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/{endpoint}?prettyPrint=false"
        ))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"))
        .header("Cookie", cookies)
        .header("Authorization", auth)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{endpoint} HTTP: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{endpoint} HTTP {}", resp.status()));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| format!("{endpoint} JSON parse: {e}"))
}

fn ytmusic_context() -> Value {
    json!({
        "client": {
            "clientName": WEB_REMIX.client_name,
            "clientVersion": WEB_REMIX.client_version,
            "hl": "en",
            "gl": "US",
        },
    })
}

/// Add a video to the user's Liked Music auto-playlist.
#[tracing::instrument(name = "yt.like", skip(cookies), fields(video_id = %video_id))]
pub async fn like_video(video_id: &str, cookies: &str) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "target": { "videoId": video_id },
    });
    post("like/like", body, cookies).await.map(|_| ())
}

/// Remove a video from the user's Liked Music auto-playlist (unlike).
#[tracing::instrument(name = "yt.unlike", skip(cookies), fields(video_id = %video_id))]
pub async fn unlike_video(video_id: &str, cookies: &str) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "target": { "videoId": video_id },
    });
    post("like/removelike", body, cookies).await.map(|_| ())
}

/// Add a video to a user playlist. `playlist_id` is the bare ID (no `VL`
/// prefix); `video_id` is the YT video ID.
#[tracing::instrument(name = "yt.playlist_add", skip(cookies), fields(playlist_id = %playlist_id, video_id = %video_id))]
pub async fn add_to_playlist(
    playlist_id: &str,
    video_id: &str,
    cookies: &str,
) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "playlistId": playlist_id,
        "actions": [{
            "action": "ACTION_ADD_VIDEO",
            "addedVideoId": video_id,
        }],
    });
    post("browse/edit_playlist", body, cookies)
        .await
        .map(|_| ())
}

/// Remove a video from a user playlist by video ID. (YT's API also
/// supports remove-by-setVideoId for repeats; we use the simpler
/// by-video-ID form which removes the first occurrence.)
#[tracing::instrument(name = "yt.playlist_remove", skip(cookies), fields(playlist_id = %playlist_id, video_id = %video_id))]
pub async fn remove_from_playlist(
    playlist_id: &str,
    video_id: &str,
    cookies: &str,
) -> Result<(), String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "playlistId": playlist_id,
        "actions": [{
            "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID",
            "removedVideoId": video_id,
        }],
    });
    post("browse/edit_playlist", body, cookies)
        .await
        .map(|_| ())
}

/// Create a new playlist with an optional initial set of video IDs.
#[tracing::instrument(name = "yt.playlist_create", skip(cookies, video_ids), fields(title = %title, count = video_ids.len()))]
pub async fn create_playlist(
    title: &str,
    video_ids: &[&str],
    cookies: &str,
) -> Result<String, String> {
    let body = json!({
        "context": { "client": ytmusic_context()["client"], "user": { "lockedSafetyMode": false } },
        "title": title,
        "description": "",
        "privacyStatus": "PRIVATE",
        "videoIds": video_ids,
    });
    let resp = post("playlist/create", body, cookies).await?;
    resp.get("playlistId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "create_playlist: no playlistId in response".to_string())
}
