//! YouTube Music Home feed parser. The wire format was reverse-engineered
//! against a live response — see `yttools/discover-home-probe` and
//! `discover-continuation-probe` for the recordings. Three shelves come
//! back per page; the section-list-level continuation token feeds the
//! next three.

use reader::models::Track;
use serde_json::{Value, json};

use super::clients::{ORIGIN_YOUTUBE_MUSIC, WEB_REMIX};
use super::innertube::{http_client, sapisid_hash};
use super::search::synthesize_album_id;

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
    /// Render as a vertical song list (with row numbers / duration)
    /// instead of a horizontal tile carousel. Only set true for the
    /// artist-page "Top songs" shelf — discover-home shelves stay
    /// horizontal.
    pub is_song_list: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiscoverItem {
    Song(Box<Track>),
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

#[tracing::instrument(name = "yt.discover_home", skip(cookies))]
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

/// Verified against /tmp/yt-artist-UC*.json via yttools/artist_probe.
///
/// `/browse?browseId=UC…` returns a `musicImmersiveHeaderRenderer` at
/// /header (with banner + subscribers + a shuffle play button) and a
/// section list whose entries are either `musicShelfRenderer` (the
/// "Top songs" list) or `musicCarouselShelfRenderer` (Albums / Singles
/// & EPs / Videos / Playlists / From your library / Fans might also
/// like). Carousel tiles are the same `musicTwoRowItemRenderer` shape
/// used in Discover home, so we reuse the existing tile classifier.
#[derive(Debug, Clone, PartialEq)]
pub struct YtArtist {
    pub channel_id: String,
    pub name: String,
    pub subscribers: Option<String>,
    pub description: Option<String>,
    pub banner_thumbnail: Option<String>,
    pub shuffle_playlist_id: Option<String>,
    pub sections: Vec<DiscoverShelf>,
}

#[tracing::instrument(name = "yt.fetch_artist", skip(cookies), fields(channel_id = %channel_id))]
pub async fn fetch_artist(channel_id: &str, cookies: &str) -> Result<YtArtist, String> {
    let body = build_browse_body(Some(channel_id));
    let resp = post(
        &format!("{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/browse?prettyPrint=false"),
        &body,
        cookies,
    )
    .await?;
    Ok(parse_artist(channel_id, &resp))
}

fn parse_artist(channel_id: &str, resp: &Value) -> YtArtist {
    let header = find_artist_header(resp);

    let name = header
        .and_then(|h| runs_text(h, "/title/runs"))
        .unwrap_or_default();
    let subscribers = header.and_then(|h| {
        runs_text(
            h,
            "/subscriptionButton/subscribeButtonRenderer/longSubscriberCountText/runs",
        )
        .or_else(|| runs_text(h, "/subscriberCountText/runs"))
    });
    let description = header.and_then(|h| runs_text(h, "/description/runs"));
    let banner_thumbnail = header.and_then(best_artist_banner);
    let shuffle_playlist_id = header
        .and_then(|h| h.get("buttons").and_then(|v| v.as_array()))
        .and_then(|btns| {
            btns.iter().find_map(|b| {
                b.get("musicPlayButtonRenderer")
                    .and_then(|p| p.pointer("/playNavigationEndpoint/watchEndpoint/playlistId"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
        });

    let mut sections = Vec::new();
    for section in album_section_contents(resp) {
        if let Some(shelf) = parse_artist_carousel(section) {
            sections.push(shelf);
        } else if let Some(shelf) = parse_artist_song_list(section) {
            sections.push(shelf);
        }
    }

    YtArtist {
        channel_id: channel_id.to_string(),
        name,
        subscribers,
        description,
        banner_thumbnail,
        shuffle_playlist_id,
        sections,
    }
}

fn find_artist_header(resp: &Value) -> Option<&Value> {
    // The immersive header sits at the root, not in the section list.
    if let Some(h) = resp.pointer("/header/musicImmersiveHeaderRenderer") {
        return Some(h);
    }
    if let Some(h) = resp.pointer("/header/musicVisualHeaderRenderer") {
        return Some(h);
    }
    None
}

fn best_artist_banner(header: &Value) -> Option<String> {
    for ptr in [
        "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        "/foregroundThumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
    ] {
        if let Some(arr) = header.pointer(ptr).and_then(|v| v.as_array()) {
            let best = arr
                .iter()
                .max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0))
                .and_then(|t| t.get("url").and_then(|u| u.as_str()))
                .map(|s| normalize_yt_thumbnail(s.to_string()));
            if best.is_some() {
                return best;
            }
        }
    }
    None
}

/// Carousel shelf — same tile shape (musicTwoRowItemRenderer) as the
/// Discover home parser, so we hand each tile back to `parse_tile`.
fn parse_artist_carousel(section: &Value) -> Option<DiscoverShelf> {
    let shelf = section.get("musicCarouselShelfRenderer")?;
    let header = shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer");
    let title = header
        .and_then(|h| runs_text(h, "/title/runs"))
        .unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    let strapline = header.and_then(|h| runs_text(h, "/strapline/runs"));
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
        is_song_list: false,
    })
}

/// "Top songs" comes back as a list shelf (musicShelfRenderer), not a
/// carousel — rows are `musicResponsiveListItemRenderer` like album
/// tracks. We turn each row into a `DiscoverItem::Song(Track)` so the
/// same `ShelfRow` component renders it as a row of song tiles.
fn parse_artist_song_list(section: &Value) -> Option<DiscoverShelf> {
    let shelf = section.get("musicShelfRenderer")?;
    let title = runs_text(shelf, "/title/runs").unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    let more_browse_id = shelf
        .pointer("/bottomEndpoint/browseEndpoint/browseId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let items: Vec<DiscoverItem> = shelf
        .get("contents")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|i| i.get("musicResponsiveListItemRenderer"))
                .filter_map(parse_artist_song_row)
                .map(|t| DiscoverItem::Song(Box::new(t)))
                .collect()
        })
        .unwrap_or_default();
    if items.is_empty() {
        return None;
    }
    Some(DiscoverShelf {
        title,
        strapline: None,
        more_browse_id,
        items,
        is_song_list: true,
    })
}

fn parse_artist_song_row(row: &Value) -> Option<Track> {
    // Classify every flex column by what it actually carries — the
    // artist Top Songs shelf has 4 columns in order title/artist/
    // play-count/album, NOT the title/artist/album layout my old
    // positional parser assumed. We pick out each role by tag, so
    // future re-orderings or extra columns just work.
    let cols = classify_flex_columns(row);
    let mut video_id: Option<String> = None;
    let mut title = String::new();
    let mut artist = String::new();
    let mut album = String::new();
    let mut flex_duration: Option<u64> = None;
    for c in &cols {
        match c {
            RowColumn::Title {
                text,
                video_id: vid,
                ..
            } => {
                if title.is_empty() {
                    title = text.clone();
                }
                if video_id.is_none()
                    && let Some(v) = vid
                {
                    video_id = Some(v.clone());
                }
            }
            RowColumn::Artist { text } if artist.is_empty() => artist = text.clone(),
            RowColumn::Album { text } if album.is_empty() => album = text.clone(),
            RowColumn::Duration { secs } if flex_duration.is_none() => {
                flex_duration = Some(*secs);
            }
            _ => {}
        }
    }
    let video_id = video_id.or_else(|| {
        row.pointer("/playlistItemData/videoId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })?;
    if title.is_empty() {
        return None;
    }
    let duration = fixed_columns_duration(row).or(flex_duration).unwrap_or(0);

    let thumbnail = row
        .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0))
        })
        .and_then(|t| t.get("url").and_then(|u| u.as_str()))
        .map(|s| normalize_yt_thumbnail(s.to_string()));

    let cover = thumbnail.map(|u| u.to_string()).filter(|u| !u.is_empty());
    let artists = if artist.is_empty() {
        Vec::new()
    } else {
        vec![artist.clone()]
    };
    Some(Track {
        id: super::yt_id(video_id.clone()),
        cover,
        album_id: synthesize_album_id(&album, &artist),
        title,
        artist,
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

#[tracing::instrument(name = "yt.fetch_album", skip(cookies), fields(browse_id = %browse_id))]
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
        .and_then(|h| runs_text(h, "/title/runs"))
        .unwrap_or_default();

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
            // OLAK… playlist id lives on the Title column's watch
            // endpoint — `classify_flex_columns` pulls it out by name,
            // no /flexColumns/N positional dive.
            if audio_pid_from_rows.is_none() {
                for c in classify_flex_columns(row) {
                    if let RowColumn::Title {
                        playlist_id: Some(pid),
                        ..
                    } = c
                    {
                        audio_pid_from_rows = Some(pid);
                        break;
                    }
                }
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
    // Two-pass: prefer the modern Responsive header across all
    // sections before falling back to the legacy Detail header. A
    // single-pass interleaved scan would let a stray Detail in
    // section[0] win over a Responsive in section[1] during a YT
    // layout-migration window, silently dropping artist / audio
    // playlist id (Detail header has neither).
    for section in sections {
        if let Some(h) = section.get("musicResponsiveHeaderRenderer") {
            return Some(h);
        }
    }
    for section in sections {
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
    // Use `let else continue` instead of `?` so a single empty/structural
    // run in the middle doesn't abort the whole scan and miss the real
    // artist later in the array.
    let arr = header
        .pointer("/subtitle/runs")
        .and_then(|v| v.as_array())?;
    for r in arr {
        let Some(text) = r.get("text").and_then(|v| v.as_str()) else {
            continue;
        };
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
    let cols = classify_flex_columns(row);
    let mut video_id: Option<String> = None;
    let mut title = String::new();
    let mut row_artist: Option<String> = None;
    let mut flex_duration: Option<u64> = None;
    for c in &cols {
        match c {
            RowColumn::Title {
                text,
                video_id: vid,
                ..
            } => {
                if title.is_empty() {
                    title = text.clone();
                }
                if video_id.is_none()
                    && let Some(v) = vid
                {
                    video_id = Some(v.clone());
                }
            }
            RowColumn::Artist { text } if row_artist.is_none() => {
                row_artist = Some(text.clone());
            }
            RowColumn::Duration { secs } if flex_duration.is_none() => {
                flex_duration = Some(*secs);
            }
            _ => {}
        }
    }
    let video_id = video_id.or_else(|| {
        row.pointer("/playlistItemData/videoId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })?;
    if title.is_empty() {
        return None;
    }
    let primary_artist = row_artist
        .or_else(|| album_artist.map(|s| s.to_string()))
        .unwrap_or_default();
    let duration = fixed_columns_duration(row).or(flex_duration).unwrap_or(0);
    let track_number = row_index_text(row).and_then(|s| s.parse::<u32>().ok());

    let artists = if primary_artist.is_empty() {
        Vec::new()
    } else {
        vec![primary_artist.clone()]
    };
    let cover = album_thumbnail
        .map(|u| u.to_string())
        .filter(|u| !u.is_empty());
    let album_id = synthesize_album_id(album_title, &primary_artist);
    Some(Track {
        id: super::yt_id(video_id.clone()),
        cover,
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

/// Anonymous-friendly POST against the YT browse endpoint. When
/// cookies is None / empty, omits Cookie + SAPISID auth so anonymous
/// YT mode can hit discover/album/artist endpoints. Private surfaces
/// (FEmusic_liked_playlists, the user's library) silently fall back
/// to a sign-in shelf the parsers will surface as empty.
async fn post(url: &str, body: &Value, cookies: &str) -> Result<Value, String> {
    let client = WEB_REMIX;
    let mut req = http_client()
        .post(url)
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    // Attach auth only when we have cookies that actually yield a
    // SAPISIDHASH. Empty cookies (anonymous mode) or a partial/expired
    // jar with no SAPISID both fall through to an anonymous request —
    // discover still returns generic recommendations rather than
    // hard-failing with "SAPISID missing".
    if !cookies.is_empty()
        && let Some(auth) = sapisid_hash(cookies, ORIGIN_YOUTUBE_MUSIC)
    {
        req = req.header("Cookie", cookies).header("Authorization", auth);
    }
    req.json(body)
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
    let sections = tab_section_contents(resp, "/contents/singleColumnBrowseResultsRenderer/tabs");
    let continuation = resp
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs")
        .and_then(|v| v.as_array())
        .and_then(|tabs| {
            tabs.iter().find_map(|tab| {
                first_continuation(
                    tab,
                    "/tabRenderer/content/sectionListRenderer/continuations",
                )
            })
        });
    DiscoverHome {
        shelves: sections.iter().filter_map(|s| parse_shelf(s)).collect(),
        continuation,
    }
}

fn parse_continuation(resp: &Value) -> DiscoverHome {
    let contents = resp
        .pointer("/continuationContents/sectionListContinuation/contents")
        .and_then(|v| v.as_array());
    let continuation = first_continuation(
        resp,
        "/continuationContents/sectionListContinuation/continuations",
    );
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
        .and_then(|h| runs_text(h, "/title/runs"))
        .unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    let strapline = header.and_then(|h| runs_text(h, "/strapline/runs"));
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
        is_song_list: false,
    })
}

fn parse_tile(item: &Value) -> Option<DiscoverItem> {
    let r = item.get("musicTwoRowItemRenderer")?;
    let title = runs_text(r, "/title/runs").unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    let subtitle = runs_text(r, "/subtitle/runs").unwrap_or_default();
    let thumbnail = best_thumbnail(r).map(normalize_yt_thumbnail);

    if let Some(video_id) = r
        .pointer("/navigationEndpoint/watchEndpoint/videoId")
        .and_then(|v| v.as_str())
    {
        return Some(DiscoverItem::Song(Box::new(build_song_track(
            video_id,
            &title,
            &subtitle,
            thumbnail.as_deref(),
        ))));
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

fn build_song_track(video_id: &str, title: &str, subtitle: &str, thumbnail: Option<&str>) -> Track {
    // Subtitle for songs/videos is typically "Artist • N views" — take
    // the first run as the primary artist; everything after the first
    // dot is metadata that doesn't belong in the artist field.
    let primary_artist = subtitle.split('•').next().unwrap_or("").trim().to_string();
    let artists = if primary_artist.is_empty() {
        Vec::new()
    } else {
        vec![primary_artist.clone()]
    };
    let cover = thumbnail.map(|u| u.to_string()).filter(|u| !u.is_empty());
    let album_id = synthesize_album_id("", &primary_artist);
    Track {
        id: super::yt_id(video_id),
        cover,
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

// ============================================================
//  Iterate-by-name helpers. Every positional /N/ pointer in this
//  module goes through one of these so the parser doesn't break the
//  moment YT reorders a column, a tab, or a run. Verified end-to-end
//  via yttools/parser_v2_probe against live home/album/artist
//  responses before porting.
// ============================================================

/// Join every `text` fragment in a runs array. Replaces
/// `.pointer("…/runs/0/text")` reads that silently drop multi-run text.
fn runs_text(v: &Value, pointer: &str) -> Option<String> {
    let arr = v.pointer(pointer).and_then(|x| x.as_array())?;
    let joined: String = arr
        .iter()
        .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
        .collect();
    (!joined.is_empty()).then_some(joined)
}

/// Scan a continuations array for any `nextContinuationData.continuation`
/// token. Replaces `…/continuations/0/nextContinuationData/continuation`.
fn first_continuation(v: &Value, pointer: &str) -> Option<String> {
    let arr = v.pointer(pointer).and_then(|x| x.as_array())?;
    for c in arr {
        if let Some(t) = c
            .pointer("/nextContinuationData/continuation")
            .and_then(|v| v.as_str())
        {
            return Some(t.to_string());
        }
    }
    None
}

/// Iterate every `tabs[].tabRenderer.content.sectionListRenderer.contents`
/// reachable from a tabs root. Replaces `tabs/0/tabRenderer/…`.
fn tab_section_contents<'a>(resp: &'a Value, tabs_pointer: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    let Some(tabs) = resp.pointer(tabs_pointer).and_then(|v| v.as_array()) else {
        return out;
    };
    for tab in tabs {
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
    out
}

#[derive(Debug, Clone)]
pub(crate) enum RowColumn {
    Title {
        text: String,
        video_id: Option<String>,
        playlist_id: Option<String>,
    },
    Artist {
        text: String,
    },
    Album {
        text: String,
    },
    Duration {
        secs: u64,
    },
    PlayCount,
    Other,
    Empty,
}

/// Classify each flex column on a `musicResponsiveListItemRenderer` by
/// what it actually carries, identifying columns by endpoint type or
/// text shape — NOT by position. Critical for artist Top Songs rows
/// where the column order is title/artist/play-count/album, not the
/// usual title/artist/album.
fn classify_flex_columns(row: &Value) -> Vec<RowColumn> {
    let mut out = Vec::new();
    let Some(cols) = row.get("flexColumns").and_then(|v| v.as_array()) else {
        return out;
    };
    for col in cols {
        let Some(runs) = col
            .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
            .and_then(|v| v.as_array())
        else {
            out.push(RowColumn::Empty);
            continue;
        };
        if runs.is_empty() {
            out.push(RowColumn::Empty);
            continue;
        }
        let text: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
            .collect();
        if text.trim().is_empty() {
            out.push(RowColumn::Empty);
            continue;
        }
        // Title check FIRST against runs[0] specifically — that's where
        // the watchEndpoint lives in every observed shape. Looking at
        // any run's navigationEndpoint (the prior `find_map` approach)
        // lost rows whose title was multi-run with an inline artist
        // mention: run[0] had the title text but no nav endpoint, and
        // run[1]'s UC… browseEndpoint won, tagging the column as Artist
        // and silently dropping the entire row.
        let first_nav = runs.first().and_then(|r| r.get("navigationEndpoint"));
        if let Some(nav) = first_nav
            && let Some(vid) = nav
                .pointer("/watchEndpoint/videoId")
                .and_then(|v| v.as_str())
        {
            let pid = nav
                .pointer("/watchEndpoint/playlistId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            out.push(RowColumn::Title {
                text,
                video_id: Some(vid.to_string()),
                playlist_id: pid,
            });
            continue;
        }
        // Artist / Album: any run carrying a typed browseEndpoint
        // wins. Also recognise the album column's OLAK5uy_… audio
        // playlist endpoint (some artist Top Songs rows link the
        // album cell to its playlist instead of the MPRE browseId).
        let mut classified = false;
        for r in runs {
            let Some(nav) = r.get("navigationEndpoint") else {
                continue;
            };
            if let Some(bid) = nav
                .pointer("/browseEndpoint/browseId")
                .and_then(|v| v.as_str())
            {
                if bid.starts_with("UC") {
                    out.push(RowColumn::Artist { text: text.clone() });
                    classified = true;
                    break;
                }
                if bid.starts_with("MPRE") {
                    out.push(RowColumn::Album { text: text.clone() });
                    classified = true;
                    break;
                }
            }
            if let Some(pid) = nav
                .pointer("/watchPlaylistEndpoint/playlistId")
                .or_else(|| nav.pointer("/watchEndpoint/playlistId"))
                .and_then(|v| v.as_str())
                && pid.starts_with("OLAK5uy_")
            {
                out.push(RowColumn::Album { text: text.clone() });
                classified = true;
                break;
            }
        }
        if classified {
            continue;
        }
        if let Some(secs) = parse_mm_ss(text.trim()) {
            out.push(RowColumn::Duration { secs });
        } else if is_play_count_text(&text) {
            out.push(RowColumn::PlayCount);
        } else {
            out.push(RowColumn::Other);
        }
    }
    out
}

fn is_play_count_text(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("play") || lower.contains("view") || lower.contains("listener")
}

/// Scan fixedColumns for an mm:ss text. Replaces
/// `/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text/runs/0/text`.
fn fixed_columns_duration(row: &Value) -> Option<u64> {
    let cols = row.get("fixedColumns").and_then(|v| v.as_array())?;
    for col in cols {
        // `let else continue` — a textless column (e.g. a like-toggle
        // fixedColumn before the duration column) must not abort the
        // whole scan; iterate until we find one that parses as mm:ss.
        let Some(text) = runs_text(col, "/musicResponsiveListItemFixedColumnRenderer/text/runs")
        else {
            continue;
        };
        if let Some(secs) = parse_mm_ss(text.trim()) {
            return Some(secs);
        }
    }
    None
}

/// Index-shelf row number (e.g. "1") — `index.runs[0].text` is a single
/// run by design; helper exists so future-us doesn't have to remember
/// that and re-introduce a /0 by hand.
fn row_index_text(row: &Value) -> Option<String> {
    runs_text(row, "/index/runs").map(|s| s.trim().to_string())
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
