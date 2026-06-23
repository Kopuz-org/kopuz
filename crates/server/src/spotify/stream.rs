//! Download one Spotify track to an in-memory audio buffer.
//!
//! Full playback: librespot downloads the file and decrypts it with the
//! per-file AES audio key. The key is required — Spotify's OGG Vorbis is
//! AES-CTR encrypted, so we only accept a candidate whose key request succeeds.
//! We skip Spotify's 167-byte (`0xA7`) proprietary OGG spacer and return the
//! rest — a clean OGG Vorbis stream Symphonia can probe and decode.
//!
//! Preview fallback ("premiumless"): Spotify currently refuses the audio key
//! for some accounts (a server-side restriction, see librespot#1649). When
//! every candidate's key is rejected we fall back to the unencrypted 30-second
//! preview clip (MP3) so the user still hears *something* — no key and no
//! Premium required. The returned extension tells the caller which decoder hint
//! to use. The player wraps the bytes in a `Cursor` (free seeking), mirroring
//! the SoundCloud Go+ HLS path.

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

/// Fetch playable audio for `track_id` (base62). Blocks the calling thread —
/// call it from the player's `spawn_blocking` decode thread. Returns the audio
/// bytes plus the container extension Symphonia should hint with: `"ogg"` for a
/// full decrypted track, `"mp3"` for the 30-second preview fallback.
pub fn fetch_track_audio(
    track_id: &str,
    access_token: &str,
) -> Result<(Vec<u8>, &'static str), String> {
    let track_id = track_id.to_string();
    let access_token = access_token.to_string();

    tracing::info!(track = %track_id, "spotify: fetch_track_audio start");
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

        // The audio key is REQUIRED: Spotify's OGG Vorbis files are AES-CTR
        // encrypted, and `AudioDecrypt` with a `None` key passes bytes through
        // unaltered — Symphonia would then choke on still-encrypted data. So we
        // gate on the key, not on the file opening: a region-locked primary
        // lists files and opens fine but has its key request rejected
        // (`error audio key …`), while a regional alternative's key succeeds.
        // Request the key first (cheap) and only download the file we can decrypt.
        let mut last_err = String::from("spotify: no playable candidate");
        let mut chosen: Option<(librespot_audio::AudioFile, AudioFileFormat, _)> = None;
        for (cand_track, format, file_id) in candidates {
            let key = match session.audio_key().request(cand_track, file_id).await {
                Ok(key) => key,
                Err(e) => {
                    tracing::warn!(?format, error = %e, "spotify: audio key rejected, trying next candidate");
                    last_err = format!("spotify audio key: {e}");
                    continue;
                }
            };
            let bps = bytes_per_second(format);
            match AudioFile::open(&session, file_id, bps).await {
                Ok(file) => {
                    tracing::info!(?format, "spotify: opened audio file with key");
                    chosen = Some((file, format, key));
                    break;
                }
                Err(e) => last_err = format!("spotify audio download: {e}"),
            }
        }
        let Some((encrypted, _format, key)) = chosen else {
            // Every audio-key request was rejected by Spotify (`AesKeyError`).
            // This is a known, server-side restriction Spotify applies to
            // certain accounts — the legacy audio-key protocol librespot uses is
            // being phased out / rate-limited per-account, so the *same* account
            // fails on every librespot-based client (incl. go-librespot) while
            // other accounts work. See librespot#1649. We can't decrypt the full
            // track, so fall back to the unencrypted 30-second preview clip.
            tracing::warn!(
                "spotify: all audio-key requests rejected (last error: {last_err}) — falling back \
                 to the 30s preview clip (known server-side restriction, see librespot#1649)"
            );
            return match fetch_preview(&session, &track).await {
                Ok(bytes) => {
                    tracing::info!(
                        bytes = bytes.len(),
                        "spotify: playing 30s preview clip (full audio key unavailable)"
                    );
                    Ok((bytes, "mp3"))
                }
                Err(prev_err) => Err(format!(
                    "Spotify refused the audio decryption key for this account and no preview is \
                     available (key: {last_err}; preview: {prev_err}). Playback is blocked \
                     server-side — a known librespot limitation (see librespot#1649), not a kopuz bug."
                )),
            };
        };
        tracing::info!("spotify: decrypting/reading");

        let buf = tokio::task::spawn_blocking(move || {
            let mut decrypt = AudioDecrypt::new(Some(key), encrypted);
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
                is_ogg,
                first4 = ?magic_at_header,
                "spotify: decoded buffer (is_ogg=true means valid decrypted OGG)"
            );
            if !is_ogg {
                return Err(format!(
                    "spotify: decrypted data is not valid OGG (first bytes {magic_at_header:?}); \
                     the audio key may be wrong for this track"
                ));
            }
            Ok::<Vec<u8>, String>(buf)
        })
        .await
        .map_err(|e| format!("spotify decrypt task: {e}"))??;
        Ok((buf, "ogg"))
    })?
}

/// Fetch the unencrypted 30-second preview clip (MP3) for a track, trying the
/// track itself then its alternatives. Needs no audio key and no Premium.
async fn fetch_preview(session: &Session, track: &SpTrack) -> Result<Vec<u8>, String> {
    let mut preview_id = pick_preview(&track.previews);
    if preview_id.is_none() {
        for alt_uri in track.alternatives.0.iter() {
            if let Ok(alt) = SpTrack::get(session, alt_uri).await
                && let Some(id) = pick_preview(&alt.previews)
            {
                preview_id = Some(id);
                break;
            }
        }
    }
    let preview_id = preview_id.ok_or_else(|| "no preview clip for this track".to_string())?;

    let bytes = session
        .spclient()
        .get_audio_preview(&preview_id)
        .await
        .map_err(|e| format!("spotify preview fetch: {e}"))?;
    if bytes.is_empty() {
        return Err("spotify preview was empty".to_string());
    }
    Ok(bytes.to_vec())
}

/// Any preview file id from a track's preview map (previews are short MP3s).
fn pick_preview(previews: &AudioFiles) -> Option<librespot_core::FileId> {
    previews.0.values().next().copied()
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
