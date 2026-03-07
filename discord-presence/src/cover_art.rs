use serde::Deserialize;

const MUSICBRAINZ_API: &str = "https://musicbrainz.org/ws/2";
const COVER_ART_ARCHIVE: &str = "https://coverartarchive.org";
const USER_AGENT: &str = "Rusic/0.3.2 (https://github.com/temidaradev/rusic)";

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

fn build_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder().user_agent(USER_AGENT).build()
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
        println!(
            "[cover_art] MusicBrainz search returned HTTP {}",
            resp.status()
        );
        return Ok(None);
    }

    let body: ReleaseSearchResponse = resp.json().await?;

    if let Some(releases) = body.releases {
        if let Some(first) = releases.first() {
            let score = first.score.unwrap_or(0);
            if score >= 80 {
                println!(
                    "[cover_art] MusicBrainz match: release={} (score={})",
                    first.id, score
                );
                return Ok(Some(first.id.clone()));
            } else {
                println!(
                    "[cover_art] MusicBrainz top result score {} too low (need >= 80)",
                    score
                );
            }
        }
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
pub async fn resolve_cover_art_url(
    mbid: Option<&str>,
    artist: &str,
    album: &str,
) -> Option<String> {
    if let Some(id) = mbid {
        if !id.is_empty() {
            match verify_cover_exists(id).await {
                Ok(true) => {
                    let url = cover_art_url(id);
                    println!("[cover_art] Resolved via embedded MBID -> {}", url);
                    return Some(url);
                }
                Ok(false) => {
                    println!(
                        "[cover_art] Embedded MBID {} has no front cover, falling back to search",
                        id
                    );
                }
                Err(e) => {
                    println!("[cover_art] Error verifying MBID {}: {}", id, e);
                }
            }
        }
    }

    match search_release_mbid(artist, album).await {
        Ok(Some(release_id)) => match verify_cover_exists(&release_id).await {
            Ok(true) => {
                let url = cover_art_url(&release_id);
                println!("[cover_art] Resolved via search -> {}", url);
                Some(url)
            }
            Ok(false) => {
                println!(
                    "[cover_art] Release {} found but has no front cover",
                    release_id
                );
                None
            }
            Err(e) => {
                println!("[cover_art] Error verifying release {}: {}", release_id, e);
                None
            }
        },
        Ok(None) => {
            println!(
                "[cover_art] No match for artist=\"{}\" album=\"{}\"",
                artist, album
            );
            None
        }
        Err(e) => {
            println!("[cover_art] MusicBrainz search failed: {}", e);
            None
        }
    }
}
