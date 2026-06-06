//! Browse the user's saved playlists and read their entries via
//! `/browse?browseId=FEmusic_liked_playlists` and `/browse?browseId=VL<id>`.
//!
//! YT Music distinguishes:
//! - **Liked Music** (`VLLM`) — the auto-playlist of liked songs. Handled
//!   separately by [`super::mod`]'s `get_liked_songs`.
//! - **User playlists** — anything else under "Library → Playlists." The
//!   browseId for the list itself is `FEmusic_liked_playlists`; each entry's
//!   nav endpoint carries `browseId = VL<playlistId>` for its contents.

use reader::models::Track;
use serde_json::Value;

use super::innertube;
use super::search::walk_playlist_shelf;

#[derive(Debug, Clone)]
pub struct YtPlaylistSummary {
    pub id: String,
    pub title: String,
    pub thumbnail_url: Option<String>,
}

/// List the signed-in user's playlists (everything under
/// "Library → Playlists"). Returns just metadata — call
/// [`get_playlist_entries`] to fetch each one's tracks lazily.
pub async fn list_playlists(cookies: &str) -> Result<Vec<YtPlaylistSummary>, String> {
    let resp: Value = innertube::browse("FEmusic_liked_playlists", cookies).await?;
    if has_sign_in_endpoint(&resp) {
        return Err("Sign-in prompt returned — cookies expired".to_string());
    }

    // The library playlists view returns a grid of musicTwoRowItemRenderer.
    // Layout (authenticated): contents.singleColumnBrowseResultsRenderer
    //   .tabs[0].tabRenderer.content.sectionListRenderer.contents[0]
    //   .gridRenderer.items[].musicTwoRowItemRenderer
    let items = resp
        .pointer(
            "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/gridRenderer/items",
        )
        .or_else(|| {
            resp.pointer(
                "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/gridRenderer/items",
            )
        })
        .and_then(|v| v.as_array());
    let Some(items) = items else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for item in items {
        let row = match item.get("musicTwoRowItemRenderer") {
            Some(r) => r,
            None => continue,
        };
        // The browseId of a playlist tile is `VL<playlistId>` — strip the
        // `VL` prefix to get the raw playlist ID we'll use later.
        let raw = row
            .pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(|v| v.as_str());
        let Some(raw) = raw else { continue };
        let id = raw.strip_prefix("VL").unwrap_or(raw).to_string();
        // YT scatters "New playlist" / "Episodes from podcasts" tiles
        // through this grid too; both have browseIds that don't start
        // with the PL/RD/OL/MM prefixes user playlists use. Skip them.
        if id.is_empty() || raw == "FEmusic_offline_storage" {
            continue;
        }
        let title = row
            .pointer("/title/runs/0/text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let thumbnail_url = row
            .pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0))
            })
            .and_then(|t| t.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string());

        out.push(YtPlaylistSummary {
            id,
            title,
            thumbnail_url,
        });
    }
    Ok(out)
}

/// Fetch every track inside a given playlist. `playlist_id` is the bare
/// playlist ID (without the `VL` prefix); we add it here. Follows
/// `nextContinuationData` until exhausted so playlists longer than the
/// first ~100-track page come through complete.
pub async fn get_playlist_entries(
    playlist_id: &str,
    cookies: &str,
) -> Result<Vec<Track>, String> {
    let browse_id = if playlist_id.starts_with("VL") {
        playlist_id.to_string()
    } else {
        format!("VL{playlist_id}")
    };
    let resp: Value = innertube::browse(&browse_id, cookies).await?;
    let (raw_first, mut next) = walk_playlist_shelf(&resp);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut tracks: Vec<Track> = raw_first
        .into_iter()
        .filter(|t| keep_unique(t, &mut seen))
        .collect();
    while let Some(token) = next.take() {
        let page = innertube::browse_continuation(&token, cookies).await?;
        let (more, next_token) = super::search::walk_playlist_continuation(&page);
        let unique: Vec<Track> = more
            .into_iter()
            .filter(|t| keep_unique(t, &mut seen))
            .collect();
        // An empty page after dedup means we've stopped making progress;
        // looping further on a continuation token YT keeps echoing back
        // would hammer the endpoint indefinitely.
        if unique.is_empty() {
            break;
        }
        tracks.extend(unique);
        next = next_token;
    }
    Ok(tracks)
}

/// Recursively look for a `signInEndpoint` object key in the response.
/// Cheaper and structurally correct vs. serialising the whole tree and
/// substring-searching the JSON.
fn has_sign_in_endpoint(v: &Value) -> bool {
    match v {
        Value::Object(map) => {
            map.contains_key("signInEndpoint")
                || map.values().any(has_sign_in_endpoint)
        }
        Value::Array(items) => items.iter().any(has_sign_in_endpoint),
        _ => false,
    }
}

fn keep_unique(t: &Track, seen: &mut std::collections::HashSet<String>) -> bool {
    let id = t
        .path
        .to_string_lossy()
        .split(':')
        .nth(1)
        .unwrap_or("")
        .to_string();
    !id.is_empty() && seen.insert(id)
}
