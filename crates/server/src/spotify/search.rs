//! Search through the web player's GraphQL gateway (`api-partner`), which
//! is what the desktop client itself uses. The gateway only answers
//! persisted queries, so the request names an operation and the SHA-256 of
//! its text; the hash comes from the web player's bundle and changes when
//! Spotify ships a new one. When that happens the gateway answers
//! `PersistedQueryNotFound`, which surfaces as a search error rather than
//! wrong results.

use librespot_core::Session;
use serde_json::{Value, json};

const GATEWAY: &str = "https://api-partner.spotify.com/pathfinder/v1/query";
/// `searchDesktop` from `xpui-routes-search`, web-player build 5cec5e91.
const SEARCH_DESKTOP: &str = "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03";

fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Tracks and albums matching `query`, as the web player's search shows
/// them.
pub async fn search(
    session: &Session,
    query: &str,
) -> Result<(Vec<reader::Track>, Vec<reader::Album>), String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let token = session
        .login5()
        .auth_token()
        .await
        .map_err(|error| format!("search token: {error}"))?
        .access_token;
    let client_token = session
        .spclient()
        .client_token()
        .await
        .map_err(|error| format!("search client token: {error}"))?;
    let body = json!({
        "operationName": "searchDesktop",
        "variables": {
            "searchTerm": query,
            "offset": 0,
            "limit": 20,
            "numberOfTopResults": 5,
            "includeAudiobooks": false,
            "includeArtistHasConcertsField": false,
            "includePreReleases": false,
            "includeLocalConcertsField": false,
            "includeAuthors": false,
        },
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": SEARCH_DESKTOP } },
    });
    let resp = client()
        .post(GATEWAY)
        .bearer_auth(token)
        .header("client-token", client_token)
        .header("app-platform", "WebPlayer")
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("search: {error}"))?;
    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|error| format!("search ({status}): {error}"))?;
    if let Some(message) = json["errors"][0]["message"].as_str() {
        return Err(format!("Spotify search refused the query: {message}"));
    }
    if !status.is_success() {
        return Err(format!("search failed ({status})"));
    }
    Ok(parse_search(&json))
}

fn parse_search(json: &Value) -> (Vec<reader::Track>, Vec<reader::Album>) {
    let results = &json["data"]["searchV2"];
    let tracks = results["tracksV2"]["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| parse_track(&item["item"]["data"]))
                .collect()
        })
        .unwrap_or_default();
    let albums = results["albumsV2"]["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| parse_album(&item["data"]))
                .collect()
        })
        .unwrap_or_default();
    (tracks, albums)
}

fn id_of(uri: &Value, kind: &str) -> Option<String> {
    uri.as_str()?
        .strip_prefix(&format!("spotify:{kind}:"))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn artist_names(data: &Value) -> Vec<String> {
    data["artists"]["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|artist| artist["profile"]["name"].as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The largest of a `coverArt.sources` list.
fn cover_of(data: &Value) -> Option<String> {
    data["coverArt"]["sources"]
        .as_array()?
        .iter()
        .max_by_key(|source| source["width"].as_u64().unwrap_or(0))
        .and_then(|source| source["url"].as_str())
        .map(str::to_string)
}

fn parse_track(data: &Value) -> Option<reader::Track> {
    let id = id_of(&data["uri"], "track")?;
    let artists = artist_names(data);
    let album = &data["albumOfTrack"];
    Some(reader::Track {
        id: reader::models::TrackId::Server {
            service: config::MusicService::Spotify,
            item_id: id,
        },
        cover: cover_of(album),
        album_id: id_of(&album["uri"], "album").unwrap_or_default(),
        title: data["name"].as_str().unwrap_or_default().to_string(),
        artist: artists.first().cloned().unwrap_or_default(),
        album: album["name"].as_str().unwrap_or_default().to_string(),
        duration: data["duration"]["totalMilliseconds"].as_u64().unwrap_or(0) / 1000,
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

fn parse_album(data: &Value) -> Option<reader::Album> {
    let id = id_of(&data["uri"], "album")?;
    Some(reader::Album {
        id,
        title: data["name"].as_str().unwrap_or_default().to_string(),
        artist: artist_names(data).into_iter().next().unwrap_or_default(),
        genre: String::new(),
        year: data["date"]["year"]
            .as_u64()
            .and_then(|y| u16::try_from(y).ok())
            .unwrap_or(0),
        cover_path: None,
        manual_cover: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_web_players_search_shape() {
        let json = json!({
            "data": { "searchV2": {
                "tracksV2": { "items": [ { "item": { "data": {
                    "uri": "spotify:track:4PTG3Z6ehGkBFwjybzWkR8",
                    "name": "Never Gonna Give You Up",
                    "duration": { "totalMilliseconds": 213573 },
                    "artists": { "items": [ { "profile": { "name": "Rick Astley" } } ] },
                    "albumOfTrack": {
                        "uri": "spotify:album:6eUW0wxWtzkFdaEFsTJto6",
                        "name": "Whenever You Need Somebody",
                        "coverArt": { "sources": [
                            { "width": 64, "url": "https://i/small" },
                            { "width": 640, "url": "https://i/large" }
                        ] }
                    }
                } } } ] },
                "albumsV2": { "items": [ { "data": {
                    "uri": "spotify:album:0W1gLfsJ17r7hRT7jW7KUT",
                    "name": "An Album",
                    "date": { "year": 2022 },
                    "artists": { "items": [ { "profile": { "name": "Someone" } } ] },
                    "coverArt": { "sources": [ { "width": 300, "url": "https://i/a" } ] }
                } } ] }
            } }
        });
        let (tracks, albums) = parse_search(&json);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id.key(), "4PTG3Z6ehGkBFwjybzWkR8");
        assert_eq!(tracks[0].duration, 213);
        assert_eq!(tracks[0].artist, "Rick Astley");
        assert_eq!(tracks[0].album_id, "6eUW0wxWtzkFdaEFsTJto6");
        assert_eq!(tracks[0].cover.as_deref(), Some("https://i/large"));
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].year, 2022);
        assert_eq!(albums[0].artist, "Someone");
    }
}
