//! Library and catalogue reads over the endpoints the desktop client uses
//! itself, through the librespot session's `spclient`.
//!
//! The public Web API is not an option here: Spotify throttles it for the
//! desktop client id every librespot-based player shares, and a token from
//! that id gets an instant 429. The client's own surfaces are not throttled
//! that way. What they are:
//!
//! - the **collection** (`/collection/collection/{user}`) — liked songs and
//!   saved albums as ids with timestamps, read as JSON; a like is a delta
//!   through `/collection/v2/write`;
//! - the **rootlist** (`/playlist/v2/user/{user}/rootlist`) and a playlist's
//!   own `SelectedListContent` — playlists and their entries;
//! - **extended metadata** — the `Track` protobufs for a batch of ids, which
//!   is how a list of ids becomes titles, artists, durations and covers;
//! - **recently played** (`/recently-played/v3`) — the contexts the account
//!   listened to lately, for the home page.
//!
//! Everything maps into `reader::Track` / `reader::Album` through
//! [`track_from`] and [`album_from`], with covers as `i.scdn.co` URLs the
//! cover seam passes straight through.

use std::collections::HashMap;

use base64::Engine as _;
use config::MusicService;
use librespot_core::{Session, SpotifyId, SpotifyUri};
use librespot_metadata::image::{ImageSize, Images};
use librespot_metadata::{Album, Metadata, Playlist, Track as SpotifyTrack};
use librespot_protocol::extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery};
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::playlist4_external::SelectedListContent;
use protobuf::{EnumOrUnknown, Message};
use reader::Track;
use reader::models::TrackId;

use crate::source::RemoteAlbum;
use crate::ytmusic::discover::{DiscoverHome, DiscoverItem, DiscoverShelf};

const IMAGE_URL: &str = "https://i.scdn.co/image/";
/// Ids per extended-metadata request. The client sends batches of this
/// order; larger ones start being refused.
const BATCH: usize = 50;
/// Rootlist page. Folders count as items, so the request over-asks a bit.
const ROOTLIST_PAGE: usize = 200;

/// The cover URL for the largest of an album's images.
pub fn image_url(images: &Images) -> Option<String> {
    let best = images
        .iter()
        .find(|image| image.size == ImageSize::LARGE)
        .or_else(|| images.iter().find(|image| image.size == ImageSize::DEFAULT))
        .or_else(|| images.first())?;
    Some(format!("{IMAGE_URL}{}", best.id))
}

fn base62(uri: &SpotifyUri) -> String {
    uri.to_id().unwrap_or_default()
}

fn track_uri(id: &str) -> Result<SpotifyUri, String> {
    SpotifyUri::from_uri(&format!("spotify:track:{id}"))
        .map_err(|error| format!("not a Spotify track id: {error}"))
}

fn album_uri(id: &str) -> Result<SpotifyUri, String> {
    SpotifyUri::from_uri(&format!("spotify:album:{id}"))
        .map_err(|error| format!("not a Spotify album id: {error}"))
}

/// A `reader::Track` from the client's track protobuf. `None` for a track
/// with no id, which the metadata service does return for removed items.
pub fn track_from(track: &SpotifyTrack) -> Option<Track> {
    let id = base62(&track.id);
    if id.is_empty() {
        return None;
    }
    let artists: Vec<String> = track
        .artists
        .iter()
        .map(|artist| artist.name.clone())
        .filter(|name| !name.is_empty())
        .collect();
    let cover = image_url(&track.album.covers).or_else(|| image_url(&track.album.cover_group));
    Some(Track {
        id: TrackId::Server {
            service: MusicService::Spotify,
            item_id: id,
        },
        cover,
        album_id: base62(&track.album.id),
        title: track.name.clone(),
        artist: artists.first().cloned().unwrap_or_default(),
        album: track.album.name.clone(),
        duration: (track.duration.max(0) as u64) / 1000,
        khz: 0,
        bitrate: 0,
        track_number: (track.number > 0).then_some(track.number as u32),
        disc_number: (track.disc_number > 0).then_some(track.disc_number as u32),
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists,
    })
}

fn album_from(album: &Album) -> reader::Album {
    reader::Album {
        id: base62(&album.id),
        title: album.name.clone(),
        artist: album
            .artists
            .first()
            .map(|artist| artist.name.clone())
            .unwrap_or_default(),
        genre: String::new(),
        year: u16::try_from(album.date.as_utc().year()).unwrap_or(0),
        cover_path: None,
        manual_cover: false,
    }
}

/// Full track metadata for `uris`, in the same order, skipping ids the
/// service has nothing for.
pub async fn tracks(session: &Session, uris: &[SpotifyUri]) -> Result<Vec<Track>, String> {
    let mut by_uri: HashMap<String, Track> = HashMap::with_capacity(uris.len());
    for chunk in uris.chunks(BATCH) {
        let request = BatchedEntityRequest {
            entity_request: chunk
                .iter()
                .filter_map(|uri| uri.to_uri().ok())
                .map(|entity_uri| EntityRequest {
                    entity_uri,
                    query: vec![ExtensionQuery {
                        extension_kind: EnumOrUnknown::new(ExtensionKind::TRACK_V4),
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let response = session
            .spclient()
            .get_extended_metadata(request)
            .await
            .map_err(|error| format!("track metadata: {error}"))?;
        for array in &response.extended_metadata {
            for data in &array.extension_data {
                let Some(payload) = data.extension_data.as_ref() else {
                    continue;
                };
                let Ok(message) =
                    librespot_protocol::metadata::Track::parse_from_bytes(&payload.value)
                else {
                    continue;
                };
                if let Some(track) = SpotifyTrack::try_from(&message)
                    .ok()
                    .as_ref()
                    .and_then(track_from)
                {
                    by_uri.insert(data.entity_uri.clone(), track);
                }
            }
        }
    }
    Ok(uris
        .iter()
        .filter_map(|uri| uri.to_uri().ok())
        .filter_map(|uri| by_uri.remove(&uri))
        .collect())
}

/// One entry of the account's collection: a liked track or a saved album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionItem {
    pub kind: CollectionKind,
    pub id: SpotifyId,
    pub added_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionKind {
    Track,
    Album,
}

/// A call on the client's own API, authenticated the way `spclient` does it
/// but without the `product=0&country=…&salt=…` query `spclient().request`
/// appends: `product=0` makes the collection answer as it would for a free
/// account, which is empty. The 404 an untouched collection gets is
/// distinguishable for callers that want an empty list instead.
async fn client_request(
    session: &Session,
    method: http::Method,
    endpoint: &str,
    content_type: Option<&'static str>,
    body: Vec<u8>,
) -> Result<bytes::Bytes, String> {
    let base = session
        .spclient()
        .base_url()
        .await
        .map_err(|error| format!("spclient: {error}"))?;
    let token = session
        .login5()
        .auth_token()
        .await
        .map_err(|error| format!("spclient token: {error}"))?;
    let mut request = http::Request::builder()
        .method(method)
        .uri(format!("{base}{endpoint}"))
        .header(http::header::ACCEPT, "application/json")
        .header(http::header::CONTENT_LENGTH, body.len())
        .header(
            http::header::AUTHORIZATION,
            format!("{} {}", token.token_type, token.access_token),
        );
    if let Some(content_type) = content_type {
        request = request.header(http::header::CONTENT_TYPE, content_type);
    }
    if let Ok(client_token) = session.spclient().client_token().await {
        request = request.header(librespot_core::spclient::CLIENT_TOKEN, client_token);
    }
    let request = request
        .body(bytes::Bytes::from(body))
        .map_err(|error| format!("spclient request: {error}"))?;
    session
        .http_client()
        .request_body(request)
        .await
        .map_err(|error| error.to_string())
}

/// The whole collection, newest first. An account that never liked
/// anything gets a 404 here, which is an empty list, not an error.
pub async fn collection(session: &Session) -> Result<Vec<CollectionItem>, String> {
    let user = session.username();
    let endpoint = format!("/collection/collection/{user}?format=json");
    let body = match client_request(session, http::Method::GET, &endpoint, None, Vec::new()).await {
        Ok(body) => body,
        Err(error) if error.contains("404") => return Ok(Vec::new()),
        Err(error) => return Err(format!("collection: {error}")),
    };
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|error| format!("collection: {error}"))?;
    let mut items: Vec<CollectionItem> = json["item"]
        .as_array()
        .map(|items| items.iter().filter_map(parse_collection_item).collect())
        .unwrap_or_default();
    items.sort_by_key(|item| std::cmp::Reverse(item.added_at));
    Ok(items)
}

fn parse_collection_item(item: &serde_json::Value) -> Option<CollectionItem> {
    let kind = match item["type"].as_str()? {
        "TRACK" => CollectionKind::Track,
        "ALBUM" => CollectionKind::Album,
        _ => return None,
    };
    let raw = base64::engine::general_purpose::STANDARD
        .decode(item["identifier"].as_str()?)
        .ok()?;
    let id = SpotifyId::from_raw(&raw).ok()?;
    Some(CollectionItem {
        kind,
        id,
        added_at: item["added_at"].as_u64().unwrap_or(0),
    })
}

/// Like or unlike one track through the collection's v2 write, which is a
/// delta: the request names the one item and whether it was added or
/// removed. (The legacy JSON endpoint's `PUT` replaces the whole set, so
/// it is never used for writes.)
pub async fn set_liked(session: &Session, track_id: &str, on: bool) -> Result<(), String> {
    const CONTENT: &str = "application/vnd.collection-v2.spotify.proto";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // CollectionItem { uri = 1, added_at = 2, is_removed = 3 }
    let mut item = Vec::new();
    proto_string(&mut item, 1, &format!("spotify:track:{track_id}"));
    proto_varint(&mut item, 2, if on { now } else { 0 });
    proto_varint(&mut item, 3, u64::from(!on));
    // WriteRequest { username = 1, set = 2, items = 3, client_update_id = 4 }
    let mut body = Vec::new();
    proto_string(&mut body, 1, &session.username());
    proto_string(&mut body, 2, "collection");
    proto_bytes(&mut body, 3, &item);
    proto_string(&mut body, 4, &format!("kopuz-{now}-{}", std::process::id()));
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::CONTENT_TYPE, CONTENT.parse().expect("static"));
    headers.insert(http::header::ACCEPT, CONTENT.parse().expect("static"));
    session
        .spclient()
        .request(
            &http::Method::POST,
            "/collection/v2/write",
            Some(headers),
            Some(&body),
        )
        .await
        .map(|_| ())
        .map_err(|error| format!("set liked: {error}"))
}

/// Minimal protobuf wire encoding for the two request shapes above.
fn proto_varint_raw(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn proto_varint(out: &mut Vec<u8>, field: u32, value: u64) {
    proto_varint_raw(out, u64::from(field << 3));
    proto_varint_raw(out, value);
}

fn proto_bytes(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    proto_varint_raw(out, u64::from((field << 3) | 2));
    proto_varint_raw(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn proto_string(out: &mut Vec<u8>, field: u32, value: &str) {
    proto_bytes(out, field, value.as_bytes());
}

/// A playlist as the rootlist lists it.
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
}

/// The account's own and followed playlists, in the client's order. Folder
/// markers in the rootlist are skipped.
pub async fn playlists(session: &Session) -> Result<Vec<PlaylistSummary>, String> {
    let mut out = Vec::new();
    let mut from = 0usize;
    loop {
        let bytes = session
            .spclient()
            .get_rootlist(from, Some(ROOTLIST_PAGE))
            .await
            .map_err(|error| format!("playlists: {error}"))?;
        let list = SelectedListContent::parse_from_bytes(&bytes)
            .map_err(|error| format!("playlists: {error}"))?;
        let items = &list.contents.items;
        if items.is_empty() {
            break;
        }
        for (index, item) in items.iter().enumerate() {
            let Some(id) = item.uri().strip_prefix("spotify:playlist:") else {
                continue;
            };
            let meta = list.contents.meta_items.get(index);
            let name = meta
                .map(|meta| meta.attributes.name().to_string())
                .unwrap_or_default();
            let image = meta.and_then(|meta| {
                let picture = meta.attributes.picture();
                (!picture.is_empty()).then(|| format!("{IMAGE_URL}{}", hex::encode(picture)))
            });
            out.push(PlaylistSummary {
                id: id.to_string(),
                name,
                image,
            });
        }
        from += items.len();
        if from >= list.length().max(0) as usize {
            break;
        }
    }
    Ok(out)
}

/// A playlist's entries, in order.
pub async fn playlist_tracks(session: &Session, playlist_id: &str) -> Result<Vec<Track>, String> {
    let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{playlist_id}"))
        .map_err(|error| format!("not a Spotify playlist id: {error}"))?;
    let playlist = Playlist::get(session, &uri)
        .await
        .map_err(|error| format!("playlist: {error}"))?;
    let uris: Vec<SpotifyUri> = playlist
        .tracks()
        .filter(|uri| matches!(uri, SpotifyUri::Track { .. }))
        .cloned()
        .collect();
    tracks(session, &uris).await
}

/// One album with its header and every track in disc order.
pub async fn album(session: &Session, album_id: &str) -> Result<RemoteAlbum, String> {
    let uri = album_uri(album_id)?;
    let album = Album::get(session, &uri)
        .await
        .map_err(|error| format!("album: {error}"))?;
    let uris: Vec<SpotifyUri> = album.tracks().cloned().collect();
    let cover = image_url(&album.covers).or_else(|| image_url(&album.cover_group));
    let mut entries = tracks(session, &uris).await?;
    for track in &mut entries {
        if track.cover.is_none() {
            track.cover = cover.clone();
        }
        if track.album.is_empty() {
            track.album = album.name.clone();
            track.album_id = album_id.to_string();
        }
    }
    let year = album.date.as_utc().year();
    Ok(RemoteAlbum {
        browse_id: album_id.to_string(),
        title: album.name.clone(),
        artist: album.artists.first().map(|artist| artist.name.clone()),
        year: (year > 0).then(|| year.to_string()),
        thumbnail: cover,
        audio_playlist_id: None,
        tracks: entries,
    })
}

/// Saved albums with their tracks, for the library snapshot.
pub async fn saved_albums(session: &Session) -> Result<(Vec<reader::Album>, Vec<Track>), String> {
    let mut albums = Vec::new();
    let mut all_tracks = Vec::new();
    for item in collection(session).await? {
        if item.kind != CollectionKind::Album {
            continue;
        }
        let id = item.id.to_base62().map_err(|error| error.to_string())?;
        let uri = album_uri(&id)?;
        let album = match Album::get(session, &uri).await {
            Ok(album) => album,
            Err(error) => {
                tracing::debug!(%error, album = %id, "spotify saved album unavailable");
                continue;
            }
        };
        let summary = album_from(&album);
        let cover = image_url(&album.covers).or_else(|| image_url(&album.cover_group));
        let uris: Vec<SpotifyUri> = album.tracks().cloned().collect();
        let mut entries = tracks(session, &uris).await?;
        for track in &mut entries {
            track.album = summary.title.clone();
            track.album_id = summary.id.clone();
            if track.cover.is_none() {
                track.cover = cover.clone();
            }
        }
        all_tracks.extend(entries);
        albums.push(summary);
    }
    Ok((albums, all_tracks))
}

/// A context the account played lately, with the track it was on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentContext {
    pub uri: String,
    pub last_track: Option<String>,
}

/// What the account listened to recently, newest first.
pub async fn recently_played(session: &Session) -> Result<Vec<RecentContext>, String> {
    let user = session.username();
    let endpoint = format!(
        "/recently-played/v3/user/{user}/recently-played?format=json&offset=0&limit=50&filter=default"
    );
    let body = client_request(session, http::Method::GET, &endpoint, None, Vec::new())
        .await
        .map_err(|error| format!("recently played: {error}"))?;
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|error| format!("recently played: {error}"))?;
    Ok(json["playContexts"]
        .as_array()
        .map(|contexts| {
            contexts
                .iter()
                .filter_map(|context| {
                    Some(RecentContext {
                        uri: context["uri"].as_str()?.to_string(),
                        last_track: context["lastPlayedTrackUri"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The home page: what was played lately, where it was played from, and
/// what was liked lately. Shelves with nothing in them are left out.
pub async fn discover_home(session: &Session) -> Result<DiscoverHome, String> {
    let mut shelves = Vec::new();

    let recent = match recently_played(session).await {
        Ok(recent) => recent,
        Err(error) => {
            tracing::warn!(%error, "spotify discover: recently played unavailable");
            Vec::new()
        }
    };

    let mut seen = std::collections::HashSet::new();
    let recent_tracks: Vec<SpotifyUri> = recent
        .iter()
        .filter_map(|context| context.last_track.as_deref())
        .filter(|uri| seen.insert((*uri).to_string()))
        .filter_map(|uri| SpotifyUri::from_uri(uri).ok())
        .take(20)
        .collect();
    if !recent_tracks.is_empty()
        && let Ok(songs) = tracks(session, &recent_tracks).await
        && !songs.is_empty()
    {
        shelves.push(song_shelf("Recently played", songs));
    }

    let mut tiles = Vec::new();
    for context in recent.iter().take(12) {
        if let Some(id) = context.uri.strip_prefix("spotify:playlist:") {
            let Ok(uri) = SpotifyUri::from_uri(&context.uri) else {
                continue;
            };
            if let Ok(playlist) = Playlist::get(session, &uri).await {
                let picture = &playlist.attributes.picture;
                tiles.push(DiscoverItem::Playlist {
                    playlist_id: id.to_string(),
                    title: playlist.name().to_string(),
                    subtitle: "Playlist".to_string(),
                    thumbnail: (!picture.is_empty())
                        .then(|| format!("{IMAGE_URL}{}", hex::encode(picture))),
                });
            }
        } else if let Some(id) = context.uri.strip_prefix("spotify:album:") {
            let Ok(uri) = SpotifyUri::from_uri(&context.uri) else {
                continue;
            };
            if let Ok(album) = Album::get(session, &uri).await {
                tiles.push(DiscoverItem::Album {
                    browse_id: id.to_string(),
                    title: album.name.clone(),
                    subtitle: album
                        .artists
                        .first()
                        .map(|artist| artist.name.clone())
                        .unwrap_or_default(),
                    thumbnail: image_url(&album.covers).or_else(|| image_url(&album.cover_group)),
                });
            }
        }
    }
    if !tiles.is_empty() {
        shelves.push(DiscoverShelf {
            title: "Jump back in".to_string(),
            strapline: None,
            more_browse_id: None,
            items: tiles,
            is_song_list: false,
        });
    }

    let liked: Vec<SpotifyUri> = collection(session)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|item| item.kind == CollectionKind::Track)
        .take(20)
        .filter_map(|item| item.id.to_base62().ok())
        .filter_map(|id| track_uri(&id).ok())
        .collect();
    if !liked.is_empty()
        && let Ok(songs) = tracks(session, &liked).await
        && !songs.is_empty()
    {
        shelves.push(song_shelf("Recently liked", songs));
    }

    if shelves.is_empty() {
        return Err("Spotify has no listening history to build a home from yet".to_string());
    }
    Ok(DiscoverHome {
        shelves,
        continuation: None,
    })
}

fn song_shelf(title: &str, songs: Vec<Track>) -> DiscoverShelf {
    DiscoverShelf {
        title: title.to_string(),
        strapline: None,
        more_browse_id: None,
        items: songs
            .into_iter()
            .map(|track| DiscoverItem::Song(Box::new(track)))
            .collect(),
        is_song_list: false,
    }
}

/// The cheapest authenticated round trip, for `validate`.
pub async fn probe(session: &Session) -> Result<(), String> {
    session
        .spclient()
        .get_rootlist(0, Some(1))
        .await
        .map(|_| ())
        .map_err(|error| format!("rootlist: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_items_decode_their_raw_ids() {
        // 16 zero-bytes base64: a valid, if unlikely, id.
        let item = serde_json::json!({
            "type": "TRACK",
            "identifier": "AAAAAAAAAAAAAAAAAAAAAA==",
            "added_at": 1_700_000_000u64,
        });
        let parsed = parse_collection_item(&item).expect("item");
        assert_eq!(parsed.kind, CollectionKind::Track);
        assert_eq!(parsed.added_at, 1_700_000_000);
        assert_eq!(parsed.id.to_base62().unwrap(), "0000000000000000000000");
    }

    #[test]
    fn write_request_encodes_as_protobuf() {
        let mut out = Vec::new();
        proto_string(&mut out, 1, "ab");
        proto_varint(&mut out, 2, 300);
        proto_varint(&mut out, 3, 1);
        assert_eq!(out, vec![0x0a, 2, b'a', b'b', 0x10, 0xac, 0x02, 0x18, 1]);
    }

    #[test]
    fn unknown_collection_kinds_are_skipped() {
        let item = serde_json::json!({
            "type": "SHOW",
            "identifier": "AAAAAAAAAAAAAAAAAAAAAA==",
        });
        assert!(parse_collection_item(&item).is_none());
    }
}
