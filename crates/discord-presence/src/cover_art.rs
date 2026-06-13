use serde::Deserialize;

const MUSICBRAINZ_API: &str = "https://musicbrainz.org/ws/2";
const COVER_ART_ARCHIVE: &str = "https://coverartarchive.org";
const ITUNES: &str = "https://itunes.apple.com/search";
pub const USER_AGENT: &str = concat!(
    "Kopuz/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/temidaradev/kopuz)"
);

fn build_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder().user_agent(USER_AGENT).build()
}

#[derive(Debug, Deserialize)]
struct ReleaseSearchResponse {
    releases: Option<Vec<ReleaseSearchResult>>,
}

#[derive(Debug, Deserialize)]
struct ReleaseSearchResult {
    id: String,
    score: Option<u8>,
}

pub fn cover_art_url(release_mbid: &str) -> String {
    format!("{}/release/{}/front", COVER_ART_ARCHIVE, release_mbid)
}

fn escape_lucene(input: &str) -> String {
    let special = [
        '\\', '+', '-', '!', '(', ')', ':', '^', '[', ']', '"', '{', '}', '~', '*', '?', '|', '&',
        '/',
    ];
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        if special.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

async fn search_release_mbid(
    artist: &str,
    album: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    if artist.is_empty() && album.is_empty() {
        return Ok(None);
    }

    let mut query_parts: Vec<String> = Vec::new();
    if !album.is_empty() {
        query_parts.push(format!("release:\"{}\"", escape_lucene(album)));
    }
    if !artist.is_empty() {
        query_parts.push(format!("artist:\"{}\"", escape_lucene(artist)));
    }
    let query = query_parts.join(" AND ");

    let client = build_client()?;
    let resp = client
        .get(format!("{}/release/", MUSICBRAINZ_API))
        .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "1")])
        .send()
        .await?;

    if !resp.status().is_success() {
        tracing::warn!("MusicBrainz search returned HTTP {}", resp.status());
        return Ok(None);
    }

    let body: ReleaseSearchResponse = resp.json().await?;
    if let Some(releases) = body.releases
        && let Some(first) = releases.first()
    {
        let score = first.score.unwrap_or(0);
        if score >= 80 {
            tracing::info!("MusicBrainz match: release={} (score={})", first.id, score);
            return Ok(Some(first.id.clone()));
        } else {
            tracing::info!(
                "MusicBrainz top result score {} too low (need >= 80)",
                score
            );
        }
    }

    Ok(None)
}

#[derive(Debug, Deserialize)]
struct ItunesSearchResponse {
    #[serde(rename = "resultCount")]
    result_count: u32,
    results: Vec<ItunesResult>,
}

#[derive(Debug, Deserialize)]
struct ItunesResult {
    #[serde(rename = "artworkUrl100")]
    artwork_url_100: Option<String>,
}

async fn resolve_via_itunes(
    artist: &str,
    album: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    if artist.is_empty() && album.is_empty() {
        return Ok(None);
    }

    let term = format!("{} {}", artist, album);
    let client = build_client()?;
    let resp = client
        .get(ITUNES)
        .query(&[("term", term.as_str()), ("entity", "album"), ("limit", "1")])
        .send()
        .await?;

    if !resp.status().is_success() {
        tracing::warn!("iTunes returned HTTP {}", resp.status());
        return Ok(None);
    }

    let body: ItunesSearchResponse = resp.json().await?;
    if body.result_count == 0 {
        return Ok(None);
    }

    if let Some(result) = body.results.first()
        && let Some(url) = &result.artwork_url_100
    {
        let hires = url.replace("100x100bb", "600x600bb");
        tracing::info!("iTunes match -> {}", hires);
        return Ok(Some(hires));
    }

    Ok(None)
}

async fn verify_cover_exists(
    release_mbid: &str,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let url = cover_art_url(release_mbid);
    let client = build_client()?;
    let resp = client.head(&url).send().await?;
    let status = resp.status();
    Ok(status.is_success() || status.is_redirection())
}

const COVER_META_KIND: &str = "cover_art";
const COVER_NEGATIVE_TTL_SECS: u64 = 7 * 24 * 60 * 60;

fn cover_cache_key(mbid: Option<&str>, artist: &str, album: &str) -> String {
    format!(
        "{}|{}|{}",
        mbid.unwrap_or(""),
        artist.to_lowercase(),
        album.to_lowercase()
    )
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read-through over [`resolve_cover_art_url`]: the MusicBrainz/iTunes result
/// (including "no cover", with a TTL) persists in the metadata cache so songs
/// aren't re-resolved across restarts.
pub async fn resolve_cover_art_url_cached(
    mbid: Option<&str>,
    artist: &str,
    album: &str,
) -> Option<String> {
    let key = cover_cache_key(mbid, artist, album);
    if let Some(handle) = utils::db_cache::get()
        && let Ok(Some(payload)) = handle.meta_get(&key, COVER_META_KIND).await
    {
        if let Some(url) = payload.strip_prefix("url:") {
            return Some(url.to_string());
        }
        if let Some(ts) = payload.strip_prefix("none:")
            && now_unix().saturating_sub(ts.parse().unwrap_or(0)) < COVER_NEGATIVE_TTL_SECS
        {
            return None;
        }
    }

    let resolved = resolve_cover_art_url(mbid, artist, album).await;
    if let Some(handle) = utils::db_cache::get() {
        let payload = match &resolved {
            Some(url) => format!("url:{url}"),
            None => format!("none:{}", now_unix()),
        };
        let _ = handle.meta_put(&key, COVER_META_KIND, &payload).await;
    }
    resolved
}

pub async fn resolve_cover_art_url(
    mbid: Option<&str>,
    artist: &str,
    album: &str,
) -> Option<String> {
    if let Some(id) = mbid
        && !id.is_empty()
    {
        match verify_cover_exists(id).await {
            Ok(true) => {
                let url = cover_art_url(id);
                tracing::info!("Resolved via embedded MBID -> {}", url);
                return Some(url);
            }
            Ok(false) => {
                tracing::warn!("Embedded MBID {} has no front cover, falling back", id)
            }
            Err(e) => tracing::warn!("Error verifying MBID {}: {}", id, e),
        }
    }

    match search_release_mbid(artist, album).await {
        Ok(Some(release_id)) => match verify_cover_exists(&release_id).await {
            Ok(true) => {
                let url = cover_art_url(&release_id);
                tracing::info!("Resolved via MusicBrainz search -> {}", url);
                return Some(url);
            }
            Ok(false) => tracing::warn!("Release {} has no front cover", release_id),
            Err(e) => tracing::warn!("Error verifying release {}: {}", release_id, e),
        },
        Ok(None) => tracing::info!(
            "No MusicBrainz match for artist=\"{}\" album=\"{}\"",
            artist,
            album
        ),
        Err(e) => tracing::warn!("MusicBrainz search failed: {}", e),
    }

    // Fallback: iTunes
    match resolve_via_itunes(artist, album).await {
        Ok(Some(url)) => return Some(url),
        Ok(None) => tracing::info!("No iTunes match"),
        Err(e) => tracing::warn!("iTunes error: {}", e),
    }

    tracing::info!(
        "All sources exhausted for artist=\"{}\" album=\"{}\"",
        artist,
        album
    );
    None
}
