use std::io::{self, Read, Seek, SeekFrom};

use crate::stream::stream_buffer::BufferProgressCallback;

pub(crate) struct SmbStream {
    reader: Option<smb2::FileReader>,
    runtime: tokio::runtime::Handle,
    length: u64,
    position: u64,
    window: Vec<u8>,
    window_start: u64,
    budget: Option<u64>,
    progress: Option<BufferProgressCallback>,
}

impl SmbStream {
    pub(super) fn new(reader: smb2::FileReader) -> Self {
        Self {
            length: reader.size(),
            reader: Some(reader),
            runtime: tokio::runtime::Handle::current(),
            position: 0,
            window: Vec::new(),
            window_start: 0,
            budget: None,
            progress: None,
        }
    }

    pub(crate) fn limit_reads(&mut self, bytes: u64) {
        self.budget = Some(bytes);
    }

    pub(crate) fn set_progress(&mut self, progress: Option<BufferProgressCallback>) {
        self.progress = progress;
    }

    pub(crate) fn length(&self) -> u64 {
        self.length
    }
}

impl Read for SmbStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        if self.position < self.window_start
            || self.position - self.window_start >= self.window.len() as u64
        {
            let read_ahead = if self.budget.is_some() {
                output.len().clamp(16 * 1024, 512 * 1024) as u64
            } else {
                512 * 1024
            };
            let size = read_ahead.min(self.budget.unwrap_or(u64::MAX));
            if size == 0 {
                return Err(io::Error::other("SMB metadata read limit reached"));
            }
            let reader = self
                .reader
                .as_ref()
                .ok_or_else(|| io::Error::other("SMB file is closed"))?;
            self.window = self
                .runtime
                .block_on(super::call(reader.read_at(self.position, size)))
                .map_err(io::Error::other)?;
            self.window_start = self.position;
            if let Some(budget) = &mut self.budget {
                *budget = budget.saturating_sub(self.window.len() as u64);
            }
            if self.window.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "SMB file changed while reading",
                ));
            }
            if let Some(progress) = &self.progress {
                progress(
                    self.position,
                    self.position + self.window.len() as u64,
                    Some(self.length),
                );
            }
        }
        let offset = (self.position - self.window_start) as usize;
        let count = output.len().min(self.window.len() - offset);
        output[..count].copy_from_slice(&self.window[offset..offset + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for SmbStream {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = seek_position(self.position, self.length, from)?;
        Ok(self.position)
    }
}

fn seek_position(current: u64, length: u64, from: SeekFrom) -> io::Result<u64> {
    let position = match from {
        SeekFrom::Start(position) => i128::from(position),
        SeekFrom::Current(offset) => i128::from(current) + i128::from(offset),
        SeekFrom::End(offset) => i128::from(length) + i128::from(offset),
    };
    u64::try_from(position)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid SMB seek position"))
}

impl symphonia::core::io::MediaSource for SmbStream {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.length())
    }
}

impl Drop for SmbStream {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            self.runtime.spawn(async move {
                let _ = super::call(reader.close()).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smb_seeks_allow_eof_and_reject_underflow_and_overflow() {
        assert_eq!(seek_position(10, 100, SeekFrom::End(-5)).unwrap(), 95);
        assert_eq!(seek_position(10, 100, SeekFrom::Current(-10)).unwrap(), 0);
        assert_eq!(seek_position(0, 100, SeekFrom::Start(101)).unwrap(), 101);
        assert!(seek_position(0, 100, SeekFrom::Current(-1)).is_err());
        assert!(seek_position(u64::MAX, 100, SeekFrom::Current(1)).is_err());
    }
}
