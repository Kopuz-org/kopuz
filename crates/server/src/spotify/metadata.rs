//! User-library + playlist fetchers over the librespot `Session`.
//!
//! Everything here goes through Spotify's internal access-point protocol (the
//! `metadata` crate's `Metadata::get` + `spclient`), never the public Web API,
//! so a full library hydration won't trip the public `429` rate limits. Per-item
//! metadata fan-out is bounded by a [`Semaphore`] to stay gentle on the AP.

use std::sync::Arc;

use config::MusicService;
use librespot_core::SpotifyUri;
use librespot_metadata::{Metadata, Playlist, Track as SpTrack};
use protobuf::Message;
use reader::models::TrackId;
use tokio::sync::Semaphore;

use super::session::{self, ensure_session};

/// Max concurrent per-track / per-playlist metadata requests.
const FANOUT: usize = 8;

/// A user playlist's listing metadata (no entries).
pub struct SpPlaylist {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
}

/// Build the CDN cover URL for an image file hash.
fn cover_url(hex: &str) -> String {
    format!("https://i.scdn.co/image/{hex}")
}

/// Map a librespot `Track` into the app's generic [`reader::Track`].
fn map_track(t: &SpTrack) -> reader::Track {
    let item_id = match &t.id {
        SpotifyUri::Track { id } => id.to_base62().unwrap_or_default(),
        _ => String::new(),
    };
    let artists: Vec<String> = t.artists.0.iter().map(|a| a.name.clone()).collect();
    let artist = artists.first().cloned().unwrap_or_default();
    let album_id = match &t.album.id {
        SpotifyUri::Album { id } => id.to_base62().unwrap_or_default(),
        _ => String::new(),
    };
    let cover = t
        .album
        .covers
        .0
        .iter()
        .chain(t.album.cover_group.0.iter())
        .find_map(|img| img.id.to_base16().ok())
        .map(|h| cover_url(&h));

    reader::Track {
        id: TrackId::Server {
            service: MusicService::Spotify,
            item_id,
        },
        cover,
        album_id,
        title: t.name.clone(),
        artist,
        album: t.album.name.clone(),
        duration: (t.duration.max(0) as u64) / 1000,
        khz: 0,
        bitrate: 0,
        track_number: Some(t.number.max(0) as u32),
        disc_number: Some(t.disc_number.max(0) as u32),
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists,
    }
}

/// Fetch full track metadata for a batch of track URIs, bounded by [`FANOUT`].
/// Returns the resolved tracks plus the count of URIs that still failed after
/// retries. Callers decide whether a shortfall is tolerable: for favorites it
/// is not — a partial result fed into the epoch sweep would delete every
/// favorite the fetch happened to miss.
async fn fetch_tracks(
    session: &librespot_core::session::Session,
    uris: Vec<SpotifyUri>,
) -> (Vec<reader::Track>, usize) {
    let mut pending: Vec<SpotifyUri> = uris
        .into_iter()
        .filter(|u| matches!(u, SpotifyUri::Track { .. }))
        .collect();
    let mut out = Vec::with_capacity(pending.len());
    let mut lost = 0usize;
    // Rate-limited fetches fail in bursts; give stragglers a breather and
    // retry instead of dropping them.
    for attempt in 0..3u32 {
        if pending.is_empty() {
            break;
        }
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * u64::from(attempt))).await;
        }
        let sem = Arc::new(Semaphore::new(FANOUT));
        let mut handles = Vec::with_capacity(pending.len());
        for uri in pending.drain(..) {
            let s = session.clone();
            let sem = sem.clone();
            handles.push(tokio::spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(_) => return Err(uri),
                };
                match SpTrack::get(&s, &uri).await {
                    Ok(t) => Ok(map_track(&t)),
                    Err(_) => Err(uri),
                }
            }));
        }
        for h in handles {
            match h.await {
                Ok(Ok(t)) => out.push(t),
                Ok(Err(uri)) => pending.push(uri),
                // Join error: the uri moved into the task, so it can't be retried.
                Err(_) => lost += 1,
            }
        }
    }
    let failed = pending.len() + lost;
    if failed > 0 {
        tracing::warn!(
            failed,
            resolved = out.len(),
            "spotify: track metadata fetches failed after retries"
        );
    }
    (out, failed)
}

/// Fetch one track's tags (title/artist/album/duration) by base62 id. Used by
/// the YouTube match resolver to build a search query for a Spotify track.
pub async fn track_meta(token: String, track_id: String) -> Result<reader::Track, String> {
    session::on_rt(async move {
        let session = ensure_session(&token).await?;
        let id = librespot_core::SpotifyId::from_base62(&track_id)
            .map_err(|e| format!("spotify id: {e}"))?;
        let track = SpTrack::get(&session, &SpotifyUri::Track { id })
            .await
            .map_err(|e| format!("spotify track metadata: {e}"))?;
        Ok::<reader::Track, String>(map_track(&track))
    })
    .await?
}

/// List the signed-in user's playlists (root list), each decorated with its name.
pub async fn list_playlists(token: String) -> Result<Vec<SpPlaylist>, String> {
    session::on_rt(async move {
        let session = ensure_session(&token).await?;
        let bytes = session
            .spclient()
            .get_rootlist(0, None)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "spotify rootlist fetch failed");
                format!("spotify rootlist: {e}")
            })?;
        tracing::info!(bytes = bytes.len(), "spotify: got rootlist");
        let msg =
            librespot_protocol::playlist4_external::SelectedListContent::parse_from_bytes(&bytes)
                .map_err(|e| format!("spotify rootlist decode: {e}"))?;

        let uris: Vec<SpotifyUri> = msg
            .contents
            .get_or_default()
            .items
            .iter()
            .filter_map(|item| item.uri.as_deref())
            .filter(|u| u.starts_with("spotify:playlist:"))
            .filter_map(|u| SpotifyUri::from_uri(u).ok())
            .collect();
        tracing::info!(found = uris.len(), "spotify: rootlist playlist uris");

        let sem = Arc::new(Semaphore::new(FANOUT));
        let mut handles = Vec::with_capacity(uris.len());
        for uri in uris {
            let s = session.clone();
            let sem = sem.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.ok()?;
                let id = match &uri {
                    SpotifyUri::Playlist { id, .. } => id.to_base62().ok()?,
                    _ => return None,
                };
                let p = Playlist::get(&s, &uri).await.ok()?;
                Some(SpPlaylist {
                    id,
                    name: p.name().to_string(),
                    image: None,
                })
            }));
        }
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            if let Ok(Some(p)) = h.await {
                out.push(p);
            }
        }
        tracing::info!(playlists = out.len(), "spotify: listed playlists");
        Ok::<Vec<SpPlaylist>, String>(out)
    })
    .await?
}

/// Fetch the user's Liked Songs ("collection") via librespot's internal
/// context-resolve endpoint — NOT the public Web API, which 429s hard on the
/// shared keymaster client id.
pub async fn liked_tracks(token: String) -> Result<Vec<reader::Track>, String> {
    session::on_rt(async move {
        let session = ensure_session(&token).await?;
        let user = session.username();
        let ctx = session
            .spclient()
            .get_context(&format!("spotify:user:{user}:collection"))
            .await
            .map_err(|e| format!("spotify collection: {e}"))?;

        let uris: Vec<SpotifyUri> = ctx
            .pages
            .iter()
            .flat_map(|p| p.tracks.iter())
            .filter_map(|t| t.uri.as_deref())
            .filter(|u| u.starts_with("spotify:track:"))
            .filter_map(|u| SpotifyUri::from_uri(u).ok())
            .collect();
        tracing::info!(count = uris.len(), "spotify: liked track uris");
        let total = uris.len();
        let (tracks, failed) = fetch_tracks(&session, uris).await;
        // A partial result must be an error, not a success: the favorites sync
        // treats a successful fetch as the complete remote set and sweeps
        // (deletes) every favorite not in it.
        if failed > 0 {
            return Err(format!(
                "spotify liked songs: {failed} of {total} tracks failed to resolve"
            ));
        }
        Ok::<Vec<reader::Track>, String>(tracks)
    })
    .await?
}

/// Fetch the tracks of a single playlist (by base62 id).
pub async fn playlist_entries(
    token: String,
    playlist_id: String,
) -> Result<Vec<reader::Track>, String> {
    session::on_rt(async move {
        let session = ensure_session(&token).await?;
        let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{playlist_id}"))
            .map_err(|e| format!("spotify playlist uri: {e}"))?;
        let plist = Playlist::get(&session, &uri)
            .await
            .map_err(|e| format!("spotify playlist: {e}"))?;
        let uris: Vec<SpotifyUri> = plist.tracks().cloned().collect();
        // Unlike favorites, a playlist shortfall only under-renders the list —
        // nothing is deleted — so partial results are acceptable here.
        let (tracks, _failed) = fetch_tracks(&session, uris).await;
        Ok::<Vec<reader::Track>, String>(tracks)
    })
    .await?
}
