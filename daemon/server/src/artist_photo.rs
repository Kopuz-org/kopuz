//! Artist photos for libraries that carry none of their own: files on disk or on a share
//! list no artists, so their photos are looked up on Deezer's public API, without an account.
//!
//! A name alone names strangers (Deezer lists three artists called Tarkan), so a photo is taken
//! from an album search whose title matches one of the artist's own albums, and only then from an
//! artist search that matches the name exactly.

use std::time::Duration;

const SEARCH_ALBUM: &str = "https://api.deezer.com/search/album";
const SEARCH_ARTIST: &str = "https://api.deezer.com/search/artist";
const TIMEOUT: Duration = Duration::from_secs(8);
/// How many of the artist's albums are searched for before the name is tried alone.
const ALBUM_TRIES: usize = 3;

/// The photo of the artist who made `albums`, by `name`. `Err` is a lookup that did not get an
/// answer, which is no reason to stop asking; `Ok(None)` is Deezer not knowing the artist.
pub async fn lookup(name: &str, albums: &[String]) -> Result<Option<String>, String> {
    let wanted = normalized(name);
    if wanted.is_empty() {
        return Ok(None);
    }
    for album in albums
        .iter()
        .filter(|album| !normalized(album).is_empty())
        .take(ALBUM_TRIES)
    {
        let found = search(SEARCH_ALBUM, &format!("{album} {name}")).await?;
        if let Some(photo) = album_match(&found, album) {
            return Ok(Some(photo));
        }
    }
    let found = search(SEARCH_ARTIST, name).await?;
    Ok(name_match(&found, &wanted))
}

async fn search(url: &str, query: &str) -> Result<serde_json::Value, String> {
    utils::http::client()
        .get(url)
        .query(&[("q", query), ("limit", "10")])
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?
        .error_for_status()
        .map_err(|error| error.without_url().to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.without_url().to_string())
}

/// The artist photo on the first album result whose title is `album`.
fn album_match(found: &serde_json::Value, album: &str) -> Option<String> {
    let wanted = normalized(album);
    found
        .get("data")?
        .as_array()?
        .iter()
        .filter(|hit| {
            hit.get("title").and_then(|t| t.as_str()).map(normalized) == Some(wanted.clone())
        })
        .find_map(|hit| photo(hit.get("artist")?))
}

/// The best-known artist whose name is exactly `wanted`, when it has a photo.
fn name_match(found: &serde_json::Value, wanted: &str) -> Option<String> {
    found
        .get("data")?
        .as_array()?
        .iter()
        .filter(|hit| {
            hit.get("name")
                .and_then(|n| n.as_str())
                .map(normalized)
                .as_deref()
                == Some(wanted)
        })
        .max_by_key(|hit| {
            hit.get("nb_fan")
                .and_then(|fans| fans.as_u64())
                .unwrap_or(0)
        })
        .and_then(photo)
}

/// The artist's largest picture, unless it is Deezer's placeholder, which has no image id in its path.
fn photo(artist: &serde_json::Value) -> Option<String> {
    let url = artist.get("picture_xl")?.as_str()?;
    (!url.is_empty() && !url.contains("/artist//")).then(|| url.to_string())
}

/// Letters and digits only, lowercased, with a trailing `(Deluxe Edition)` or `[Remastered]` dropped,
/// so a title or name compares the same however it is punctuated.
fn normalized(text: &str) -> String {
    let trimmed = text.trim();
    let base = match trimmed.find([' ', '\t']).and(trimmed.rfind(['(', '['])) {
        Some(at) if at > 0 => &trimmed[..at],
        _ => trimmed,
    };
    base.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artist(name: &str, fans: u64, id: &str) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "nb_fan": fans,
            "picture_xl": format!("https://cdn-images.dzcdn.net/images/artist/{id}/1000x1000.jpg"),
        })
    }

    #[test]
    fn a_title_or_name_compares_however_it_is_dressed() {
        assert_eq!(
            normalized("Late Registration (Deluxe Edition)"),
            "lateregistration"
        );
        assert_eq!(normalized("AC/DC"), "acdc");
        assert_eq!(normalized("いよわ"), "いよわ");
    }

    #[test]
    fn the_album_decides_which_artist_it_is() {
        let found = serde_json::json!({ "data": [
            { "title": "Late Orchestration", "artist": artist("Kanye West", 1, "a") },
            { "title": "Late Registration", "artist": artist("Kanye West", 1, "b") },
        ]});
        assert!(album_match(&found, "Late Registration").is_some_and(|url| url.contains("/b/")));
        assert_eq!(album_match(&found, "Graduation"), None);
    }

    #[test]
    fn a_name_takes_the_best_known_namesake_with_a_photo() {
        let found = serde_json::json!({ "data": [
            artist("Tarkan", 47, "small"),
            artist("Tarkan", 542_539, "big"),
            artist("Sezen Aksu", 1_187_519, "other"),
        ]});
        assert!(name_match(&found, "tarkan").is_some_and(|url| url.contains("/big/")));
        assert_eq!(name_match(&found, "nobody"), None);
    }

    #[test]
    fn the_placeholder_is_no_photo() {
        assert_eq!(photo(&artist("Tarkan", 1, "")), None);
    }
}
