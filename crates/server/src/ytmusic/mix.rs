use reader::models::{Counterpart, SharedSegment, Track};
use serde_json::{Value, json};

use super::clients::WEB_REMIX;
use super::innertube::{sapisid_hash, signed};
use super::search::{linked_album, runs_with_browse};

const ORIGIN: &str = "https://music.youtube.com";

/// What a mix is generated from. The `RD…` id and whether the request pins a
/// first video are both derived from this one value, so they can't be paired
/// wrongly — a playlist mix with a `videoId` is not a request YT answers.
pub(super) enum MixSeed<'a> {
    /// One track: the mix plays on from it, with the seed itself at index 0.
    Video(&'a str),
    /// A whole playlist — YT's own "Start radio" on a playlist. The queue is
    /// generated, so it bears no relation to the playlist's track list.
    Playlist(&'a str),
}

impl MixSeed<'_> {
    /// The `RD…` playlist id the mix is built under.
    fn mix_playlist_id(&self) -> String {
        match self {
            Self::Video(id) => format!("RDAMVM{id}"),
            // Playlist ids are stored bare, but the `VL`-prefixed browse form
            // leaks in from some InnerTube responses (see `playlists.rs`) and
            // `RDAMPL` wants the bare one. Normalizing here rather than at
            // construction means no caller can skip it.
            Self::Playlist(id) => format!("RDAMPL{}", id.strip_prefix("VL").unwrap_or(id)),
        }
    }

    /// The video pinned as the queue's first entry, if any. Sending an empty
    /// `videoId` makes YT hand back an empty queue, so a playlist mix omits the
    /// field entirely rather than passing a blank one.
    fn pinned_video_id(&self) -> Option<&str> {
        match self {
            Self::Video(id) => Some(id),
            Self::Playlist(_) => None,
        }
    }
}

impl std::fmt::Display for MixSeed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Video(id) => write!(f, "video:{id}"),
            Self::Playlist(id) => write!(f, "playlist:{id}"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MixPage {
    pub tracks: Vec<Track>,
    /// The `RD…` id YT built the mix under; a continuation names it again.
    pub playlist_id: Option<String>,
    pub continuation: Option<String>,
}

/// Ask YT to generate a radio queue from `seed` and return its first page.
#[tracing::instrument(name = "yt.mix", skip(cookies), fields(seed = %seed))]
pub(super) async fn fetch(seed: MixSeed<'_>, cookies: &str) -> Result<MixPage, String> {
    let client = WEB_REMIX;
    let mut body = json!({
        "enablePersistentPlaylistPanel": true,
        "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
        "playlistId": seed.mix_playlist_id(),
        "params": "wAEB",
        "isAudioOnly": true,
        "context": {
            "client": {
                "clientName": client.client_name,
                "clientVersion": client.version(),
                "hl": "en",
                "gl": "US",
            },
            "user": { "lockedSafetyMode": false },
        },
    });
    if let Some(video_id) = seed.pinned_video_id()
        && let Some(obj) = body.as_object_mut()
    {
        obj.insert("videoId".into(), json!(video_id));
    }
    let resp = post_next(&body, cookies).await?;
    Ok(walk_queue(&resp))
}

#[tracing::instrument(name = "yt.mix_continuation", skip(token, cookies), fields(playlist = %playlist_id))]
pub(super) async fn fetch_continuation(
    playlist_id: &str,
    token: &str,
    cookies: &str,
) -> Result<MixPage, String> {
    let client = WEB_REMIX;
    let body = json!({
        "enablePersistentPlaylistPanel": true,
        "continuation": token,
        "playlistId": playlist_id,
        "isAudioOnly": true,
        "context": {
            "client": {
                "clientName": client.client_name,
                "clientVersion": client.version(),
                "hl": "en",
                "gl": "US",
            },
            "user": { "lockedSafetyMode": false },
        },
    });
    let resp = post_next(&body, cookies).await?;
    Ok(walk_continuation(&resp))
}

/// Works without auth (anonymous radio for any public video), so empty cookies
/// skip Cookie + SAPISID.
async fn post_next(body: &Value, cookies: &str) -> Result<Value, String> {
    let client = WEB_REMIX;
    let mut req = super::innertube::http_client()
        .clone()
        .post(format!("{ORIGIN}/youtubei/v1/next?prettyPrint=false"))
        // The same header set every other InnerTube call sends. This one was
        // missing the User-Agent and the API format version, which is what
        // YouTube started answering with a bare 403 -- browse and player, which
        // send them, kept working from the same session and cookies.
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.version())
        .header("X-Origin", ORIGIN)
        .header("Origin", ORIGIN)
        .header("Referer", format!("{ORIGIN}/"));
    let mut body = body.clone();
    if !cookies.is_empty() {
        let auth = sapisid_hash(cookies, ORIGIN).ok_or_else(|| "SAPISID missing".to_string())?;
        req = signed(req, &mut body, cookies, auth);
    }
    req.json(&body)
        .send()
        .await
        .map_err(|e| format!("next HTTP: {}", e.without_url()))?
        .error_for_status()
        .map_err(|e| format!("next HTTP: {}", e.without_url()))?
        .json()
        .await
        .map_err(|e| format!("next JSON: {}", e.without_url()))
}

#[tracing::instrument(name = "yt.watch_track", skip(cookies))]
pub(super) async fn watch_track(
    video_id: &str,
    cookies: Option<&str>,
) -> Result<Option<Track>, String> {
    let payload = json!({ "videoId": video_id, "isAudioOnly": true });
    let resp = super::innertube::post(WEB_REMIX, "next", payload, cookies).await?;
    Ok(walk_queue(&resp)
        .tracks
        .into_iter()
        .find(|track| track.id.key() == video_id))
}

pub(super) fn walk_queue(resp: &Value) -> MixPage {
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
        return MixPage::default();
    };
    tabs.iter()
        .find_map(|tab| {
            tab.get("tabRenderer")
                .and_then(|t| t.get("content"))
                .and_then(|c| c.get("musicQueueRenderer"))
                .and_then(|q| q.get("content"))
                .and_then(|c| c.get("playlistPanelRenderer"))
        })
        .map(walk_panel)
        .unwrap_or_default()
}

fn walk_continuation(resp: &Value) -> MixPage {
    resp.pointer("/continuationContents/playlistPanelContinuation")
        .map(walk_panel)
        .unwrap_or_default()
}

/// Takes a `playlistPanelRenderer` or a later page's `playlistPanelContinuation`.
fn walk_panel(panel: &Value) -> MixPage {
    let tracks = panel
        .get("contents")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let row = item.get("playlistPanelVideoRenderer").or_else(|| {
                        item.pointer(
                            "/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer",
                        )
                    })?;
                    let mut track = parse_queue_row(row)?;
                    track.counterpart = item
                        .pointer("/playlistPanelVideoWrapperRenderer/counterpart/0")
                        .and_then(parse_counterpart)
                        .map(Box::new);
                    Some(track)
                })
                .collect()
        })
        .unwrap_or_default();
    // `nextRadioContinuationData` for a mix, `nextContinuationData` for a
    // long playlist queue; the key is the only difference.
    let continuation = panel
        .get("continuations")
        .and_then(|v| v.as_array())
        .and_then(|list| {
            list.iter().find_map(|entry| {
                entry
                    .as_object()?
                    .values()
                    .find_map(|data| data.get("continuation")?.as_str())
            })
        })
        .filter(|token| !token.is_empty())
        .map(str::to_string);
    MixPage {
        tracks,
        playlist_id: panel
            .get("playlistId")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        continuation,
    }
}

/// The other cut a wrapper row pairs with its primary one. Only signed-in
/// sessions are sent wrapper rows.
fn parse_counterpart(v: &Value) -> Option<Counterpart> {
    let row = v.pointer("/counterpartRenderer/playlistPanelVideoRenderer")?;
    let ms = |s: &Value, key: &str| s.get(key)?.as_str()?.parse::<u64>().ok();
    let segments = v
        .pointer("/segmentMap/segment")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some(SharedSegment {
                start_ms: ms(s, "primaryVideoStartTimeMilliseconds")?,
                counterpart_start_ms: ms(s, "counterpartVideoStartTimeMilliseconds")?,
                duration_ms: ms(s, "durationMilliseconds")?,
            })
        })
        .collect();
    Some(Counterpart {
        item_id: row.get("videoId")?.as_str()?.to_string(),
        video: !is_song(music_video_type(row)),
        duration_ms: row
            .pointer("/lengthText/runs/0/text")
            .and_then(Value::as_str)
            .and_then(parse_mm_ss)
            .map(|secs| secs * 1000),
        segments,
    })
}

fn music_video_type(row: &Value) -> Option<&str> {
    row.pointer("/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
        .and_then(Value::as_str)
}

fn is_song(music_video_type: Option<&str>) -> bool {
    matches!(
        music_video_type,
        Some("MUSIC_VIDEO_TYPE_ATV" | "MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC")
    )
}

/// The other cut of `video_id` from either side of a watch-next row: a row whose
/// counterpart is the asked-for id says the same thing from the other end.
pub(super) fn counterpart_of(rows: &[Track], video_id: &str) -> Option<Counterpart> {
    rows.iter().find_map(|row| {
        let other = row.counterpart.as_deref()?;
        if row.id.key() == video_id {
            return Some(other.clone());
        }
        (other.item_id == video_id)
            .then(|| row.counterpart_track())
            .flatten()
            .and_then(|swapped| swapped.counterpart.map(|c| *c))
    })
}

/// The other cut of one track, as the watch-next queue YT opens for it pairs
/// them. Anonymous sessions are never sent the pairing, so they are not asked.
#[tracing::instrument(name = "yt.counterpart", skip(cookies), fields(video_id = %video_id))]
pub(super) async fn counterpart(
    video_id: &str,
    cookies: &str,
) -> Result<Option<Counterpart>, String> {
    if cookies.is_empty() {
        return Ok(None);
    }
    let client = WEB_REMIX;
    let body = json!({
        "videoId": video_id,
        "isAudioOnly": true,
        "enablePersistentPlaylistPanel": true,
        "context": {
            "client": {
                "clientName": client.client_name,
                "clientVersion": client.version(),
                "hl": "en",
                "gl": "US",
            },
        },
    });
    let auth = sapisid_hash(cookies, ORIGIN).ok_or_else(|| "SAPISID missing".to_string())?;
    let resp: Value = super::innertube::http_client()
        .clone()
        .post(format!("{ORIGIN}/youtubei/v1/next?prettyPrint=false"))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.version())
        .header("X-Origin", ORIGIN)
        .header("Origin", ORIGIN)
        .header("Referer", format!("{ORIGIN}/"))
        .header("Cookie", cookies)
        .header("Authorization", auth)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("next HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("next HTTP: {e}"))?
        .json()
        .await
        .map_err(|e| format!("next JSON: {e}"))?;
    Ok(counterpart_of(&walk_queue(&resp).tracks, video_id))
}

fn parse_queue_row(row: &Value) -> Option<Track> {
    let video_id = row.get("videoId").and_then(|v| v.as_str())?.to_string();
    let mvt = music_video_type(row);
    if !matches!(
        mvt,
        Some(
            "MUSIC_VIDEO_TYPE_ATV"
                | "MUSIC_VIDEO_TYPE_OMV"
                | "MUSIC_VIDEO_TYPE_UGC"
                | "MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC"
        )
    ) {
        return None;
    }
    let has_album = is_song(mvt);

    let title = row
        .pointer("/title/runs/0/text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let byline = runs_with_browse(row.pointer("/longBylineText/runs"));

    // For songs (has_album): byline = [artist, album, year-or-views, likes]
    // For videos:            byline = [artist, views, likes]
    let primary_artist = byline
        .first()
        .map(|(text, _)| text.clone())
        .unwrap_or_default();
    let artists = if primary_artist.is_empty() {
        Vec::new()
    } else {
        vec![primary_artist.clone()]
    };
    // The album is the run that links one; its text alone names no release.
    let (album, album_id) = match linked_album(&byline) {
        Some(linked) => linked,
        None if has_album => (
            byline
                .get(1)
                .map(|(text, _)| text.clone())
                .unwrap_or_default(),
            String::new(),
        ),
        None => (String::new(), String::new()),
    };

    let duration = row
        .pointer("/lengthText/runs/0/text")
        .and_then(|v| v.as_str())
        .and_then(parse_mm_ss)
        .unwrap_or(0);

    let thumbnail = row
        .pointer("/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .max_by_key(|t| t.get("width").and_then(|w| w.as_u64()).unwrap_or(0))
        })
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(normalize_yt_thumbnail);

    let cover = thumbnail.filter(|u| !u.is_empty());

    Some(Track {
        id: super::yt_id(video_id.to_string()),
        cover,
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
        credits: Vec::new(),
        artists,
        replay_gain: config::ReplayGainInfo::default(),
        explicit: super::has_explicit_badge(row),
        plays: None,
        counterpart: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn recorded() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/next_radio.json"))
            .expect("fixture is JSON")
    }

    #[test]
    fn a_radio_response_lists_its_tracks_and_the_next_page() {
        let page = walk_queue(&recorded());

        assert!(page.tracks.len() >= 40, "got {}", page.tracks.len());
        assert_eq!(page.tracks[0].id.key(), "IluRBvnYMoY");
        assert_eq!(page.playlist_id.as_deref(), Some("RDAMVMIluRBvnYMoY"));
        assert!(
            page.continuation
                .as_deref()
                .is_some_and(|token| token.starts_with("CDIS")),
            "{:?}",
            page.continuation
        );
    }

    #[test]
    fn a_continuation_page_parses_like_the_first() {
        let first = recorded();
        let panel = first
            .pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer")
            .expect("queue panel")
            .clone();
        let continued = json!({ "continuationContents": { "playlistPanelContinuation": panel } });

        assert_eq!(walk_continuation(&continued), walk_queue(&first));
    }

    #[test]
    fn a_last_page_has_no_continuation() {
        let page = walk_continuation(&json!({
            "continuationContents": { "playlistPanelContinuation": { "contents": [] } }
        }));

        assert_eq!(page, MixPage::default());
    }
}

#[cfg(test)]
mod album_tests {
    use super::parse_queue_row;
    use serde_json::json;

    fn queue_row(byline: serde_json::Value) -> serde_json::Value {
        json!({
            "videoId": "vid",
            "navigationEndpoint": { "watchEndpoint": { "watchEndpointMusicSupportedConfigs": {
                "watchEndpointMusicConfig": { "musicVideoType": "MUSIC_VIDEO_TYPE_ATV" }
            } } },
            "title": { "runs": [{ "text": "Song" }] },
            "longBylineText": { "runs": byline },
        })
    }

    #[test]
    fn a_queue_row_files_under_the_release_its_byline_links() {
        let track = parse_queue_row(&queue_row(json!([
            { "text": "Ada" },
            { "text": " • " },
            { "text": "Hits", "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_one" } } },
            { "text": " • " },
            { "text": "2021" }
        ])))
        .expect("a song row");
        assert_eq!(track.album, "Hits");
        assert_eq!(track.album_id, "ytmusic:album:MPREb_one");
    }

    /// The text still shows; it is the id that would have been a guess.
    #[test]
    fn an_unlinked_album_name_files_under_no_album() {
        let track = parse_queue_row(&queue_row(json!([
            { "text": "Ada" },
            { "text": " • " },
            { "text": "Hits" },
            { "text": " • " },
            { "text": "2021" }
        ])))
        .expect("a song row");
        assert_eq!(track.album, "Hits");
        assert_eq!(track.album_id, "");
    }
}
