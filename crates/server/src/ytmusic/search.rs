use std::path::PathBuf;

use reader::models::Track;
use serde_json::{Value, json};

use super::SOURCE_PREFIX;
use super::clients::WEB_REMIX;
use super::innertube::sapisid_hash;

const ORIGIN_YT_MUSIC: &str = "https://music.youtube.com";
const SONGS_FILTER: &str = "EgWKAQIIAWoMEAMQBBAJEAoQDhAV";
const VIDEOS_FILTER: &str = "EgWKAQIQAWoMEAMQBBAJEAoQDhAV";
// `params` value for "Artists" tab on YT Music search. Restricts hits
// to musicResponsiveListItemRenderer rows whose nav endpoint browseId
// begins with `UC…`, exactly what we need for name → channel resolve.
const ARTISTS_FILTER: &str = "EgWKAQIgAWoMEAMQBBAJEAoQDhAV";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicVideoType {
    AlbumTrack,
    OfficialMusicVideo,
    UserGenerated,
    OfficialSourceMusic,
}

impl MusicVideoType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "MUSIC_VIDEO_TYPE_ATV" => Some(Self::AlbumTrack),
            "MUSIC_VIDEO_TYPE_OMV" => Some(Self::OfficialMusicVideo),
            "MUSIC_VIDEO_TYPE_UGC" => Some(Self::UserGenerated),
            "MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC" => Some(Self::OfficialSourceMusic),
            _ => None,
        }
    }

    /// True for types whose rows include an album field (album tracks
    /// and provider-tagged music). Videos (OMV / UGC) put view-count in
    /// the slot a song would put album.
    fn has_album(self) -> bool {
        matches!(self, Self::AlbumTrack | Self::OfficialSourceMusic)
    }
}

#[derive(Debug, Clone)]
struct ParsedRow {
    video_id: String,
    title: String,
    artists: Vec<String>,
    album: Option<String>,
    album_browse_id: Option<String>,
    duration: u64,
    thumbnail_url: Option<String>,
}

#[tracing::instrument(name = "yt.search", skip(cookies), fields(query = %query))]
pub async fn music_search_tracks(
    query: &str,
    cookies: Option<&str>,
) -> Result<Vec<Track>, String> {
    let http = super::innertube::http_client();
    let (top, songs, videos) = tokio::join!(
        do_search(http, query, None, cookies),
        do_search(http, query, Some(SONGS_FILTER), cookies),
        do_search(http, query, Some(VIDEOS_FILTER), cookies),
    );

    let top = top?;
    let mut songs = songs?.into_iter();
    let mut videos = videos?.into_iter();

    let mut out: Vec<Track> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push = |t: Track, out: &mut Vec<Track>, seen: &mut std::collections::HashSet<String>| {
        let id = track_id(&t);
        if id.is_empty() || seen.insert(id) {
            out.push(t);
        }
    };

    for t in top {
        push(t, &mut out, &mut seen);
    }
    loop {
        let s = songs.next();
        let v = videos.next();
        if s.is_none() && v.is_none() {
            break;
        }
        if let Some(s) = s {
            push(s, &mut out, &mut seen);
        }
        if let Some(v) = v {
            push(v, &mut out, &mut seen);
        }
    }
    Ok(out)
}

/// Resolve a free-text artist name to a YT Music channel id (`UC…`).
/// Powers the artist page when navigation only had a name (track row
/// click, sidebar tag, etc.) and the YT backend is active. Returns
/// None if the search returned no artist row at all.
#[tracing::instrument(name = "yt.resolve_artist", skip(cookies), fields(query = %query))]
pub async fn resolve_artist_channel_id(
    query: &str,
    cookies: Option<&str>,
) -> Result<Option<String>, String> {
    if query.trim().is_empty() {
        return Ok(None);
    }
    let http = super::innertube::http_client();
    let resp = do_search_raw(http, query, Some(ARTISTS_FILTER), cookies).await?;
    Ok(walk_first_artist_browse_id(&resp))
}

/// Recursively walk the JSON for the first browseEndpoint pointing at
/// a `UC…` channel. The artists filter restricts results to artist
/// rows so the first hit is the top-ranked match.
fn walk_first_artist_browse_id(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            if let Some(ep) = map.get("browseEndpoint")
                && let Some(bid) = ep.get("browseId").and_then(|x| x.as_str())
                && bid.starts_with("UC")
            {
                return Some(bid.to_string());
            }
            for child in map.values() {
                if let Some(found) = walk_first_artist_browse_id(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => arr.iter().find_map(walk_first_artist_browse_id),
        _ => None,
    }
}

async fn do_search_raw(
    http: &reqwest::Client,
    query: &str,
    params: Option<&str>,
    cookies: Option<&str>,
) -> Result<Value, String> {
    let client = WEB_REMIX;
    let mut body = json!({
        "context": {
            "client": {
                "clientName": client.client_name,
                "clientVersion": client.client_version,
                "hl": "en",
                "gl": "US",
            },
        },
        "query": query,
    });
    if let Some(p) = params {
        body.as_object_mut().unwrap().insert("params".into(), json!(p));
    }
    let mut req = http
        .post(format!("{ORIGIN_YT_MUSIC}/youtubei/v1/search?prettyPrint=false"))
        .header("Content-Type", "application/json")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("Origin", ORIGIN_YT_MUSIC)
        .header("Referer", format!("{ORIGIN_YT_MUSIC}/"))
        .json(&body);
    if let Some(c) = cookies {
        req = req.header("Cookie", c);
        if let Some(auth) = sapisid_hash(c, ORIGIN_YT_MUSIC) {
            req = req.header("Authorization", auth);
        }
    }
    req.send()
        .await
        .map_err(|e| format!("search HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("search HTTP: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("search JSON: {e}"))
}

async fn do_search(
    http: &reqwest::Client,
    query: &str,
    params: Option<&str>,
    cookies: Option<&str>,
) -> Result<Vec<Track>, String> {
    let resp = do_search_raw(http, query, params, cookies).await?;
    Ok(walk_tracks(&resp))
}

fn walk_tracks(resp: &Value) -> Vec<Track> {
    let shelves = resp
        .pointer(
            "/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
        )
        .and_then(|v| v.as_array());
    let Some(shelves) = shelves else {
        return Vec::new();
    };

    let mut out: Vec<Track> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut emit = |row: ParsedRow, out: &mut Vec<Track>| {
        if seen_ids.insert(row.video_id.clone()) {
            out.push(parsed_to_track(row));
        }
    };
    for shelf in shelves {
        if let Some(card) = shelf.get("musicCardShelfRenderer") {
            if let Some(parsed) = parse_card_shelf(card) {
                emit(parsed, &mut out);
            }
            if let Some(items) = card.get("contents").and_then(|v| v.as_array()) {
                for item in items {
                    if let Some(parsed) = parse_row(item) {
                        emit(parsed, &mut out);
                    }
                }
            }
        }
        if let Some(items) = shelf
            .pointer("/musicShelfRenderer/contents")
            .and_then(|v| v.as_array())
        {
            for item in items {
                if let Some(parsed) = parse_row(item) {
                    emit(parsed, &mut out);
                }
            }
        }
    }
    out
}

fn track_id(t: &Track) -> String {
    t.path
        .to_string_lossy()
        .split(':')
        .nth(1)
        .unwrap_or("")
        .to_string()
}

fn parse_card_shelf(card: &Value) -> Option<ParsedRow> {
    let endpoint = card.pointer("/onTap/watchEndpoint")?;
    let video_id = endpoint.get("videoId").and_then(|v| v.as_str())?.to_string();
    let mvt = endpoint
        .pointer("/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
        .and_then(|v| v.as_str())
        .and_then(MusicVideoType::from_str)?;

    let title = card
        .pointer("/title/runs/0/text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Subtitle is "Kind • Artist • [Album|Views|Year] • [...]". First token is
    // the kind label which mvt already encodes; skip it and take the next as
    // the primary artist. For songs the third slot is the album; for videos
    // it's view count which we drop.
    let mut subtitle: Vec<String> = card
        .pointer("/subtitle/runs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                .filter(|s| !is_separator(s))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    if !subtitle.is_empty() {
        subtitle.remove(0);
    }
    let artist = subtitle.first().cloned().unwrap_or_default();
    let album = if mvt.has_album() {
        subtitle.get(1).cloned()
    } else {
        None
    };

    let thumbnail_url = card
        .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.iter().max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0)))
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(normalize_yt_thumbnail);

    Some(ParsedRow {
        video_id,
        title,
        artists: if artist.is_empty() {
            Vec::new()
        } else {
            vec![artist]
        },
        album,
        album_browse_id: None,
        duration: 0,
        thumbnail_url,
    })
}

fn parse_row(item: &Value) -> Option<ParsedRow> {
    let row = item.get("musicResponsiveListItemRenderer")?;
    let mvt = find_music_video_type(row).and_then(MusicVideoType::from_str)?;
    let video_id = row
        .pointer("/playlistItemData/videoId")
        .and_then(|v| v.as_str())?
        .to_string();
    let thumbnail_url = best_thumbnail(row);
    let title = pick_run(row, 0, 0);

    // Playlist-track rows ship the duration in a separate `fixedColumns`
    // cell. Search-result rows pack everything into flex[1] separated by
    // " • " runs. The shapes are visually distinct in the JSON so we
    // dispatch on presence, not on guesswork.
    if row.get("fixedColumns").is_some() {
        Some(parse_playlist_track(row, video_id, title, mvt, thumbnail_url))
    } else {
        Some(parse_search_row(row, video_id, title, mvt, thumbnail_url))
    }
}

fn parse_playlist_track(
    row: &Value,
    video_id: String,
    title: String,
    mvt: MusicVideoType,
    thumbnail_url: Option<String>,
) -> ParsedRow {
    let primary_artist = pick_run(row, 1, 0);
    let artists = if primary_artist.is_empty() {
        Vec::new()
    } else {
        vec![primary_artist]
    };
    let album = if mvt.has_album() {
        let s = pick_run(row, 2, 0);
        if s.is_empty() { None } else { Some(s) }
    } else {
        None
    };
    let album_browse_id = if mvt.has_album() {
        row
            .pointer("/flexColumns/2/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/browseEndpoint/browseId")
            .and_then(|v| v.as_str())
            .filter(|s| s.starts_with("MPRE"))
            .map(|s| s.to_string())
    } else {
        None
    };
    let duration = row
        .pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text/runs/0/text")
        .and_then(|v| v.as_str())
        .and_then(parse_mm_ss)
        .unwrap_or(0);

    ParsedRow {
        video_id,
        title,
        artists,
        album,
        album_browse_id,
        duration,
        thumbnail_url,
    }
}

fn parse_search_row(
    row: &Value,
    video_id: String,
    title: String,
    mvt: MusicVideoType,
    thumbnail_url: Option<String>,
) -> ParsedRow {
    // flex[1] runs, separators filtered:
    //   has_album: [artist..., album, duration]
    //   else:      [artist..., view-count, duration]
    // duration is always last; the slot before it is album OR view-count
    // depending on mvt. We dispatch on mvt — no view-count text sniffing.
    let mut tokens = pick_all_runs(row, 1);
    let duration = tokens
        .pop()
        .as_deref()
        .and_then(parse_mm_ss)
        .unwrap_or(0);
    let second_last = tokens.pop();
    let (album, artists) = if mvt.has_album() {
        let album = second_last.filter(|s| !s.is_empty());
        (album, tokens)
    } else {
        // second_last is the view count; drop it, the remaining tokens
        // are artists.
        (None, tokens)
    };

    ParsedRow {
        video_id,
        title,
        artists,
        album,
        album_browse_id: None,
        duration,
        thumbnail_url,
    }
}

fn parsed_to_track(p: ParsedRow) -> Track {
    let primary_artist = p.artists.first().cloned().unwrap_or_default();
    let album = p.album.clone().unwrap_or_default();
    let album_id = match p.album_browse_id {
        Some(id) => format!("{SOURCE_PREFIX}:album:{id}"),
        None => synthesize_album_id(&album, &primary_artist),
    };
    let path = match p.thumbnail_url {
        Some(ref url) if !url.is_empty() => PathBuf::from(format!(
            "{SOURCE_PREFIX}:{}:{}",
            p.video_id,
            encode_url_tag(url)
        )),
        _ => PathBuf::from(format!("{SOURCE_PREFIX}:{}", p.video_id)),
    };

    Track {
        path,
        album_id,
        title: p.title,
        artist: primary_artist,
        album,
        duration: p.duration,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: p.artists,
    }
}

fn pick_run(row: &Value, col: usize, run: usize) -> String {
    row.get("flexColumns")
        .and_then(|c| c.as_array())
        .and_then(|cs| cs.get(col))
        .and_then(|c| c.pointer(&format!("/musicResponsiveListItemFlexColumnRenderer/text/runs/{run}/text")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn pick_all_runs(row: &Value, col: usize) -> Vec<String> {
    row.get("flexColumns")
        .and_then(|c| c.as_array())
        .and_then(|cs| cs.get(col))
        .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(|v| v.as_array())
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                .filter(|s| !is_separator(s))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn is_separator(s: &str) -> bool {
    matches!(s, " • " | " & " | ", ")
}

fn walk_items(items: &[Value]) -> (Vec<Track>, Option<String>) {
    let mut tracks = Vec::new();
    let mut continuation = None;
    for item in items {
        if let Some(cont) = item.get("continuationItemRenderer") {
            continuation = cont
                .pointer("/continuationEndpoint/continuationCommand/token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            continue;
        }
        if let Some(parsed) = parse_row(item) {
            tracks.push(parsed_to_track(parsed));
        }
    }
    (tracks, continuation)
}

pub fn walk_playlist_shelf(resp: &Value) -> (Vec<Track>, Option<String>) {
    let shelves = resp
        .pointer(
            "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents",
        )
        .or_else(|| {
            resp.pointer(
                "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
            )
        })
        .and_then(|v| v.as_array());
    let Some(shelves) = shelves else {
        return (Vec::new(), None);
    };
    let mut tracks = Vec::new();
    let mut continuation = None;
    for shelf in shelves {
        let Some(playlist) = shelf.get("musicPlaylistShelfRenderer") else {
            continue;
        };
        if let Some(items) = playlist.get("contents").and_then(|v| v.as_array()) {
            let (page_tracks, page_cont) = walk_items(items);
            tracks.extend(page_tracks);
            if continuation.is_none() {
                continuation = page_cont;
            }
        }
        if continuation.is_none() {
            continuation = playlist
                .pointer("/continuations/0/nextContinuationData/continuation")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    (tracks, continuation)
}

pub fn walk_playlist_continuation(resp: &Value) -> (Vec<Track>, Option<String>) {
    if let Some(actions) = resp
        .pointer("/onResponseReceivedActions")
        .and_then(|v| v.as_array())
    {
        let mut tracks = Vec::new();
        let mut continuation = None;
        for action in actions {
            if let Some(items) = action
                .pointer("/appendContinuationItemsAction/continuationItems")
                .and_then(|v| v.as_array())
            {
                let (page_tracks, page_cont) = walk_items(items);
                tracks.extend(page_tracks);
                if continuation.is_none() {
                    continuation = page_cont;
                }
            }
        }
        return (tracks, continuation);
    }
    if let Some(cont) = resp
        .pointer("/continuationContents/musicPlaylistShelfContinuation")
        .or_else(|| resp.pointer("/continuationContents/musicShelfContinuation"))
    {
        let items = cont
            .get("contents")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let (tracks, mut continuation) = walk_items(&items);
        if continuation.is_none() {
            continuation = cont
                .pointer("/continuations/0/nextContinuationData/continuation")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        return (tracks, continuation);
    }
    (Vec::new(), None)
}

fn find_music_video_type(v: &Value) -> Option<&str> {
    match v {
        Value::Object(m) => {
            if let Some(t) = m.get("musicVideoType").and_then(|x| x.as_str()) {
                return Some(t);
            }
            for child in m.values() {
                if let Some(t) = find_music_video_type(child) {
                    return Some(t);
                }
            }
            None
        }
        Value::Array(arr) => {
            for child in arr {
                if let Some(t) = find_music_video_type(child) {
                    return Some(t);
                }
            }
            None
        }
        _ => None,
    }
}

fn best_thumbnail(row: &Value) -> Option<String> {
    let thumbs = row
        .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")?
        .as_array()?;
    thumbs
        .iter()
        .max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0))
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(normalize_yt_thumbnail)
}

fn normalize_yt_thumbnail(url: &str) -> String {
    // Only rewrite photo-CDN URLs whose existing size suffix is
    // `=wNNN-hNNN…`. Other shapes (mixart token URLs, query-string
    // CDN URLs) get the suffix glued on incorrectly and 404. Match
    // discover.rs's guarded version: require `=w` immediately
    // followed by a digit.
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

pub(crate) fn encode_url_tag(url: &str) -> String {
    format!("urlhex_{}", hex::encode(url.as_bytes()))
}

pub(crate) fn synthesize_album_id(album: &str, artist: &str) -> String {
    if album.is_empty() {
        return format!("{SOURCE_PREFIX}:album:singles");
    }
    let mut key = album.to_lowercase();
    if !artist.is_empty() {
        key.push('|');
        key.push_str(&artist.to_lowercase());
    }
    format!("{SOURCE_PREFIX}:album:{}", hex::encode(key.as_bytes()))
}

fn parse_mm_ss(s: &str) -> Option<u64> {
    let mut parts = s.split(':').rev();
    let secs: u64 = parts.next()?.parse().ok()?;
    let mins: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let hours: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some(hours * 3600 + mins * 60 + secs)
}
