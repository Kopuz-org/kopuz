//! Download one Spotify track to an in-memory OGG buffer.
//!
//! librespot downloads the file and (when a key is available) decrypts it; the
//! legacy AES audio key is optional now, so we tolerate its absence. We skip
//! Spotify's 167-byte (`0xA7`) proprietary OGG spacer and return the rest — a
//! clean OGG Vorbis stream Symphonia can probe and decode. The player wraps the
//! bytes in a `Cursor` (free seeking), mirroring the SoundCloud Go+ HLS path.

use std::io::{Read, Seek, SeekFrom};

use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::SpotifyId;
use librespot_core::session::Session;
use librespot_metadata::audio::{AudioFileFormat, AudioFiles};
use librespot_metadata::{Metadata, Track as SpTrack};

use super::session::{block_on_rt, ensure_session};

/// Spotify prefixes OGG Vorbis files with a 167-byte proprietary header.
const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;

/// Preference order: highest-quality OGG Vorbis first.
const FORMAT_PREFERENCE: &[AudioFileFormat] = &[
    AudioFileFormat::OGG_VORBIS_320,
    AudioFileFormat::OGG_VORBIS_160,
    AudioFileFormat::OGG_VORBIS_96,
];

/// Rough bytes/second for the chosen format (drives librespot's prefetch sizing).
fn bytes_per_second(format: AudioFileFormat) -> usize {
    match format {
        AudioFileFormat::OGG_VORBIS_320 => 40 * 1024,
        AudioFileFormat::OGG_VORBIS_160 => 20 * 1024,
        _ => 12 * 1024,
    }
}

/// Pick the best available OGG file from a track's file map.
fn pick_file(files: &AudioFiles) -> Option<(AudioFileFormat, librespot_core::FileId)> {
    FORMAT_PREFERENCE
        .iter()
        .find_map(|fmt| files.0.get(fmt).map(|id| (*fmt, *id)))
}

/// Download + decrypt `track_id` (base62) to a full OGG byte buffer. Blocks the
/// calling thread — call it from the player's `spawn_blocking` decode thread.
pub fn fetch_decrypted_ogg(track_id: &str, access_token: &str) -> Result<Vec<u8>, String> {
    let track_id = track_id.to_string();
    let access_token = access_token.to_string();

    tracing::info!(track = %track_id, "spotify: fetch_decrypted_ogg start");
    if access_token.is_empty() {
        return Err("spotify: no access token for playback".to_string());
    }
    block_on_rt(async move {
        let session = ensure_session(&access_token).await?;
        let spotify_id =
            SpotifyId::from_base62(&track_id).map_err(|e| format!("spotify id: {e}"))?;

        let track = SpTrack::get(&session, &librespot_core::SpotifyUri::Track { id: spotify_id })
            .await
            .map_err(|e| format!("spotify track metadata: {e}"))?;

        let candidates = playable_candidates(&session, &track).await;
        if candidates.is_empty() {
            return Err("no playable audio file for this track".to_string());
        }

        let mut last_err = String::from("spotify: no playable candidate");
        let mut chosen: Option<(librespot_audio::AudioFile, AudioFileFormat, Option<_>)> = None;
        for (cand_track, format, file_id) in candidates {
            let key = session.audio_key().request(cand_track, file_id).await.ok();
            let bps = bytes_per_second(format);
            match AudioFile::open(&session, file_id, bps).await {
                Ok(file) => {
                    tracing::info!(?format, has_key = key.is_some(), "spotify: opened audio file");
                    chosen = Some((file, format, key));
                    break;
                }
                Err(e) => last_err = format!("spotify audio download: {e}"),
            }
        }
        let Some((encrypted, _format, key)) = chosen else {
            return Err(last_err);
        };
        tracing::info!("spotify: decrypting/reading");

        let has_key = key.is_some();
        tokio::task::spawn_blocking(move || {
            let mut decrypt = AudioDecrypt::new(key, encrypted);
            decrypt
                .seek(SeekFrom::Start(SPOTIFY_OGG_HEADER_END))
                .map_err(|e| format!("spotify ogg seek: {e}"))?;
            let mut buf = Vec::new();
            decrypt
                .read_to_end(&mut buf)
                .map_err(|e| format!("spotify ogg read: {e}"))?;
            let magic_at_header = &buf[..buf.len().min(4)];
            let is_ogg = magic_at_header == b"OggS";
            tracing::info!(
                bytes = buf.len(),
                has_key,
                is_ogg,
                first4 = ?magic_at_header,
                "spotify: decoded buffer (is_ogg=true means valid unencrypted OGG)"
            );
            Ok::<Vec<u8>, String>(buf)
        })
        .await
        .map_err(|e| format!("spotify decrypt task: {e}"))?
    })?
}

/// All playable (track_id, format, file_id) candidates: the track itself first,
/// then each regional alternative. The caller tries them in order until the
/// audio-key request succeeds (region-locked primaries fail the key request even
/// though they list files).
async fn playable_candidates(
    session: &Session,
    track: &SpTrack,
) -> Vec<(SpotifyId, AudioFileFormat, librespot_core::FileId)> {
    let mut out = Vec::new();
    if let Some((fmt, id)) = pick_file(&track.files)
        && let librespot_core::SpotifyUri::Track { id: tid } = &track.id
    {
        out.push((*tid, fmt, id));
    }
    for alt_uri in track.alternatives.0.iter() {
        if let Ok(alt) = SpTrack::get(session, alt_uri).await
            && let Some((fmt, id)) = pick_file(&alt.files)
            && let librespot_core::SpotifyUri::Track { id: tid } = &alt.id
        {
            out.push((*tid, fmt, id));
        }
    }
    out
}
