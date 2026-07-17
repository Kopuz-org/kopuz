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
// `params` value for the "Albums" tab. Restricts hits to album rows whose
// nav endpoint browseId begins with `MPRE…`, what we need to resolve a
// title+artist back to its album browse id (see `resolve_album_browse_id`).
const ALBUMS_FILTER: &str = "EgWKAQIYAWoMEAMQBBAJEAoQDhAV";

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
pub async fn music_search_tracks(query: &str, cookies: Option<&str>) -> Result<Vec<Track>, String> {
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
    Ok(pick_artist_row(&resp, query).and_then(|row| {
        row.pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }))
}

/// Resolve an album title + artist to its YT Music album browse id
/// (`MPRE…`). The local library stores YT albums under a title+artist hash
/// with no browse id, so the album page resolves it on demand to fetch the
/// album's full track list (see [`fetch_album`](super::discover::fetch_album)).
/// Searches the Albums tab for "<album> <artist>" and takes the first
/// `MPRE…` browseId — the top-ranked match. Returns None if nothing matched.
#[tracing::instrument(name = "yt.resolve_album", skip(cookies), fields(album = %album))]
pub async fn resolve_album_browse_id(
    album: &str,
    artist: &str,
    cookies: Option<&str>,
) -> Result<Option<String>, String> {
    if album.trim().is_empty() {
        return Ok(None);
    }
    let query = if artist.trim().is_empty() {
        album.to_string()
    } else {
        format!("{album} {artist}")
    };
    let http = super::innertube::http_client();
    let resp = do_search_raw(http, &query, Some(ALBUMS_FILTER), cookies).await?;
    Ok(walk_first_album_browse_id(&resp))
}

/// Recursively walk the JSON for the first browseEndpoint pointing at an
/// `MPRE…` album. The albums filter restricts results to album rows so the
/// first hit is the top-ranked match.
fn walk_first_album_browse_id(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            if let Some(ep) = map.get("browseEndpoint")
                && let Some(bid) = ep.get("browseId").and_then(|x| x.as_str())
                && bid.starts_with("MPRE")
            {
                return Some(bid.to_string());
            }
            for child in map.values() {
                if let Some(found) = walk_first_album_browse_id(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => arr.iter().find_map(walk_first_album_browse_id),
        _ => None,
    }
}

/// The best-matching artist's avatar URL for `name` from the YT Music
/// "Artists" tab. Powers the Artists grid so its circular photos are the real
/// YT artist images and nothing else. Returns None when no artist row
/// survived the row checks — the grid then falls back to an album cover,
/// which beats confidently showing someone else's face.
#[tracing::instrument(name = "yt.artist_image", skip(cookies), fields(name = %name))]
pub async fn resolve_artist_image(
    name: &str,
    cookies: Option<&str>,
) -> Result<Option<String>, String> {
    if name.trim().is_empty() {
        return Ok(None);
    }
    let http = super::innertube::http_client();
    let resp = do_search_raw(http, name, Some(ARTISTS_FILTER), cookies).await?;
    Ok(pick_artist_row(&resp, name).and_then(best_thumbnail))
}

/// The search row that best represents artist `want`:
///
/// - only rows whose OWN navigation endpoint browses to a `UC…` channel typed
///   `MUSIC_PAGE_TYPE_ARTIST` count. Checking for a `UC…` id anywhere inside
///   the row (as this used to) also matched playlist/profile rows, whose
///   per-artist title runs carry channel links — donating a playlist cover as
///   an "artist photo".
/// - aggregate auto-channels are skipped: they title themselves with a pile
///   of names ("A、B、C、…"), carry a generic mosaic avatar, and rank first
///   for many different queries — the "several tiles share one wrong photo"
///   failure.
/// - a row whose folded title equals the query wins; otherwise the first
///   surviving row does. The fallback is deliberate: YT restyles names the
///   library spells differently (romanization, casing, spacing — "さんみゅ~"
///   is titled "Sunmyu" with `hl=en`), so demanding a title match would strip
///   photos from every such artist.
fn pick_artist_row<'a>(resp: &'a Value, want: &str) -> Option<&'a Value> {
    let mut rows = Vec::new();
    collect_search_rows(resp, &mut rows);
    let candidates: Vec<&Value> = rows
        .into_iter()
        .filter(|r| is_artist_row(r))
        .filter(|r| row_title(r).is_some_and(|t| !looks_like_artist_pile(t)))
        .collect();
    let want = fold_artist_name(want);
    candidates
        .iter()
        .find(|r| row_title(r).is_some_and(|t| fold_artist_name(t) == want))
        .or(candidates.first())
        .copied()
}

/// Every `musicResponsiveListItemRenderer` in the response, in document order.
fn collect_search_rows<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(map) => {
            if let Some(row) = map.get("musicResponsiveListItemRenderer") {
                out.push(row);
            }
            for child in map.values() {
                collect_search_rows(child, out);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                collect_search_rows(child, out);
            }
        }
        _ => {}
    }
}

/// Whether the row itself navigates to an artist page (not merely contains a
/// channel link somewhere inside).
fn is_artist_row(row: &Value) -> bool {
    let Some(ep) = row.pointer("/navigationEndpoint/browseEndpoint") else {
        return false;
    };
    ep.get("browseId")
        .and_then(|v| v.as_str())
        .is_some_and(|b| b.starts_with("UC"))
        && ep
            .pointer(
                "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
            )
            .and_then(|v| v.as_str())
            == Some("MUSIC_PAGE_TYPE_ARTIST")
}

fn row_title(row: &Value) -> Option<&str> {
    row.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
        .and_then(|v| v.as_str())
}

/// Aggregate auto-channels list several artists in their title; three or more
/// comma segments marks one. (Two is a legitimate duo credit.)
fn looks_like_artist_pile(title: &str) -> bool {
    title.split(['、', ',']).count() >= 3
}

/// Fold a name for comparison: lowercase, whitespace removed. YT restyles
/// casing and spacing freely — "FujiwaraHagane" for "Fujiwara Hagane",
/// "D-Cell" for "D-CELL".
pub(super) fn fold_artist_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// Split an unlinked artist credit into individual names: "feat."/"ft."
/// (space-prefixed, any case) and commas separate; a bare "&" does NOT —
/// "MYTH & ROID" is one artist. Leading "&"s left by a ", & C" tail are
/// stripped. A plain single name passes through unchanged.
fn split_unlinked_artists(text: &str) -> Vec<String> {
    let mut joined = text.to_string();
    for sep in [" feat.", " Feat.", " FEAT.", " ft.", " Ft.", " FT."] {
        joined = joined.replace(sep, ",");
    }
    joined
        .split([',', '、'])
        .map(|part| part.trim().trim_start_matches('&').trim())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect()
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
        body.as_object_mut()
            .unwrap()
            .insert("params".into(), json!(p));
    }
    let mut req = http
        .post(format!(
            "{ORIGIN_YT_MUSIC}/youtubei/v1/search?prettyPrint=false"
        ))
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
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
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
    t.id.key().into_owned()
}

fn parse_card_shelf(card: &Value) -> Option<ParsedRow> {
    let endpoint = card.pointer("/onTap/watchEndpoint")?;
    let video_id = endpoint
        .get("videoId")
        .and_then(|v| v.as_str())?
        .to_string();
    // Validate it's a music card (skip non-music results) — the value itself is
    // no longer needed now that fields are slotted by link.
    endpoint
        .pointer("/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
        .and_then(|v| v.as_str())
        .and_then(MusicVideoType::from_str)?;

    let title = card
        .pointer("/title/runs/0/text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Subtitle is "Kind • Artist • [Album|Views|Year] • [...]". Slot it by link
    // (artist → UC, album → MPRE) instead of position: the kind label and an
    // absent album otherwise shift "Views"/"Year" into the album field.
    let subtitle = runs_with_browse(card.pointer("/subtitle/runs"));
    let (album, album_browse_id) = subtitle
        .iter()
        .find(|(_, b)| b.as_deref().is_some_and(|b| b.starts_with("MPRE")))
        .map(|(t, b)| (Some(t.clone()), b.clone()))
        .unwrap_or((None, None));
    let artist = subtitle
        .iter()
        .find(|(_, b)| b.as_deref().is_some_and(|b| b.starts_with("UC")))
        .map(|(t, _)| t.clone())
        // No channel link: skip the leading kind label, take the next token.
        .or_else(|| subtitle.get(1).map(|(t, _)| t.clone()))
        .unwrap_or_default();

    let thumbnail_url = card
        .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .max_by_key(|t| t.get("width").and_then(|v| v.as_u64()).unwrap_or(0))
        })
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
        album_browse_id,
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
        Some(parse_playlist_track(
            row,
            video_id,
            title,
            mvt,
            thumbnail_url,
        ))
    } else {
        Some(parse_search_row(row, video_id, title, thumbnail_url))
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
    thumbnail_url: Option<String>,
) -> ParsedRow {
    // flex[1] packs "artist[s] • [album|view-count] • duration" but the slots
    // are NOT positionally stable: some rows omit the album, some shelves (the
    // unfiltered "top" results) append a trailing run, and `mvt.has_album()`
    // lies for album-typed tracks that carry no album. Positional popping
    // therefore mis-slotted fields (duration landing in album, duration 0).
    //
    // Classify by each run's link instead: an album run links to an `MPRE…`
    // browse id, an artist run to a `UC…` channel. Duration is the m:ss run.
    let runs = pick_runs_with_browse(row, 1);
    let duration = runs
        .iter()
        .find(|(t, _)| looks_like_duration(t))
        .and_then(|(t, _)| parse_mm_ss(t))
        .unwrap_or(0);
    let (album, album_browse_id) = runs
        .iter()
        .find(|(_, b)| b.as_deref().is_some_and(|b| b.starts_with("MPRE")))
        .map(|(t, b)| (Some(t.clone()), b.clone()))
        .unwrap_or((None, None));
    let mut artists: Vec<String> = runs
        .iter()
        .filter(|(_, b)| b.as_deref().is_some_and(|b| b.starts_with("UC")))
        .map(|(t, _)| t.clone())
        .collect();
    if artists.is_empty() {
        // Unlinked artist text (no channel run). Fall back to the first run
        // that isn't the duration, album, or — for albumless rows — the
        // view-count tail. The leading run is the artist in every observed
        // shape. Unlike channel-linked runs, this text can be a joined credit
        // ("INABAKUMORI feat. Kaai Yuki") — split it, or the whole credit
        // becomes a phantom artist of its own in the Artists grid.
        artists = runs
            .iter()
            .map(|(t, _)| t.clone())
            .filter(|t| !looks_like_duration(t))
            .filter(|t| Some(t) != album.as_ref())
            .take(1)
            .flat_map(|t| split_unlinked_artists(&t))
            .collect();
    }

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

/// flex-column runs paired with their `navigationEndpoint` browse id (album →
/// `MPRE…`, artist → `UC…`), separators and empty runs dropped. Lets the row
/// parser slot fields by link rather than by fragile position.
fn pick_runs_with_browse(row: &Value, col: usize) -> Vec<(String, Option<String>)> {
    runs_with_browse(
        row.get("flexColumns")
            .and_then(|c| c.as_array())
            .and_then(|cs| cs.get(col))
            .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")),
    )
}

/// A `text/runs` array → `(text, browse_id)` pairs, separators and empty runs
/// dropped. Shared by the flex-column rows and the top-result card subtitle.
fn runs_with_browse(runs: Option<&Value>) -> Vec<(String, Option<String>)> {
    runs.and_then(|v| v.as_array())
        .map(|runs| {
            runs.iter()
                .filter_map(|r| {
                    let text = r.get("text").and_then(|t| t.as_str())?;
                    if is_separator(text) || text.trim().is_empty() {
                        return None;
                    }
                    let browse_id = r
                        .pointer("/navigationEndpoint/browseEndpoint/browseId")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    Some((text.to_string(), browse_id))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parsed_to_track(p: ParsedRow) -> Track {
    let primary_artist = p.artists.first().cloned().unwrap_or_default();
    let album = p.album.clone().unwrap_or_default();
    let album_id = match p.album_browse_id {
        Some(id) => format!("{SOURCE_PREFIX}:album:{id}"),
        None => synthesize_album_id(&album, &primary_artist),
    };
    let cover = p.thumbnail_url.filter(|u| !u.is_empty());

    Track {
        id: super::yt_id(p.video_id),
        cover,
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
        .and_then(|c| {
            c.pointer(&format!(
                "/musicResponsiveListItemFlexColumnRenderer/text/runs/{run}/text"
            ))
        })
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// A duration cell looks like `m:ss` / `h:mm:ss` — must contain a colon so a
/// bare year ("2009") or numeric token can't masquerade as a duration.
fn looks_like_duration(s: &str) -> bool {
    s.contains(':') && parse_mm_ss(s).is_some()
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
        .pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents")
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

/// The real `MPRE…` album browse id carried by an album id, if any. Handles
/// both the raw `MPRE…` form (Discover passes that straight to navigation) and
/// the `ytmusic:album:MPRE…` form synthesized for search/track rows. Returns
/// None for synthesized hash ids and the `singles` bucket.
pub fn album_browse_id(id: &str) -> Option<String> {
    let token = id.rsplit(':').next().unwrap_or(id);
    token.starts_with("MPRE").then(|| token.to_string())
}

/// `(album, artist)` recovered from a synthesized album id
/// (`ytmusic:album:<hex(album|artist)>`). Lets the album page resolve a browse
/// id on demand for albums that came back from search without an `MPRE` link.
/// None for browse-id ids and the `singles` bucket.
pub fn synth_album_parts(id: &str) -> Option<(String, String)> {
    let token = id.rsplit(':').next()?;
    if token.starts_with("MPRE") || token == "singles" {
        return None;
    }
    let bytes = hex::decode(token).ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    let (album, artist) = decoded.split_once('|').unwrap_or((decoded.as_str(), ""));
    (!album.is_empty()).then(|| (album.to_string(), artist.to_string()))
}

fn parse_mm_ss(s: &str) -> Option<u64> {
    let mut parts = s.split(':').rev();
    let secs: u64 = parts.next()?.parse().ok()?;
    let mins: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let hours: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some(hours * 3600 + mins * 60 + secs)
}

#[cfg(test)]
mod artist_split_tests {
    use super::split_unlinked_artists;

    #[test]
    fn splits_feat_and_commas_keeps_ampersand_names() {
        assert_eq!(
            split_unlinked_artists("INABAKUMORI feat. Kaai Yuki"),
            vec!["INABAKUMORI", "Kaai Yuki"]
        );
        // No space after "feat." — as YT actually delivers it.
        assert_eq!(
            split_unlinked_artists("かいりきベア feat.缶缶"),
            vec!["かいりきベア", "缶缶"]
        );
        assert_eq!(
            split_unlinked_artists("塞壬唱片-MSR, 真名辺あや, & TMKJ"),
            vec!["塞壬唱片-MSR", "真名辺あや", "TMKJ"]
        );
        // "&" alone never splits, and "ft." needs its leading space — names
        // like "Swift." must not be cut mid-word.
        assert_eq!(split_unlinked_artists("MYTH & ROID"), vec!["MYTH & ROID"]);
        assert_eq!(
            split_unlinked_artists("Taylor Swift."),
            vec!["Taylor Swift."]
        );
        assert_eq!(split_unlinked_artists("Reol"), vec!["Reol"]);
    }
}
