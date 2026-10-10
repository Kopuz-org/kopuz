//! Audio CD tracks exposed as seekable PCM WAV streams. The drive stays on the
//! daemon host; clients only see opaque, disc-specific track keys.

use std::io::{self, Read, Seek, SeekFrom};

use sha2::{Digest, Sha256};

mod buffer;
mod device;
pub mod metadata;
mod native;
pub use native::Monitor;
pub(crate) use native::Reader;

const SECTOR_BYTES: usize = 2352;
const HEADER_BYTES: u64 = 44;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscTrack {
    pub number: u8,
    pub start: i32,
    pub end: i32,
    pub audio: bool,
}

#[derive(Clone, Debug)]
pub struct Disc {
    pub id: String,
    pub tracks: Vec<DiscTrack>,
}

impl Disc {
    fn new(tracks: Vec<DiscTrack>) -> io::Result<Self> {
        if tracks.is_empty()
            || tracks.len() > 99
            || tracks
                .iter()
                .any(|t| t.number == 0 || t.number > 99 || t.start < 0 || t.end <= t.start)
            || tracks
                .windows(2)
                .any(|pair| pair[0].number + 1 != pair[1].number || pair[0].end > pair[1].start)
        {
            return Err(io::Error::other("Invalid audio CD table of contents"));
        }
        let mut hash = Sha256::new();
        for track in &tracks {
            hash.update([track.number, u8::from(track.audio)]);
            hash.update(track.start.to_le_bytes());
            hash.update(track.end.to_le_bytes());
        }
        Ok(Self {
            id: hex::encode(hash.finalize()),
            tracks,
        })
    }

    pub fn key(&self, track: &DiscTrack) -> String {
        format!("{}-{:02}", self.id, track.number)
    }

    fn track(&self, key: &str) -> io::Result<&DiscTrack> {
        self.tracks.iter().find(|track| track.audio && self.key(track) == key)
            .ok_or_else(|| io::Error::other("This track is not on the inserted audio CD. Refresh the source after changing discs."))
    }
}

pub fn supported() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "windows",
        target_os = "macos"
    ))
}

pub fn inspect(device: &str) -> io::Result<Disc> {
    native::Drive::open(device)?.disc()
}

pub fn open(device: &str, key: &str) -> io::Result<Box<dyn symphonia::core::io::MediaSource>> {
    Reader::new(device).request().open(key)
}

trait SectorReader: Send + Sync {
    fn sector(&mut self, lsn: i32, output: &mut [u8; SECTOR_BYTES]) -> io::Result<()>;
}

struct WavStream<R> {
    reader: R,
    track: DiscTrack,
    header: [u8; HEADER_BYTES as usize],
    length: u64,
    position: u64,
    cached_sector: Option<i32>,
    sector: [u8; SECTOR_BYTES],
}

impl<R: SectorReader> WavStream<R> {
    fn new(reader: R, track: DiscTrack) -> io::Result<Self> {
        let sectors = u32::try_from(track.end - track.start)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| io::Error::other("Invalid audio CD track length"))?;
        let data_len = sectors
            .checked_mul(SECTOR_BYTES as u32)
            .filter(|n| n.checked_add(36).is_some())
            .ok_or_else(|| io::Error::other("Audio CD track exceeds WAV size limit"))?;
        let mut header = [0; HEADER_BYTES as usize];
        header[..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&(data_len + 36).to_le_bytes());
        header[8..16].copy_from_slice(b"WAVEfmt ");
        header[16..20].copy_from_slice(&16u32.to_le_bytes());
        header[20..22].copy_from_slice(&1u16.to_le_bytes());
        header[22..24].copy_from_slice(&2u16.to_le_bytes());
        header[24..28].copy_from_slice(&44100u32.to_le_bytes());
        header[28..32].copy_from_slice(&176400u32.to_le_bytes());
        header[32..34].copy_from_slice(&4u16.to_le_bytes());
        header[34..36].copy_from_slice(&16u16.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&data_len.to_le_bytes());
        Ok(Self {
            reader,
            track,
            header,
            length: HEADER_BYTES + u64::from(data_len),
            position: 0,
            cached_sector: None,
            sector: [0; SECTOR_BYTES],
        })
    }
}

impl<R: SectorReader> Read for WavStream<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        // Return at sector boundaries so errors never discard already-copied bytes.
        let bytes = if self.position < HEADER_BYTES {
            &self.header[self.position as usize..]
        } else {
            let offset = self.position - HEADER_BYTES;
            let lsn = self.track.start + (offset / SECTOR_BYTES as u64) as i32;
            if self.cached_sector != Some(lsn) {
                self.reader.sector(lsn, &mut self.sector)?;
                self.cached_sector = Some(lsn);
            }
            &self.sector[(offset % SECTOR_BYTES as u64) as usize..]
        };
        let count = bytes.len().min(output.len());
        output[..count].copy_from_slice(&bytes[..count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl<R: SectorReader> Seek for WavStream<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid CD seek position"))?;
        Ok(self.position)
    }
}

impl<R: SectorReader> symphonia::core::io::MediaSource for WavStream<R> {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}

#[cfg(test)]
mod tests;
