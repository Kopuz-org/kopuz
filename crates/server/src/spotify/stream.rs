//! Spotify audio as a seekable byte source.
//!
//! A track's audio is an Ogg Vorbis (or MP3) file on Spotify's CDN, AES-CTR
//! encrypted with a per-file key the access point hands out to a logged-in
//! Premium session. librespot fetches the file in ranges and decrypts on
//! read, so the result is a `Read + Seek` the engine's decoder can scrub
//! through exactly like a local file. The engine decodes it: nothing here
//! touches samples.

use std::io::{self, Read, Seek, SeekFrom};

use librespot_audio::{AudioDecrypt, AudioFile};
use librespot_core::{Session, SpotifyId, SpotifyUri};
use librespot_metadata::audio::{AudioFileFormat, AudioFiles, AudioItem};

/// Spotify's Ogg files open with a 0xa7-byte custom header (normalisation
/// data) that no demuxer understands; the Ogg stream starts after it.
const OGG_HEADER_END: u64 = 0xa7;

/// Preference order: best Ogg Vorbis first, MP3 as the fallback Spotify
/// keeps for some catalogue items. Free accounts only get the 160 kbps file.
const FORMATS: &[AudioFileFormat] = &[
    AudioFileFormat::OGG_VORBIS_320,
    AudioFileFormat::OGG_VORBIS_160,
    AudioFileFormat::OGG_VORBIS_96,
    AudioFileFormat::MP3_320,
    AudioFileFormat::MP3_256,
    AudioFileFormat::MP3_160,
    AudioFileFormat::MP3_96,
];

/// The nominal bitrate, which sizes librespot's read-ahead and is what the
/// now-playing surface shows.
fn kbps(format: AudioFileFormat) -> u32 {
    match format {
        AudioFileFormat::OGG_VORBIS_320 | AudioFileFormat::MP3_320 => 320,
        AudioFileFormat::MP3_256 => 256,
        AudioFileFormat::OGG_VORBIS_160 | AudioFileFormat::MP3_160 => 160,
        AudioFileFormat::OGG_VORBIS_96 | AudioFileFormat::MP3_96 => 96,
        _ => 160,
    }
}

/// One track's decrypted audio, positioned past the Spotify header.
pub struct SpotifyAudio {
    inner: AudioDecrypt<AudioFile>,
    /// Where the real container starts inside the decrypted file.
    offset: u64,
    /// Bytes of container, i.e. the file length minus the header.
    len: u64,
    /// The demuxer hint: `ogg` or `mp3`.
    pub extension: &'static str,
    pub bitrate_kbps: u32,
}

impl SpotifyAudio {
    /// The container's length, for the decoder's seek table.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Read for SpotifyAudio {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for SpotifyAudio {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(at) => SeekFrom::Start(at.saturating_add(self.offset)),
            SeekFrom::End(delta) => {
                let target = (self.len as i64).saturating_add(delta);
                if target < 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "seek before the start of the stream",
                    ));
                }
                SeekFrom::Start((target as u64).saturating_add(self.offset))
            }
            SeekFrom::Current(delta) => SeekFrom::Current(delta),
        };
        let absolute = self.inner.seek(pos)?;
        Ok(absolute.saturating_sub(self.offset))
    }
}

/// Fetch and decrypt the audio for `track_id` (base62). Fails with a message
/// meant for the user when the track is unavailable in the account's
/// country or the account cannot stream.
pub async fn open(session: &Session, track_id: &str) -> Result<SpotifyAudio, String> {
    let uri = SpotifyUri::from_uri(&format!("spotify:track:{track_id}"))
        .map_err(|error| format!("not a Spotify track id: {error}"))?;
    let item = AudioItem::get_file(session, uri.clone())
        .await
        .map_err(|error| format!("Spotify has no metadata for this track: {error}"))?;
    let item = playable(session, item).await?;

    let (format, file_id) = FORMATS
        .iter()
        .find_map(|format| item.files.get(format).map(|id| (*format, *id)))
        .ok_or_else(|| {
            format!(
                "Spotify offers \"{}\" in no format kopuz can play",
                item.name
            )
        })?;
    let bitrate_kbps = kbps(format);
    let bytes_per_second = (bitrate_kbps as usize) * 1024 / 8;

    let encrypted = AudioFile::open(session, file_id, bytes_per_second)
        .await
        .map_err(|error| format!("Spotify would not stream \"{}\": {error}", item.name))?;
    let total = encrypted
        .get_stream_loader_controller()
        .map_err(|error| error.to_string())?
        .len() as u64;

    // The playable item may be an alternative of the asked-for track; the
    // key is bound to the file that is actually being read.
    let key_track: SpotifyId = (&item.track_id)
        .try_into()
        .map_err(|error| format!("Spotify track id: {error}"))?;
    // Every catalogue file is encrypted, so a refused key is the end of the
    // road: reading the bytes anyway only fails in the demuxer. A refusal is
    // either a free account or one of the accounts Spotify's backend blocks
    // from librespot clients outright (librespot-org/librespot#1649); the
    // client cannot tell which, and can do nothing about either.
    let key = session
        .audio_key()
        .request(key_track, file_id)
        .await
        .map_err(|error| {
            format!(
                "Spotify would not hand out the key for \"{}\" ({error}): the account is \
                 free, or one Spotify blocks from third-party clients",
                item.name
            )
        })?;
    let mut inner = AudioDecrypt::new(Some(key), encrypted);

    let (offset, extension) = if AudioFiles::is_ogg_vorbis(format) {
        (OGG_HEADER_END, "ogg")
    } else {
        (0, "mp3")
    };
    inner
        .seek(SeekFrom::Start(offset))
        .map_err(|error| format!("Spotify stream would not start: {error}"))?;
    Ok(SpotifyAudio {
        inner,
        offset,
        len: total.saturating_sub(offset),
        extension,
        bitrate_kbps,
    })
}

/// The item itself when the account may play it, else the first playable
/// alternative Spotify lists (a re-release of the same recording).
async fn playable(session: &Session, item: AudioItem) -> Result<AudioItem, String> {
    match &item.availability {
        Ok(()) if !item.files.is_empty() => return Ok(item),
        Ok(()) => {}
        Err(reason) => {
            tracing::debug!(%reason, track = %item.name, "spotify track unavailable here");
        }
    }
    let alternatives = item
        .alternatives
        .as_ref()
        .map(|list| list.0.clone())
        .unwrap_or_default();
    for alternative in alternatives {
        if let Ok(candidate) = AudioItem::get_file(session, alternative).await
            && candidate.availability.is_ok()
            && !candidate.files.is_empty()
        {
            return Ok(candidate);
        }
    }
    Err(match item.availability {
        Err(reason) => format!(
            "\"{}\" is not available on Spotify here: {reason}",
            item.name
        ),
        Ok(()) => format!("\"{}\" has no audio on Spotify", item.name),
    })
}
