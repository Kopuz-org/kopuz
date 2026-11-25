//! A stream its service encrypts with AES-128-CTR, decrypted as it is read.
//! CTR needs only the byte offset to decrypt from anywhere, so the stream
//! under it stays seekable.

use std::io::{Error, ErrorKind, Read, Result, Seek, SeekFrom};

use aes::cipher::{KeyIvInit as _, StreamCipher as _, StreamCipherSeek as _};

type Cipher = ctr::Ctr128BE<aes::Aes128>;

pub struct AesCtrStream<R> {
    inner: R,
    cipher: Cipher,
    /// Decrypted bytes at the start of the file that are not audio.
    skip: u64,
    /// The audio's length, without `skip`.
    len: u64,
}

impl<R: Read + Seek> AesCtrStream<R> {
    /// `total` is the length of the whole encrypted file.
    pub fn new(mut inner: R, total: u64, key: [u8; 16], iv: [u8; 16], skip: u32) -> Result<Self> {
        let skip = u64::from(skip);
        let mut cipher = Cipher::new(&key.into(), &iv.into());
        inner.seek(SeekFrom::Start(skip))?;
        cipher.seek(skip);
        Ok(Self {
            inner,
            cipher,
            skip,
            len: total.saturating_sub(skip),
        })
    }
}

impl<R: Read> Read for AesCtrStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let read = self.inner.read(buf)?;
        self.cipher.apply_keystream(&mut buf[..read]);
        Ok(read)
    }
}

impl<R: Read + Seek> Seek for AesCtrStream<R> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let before = || Error::new(ErrorKind::InvalidInput, "seek before the start");
        let target = match pos {
            SeekFrom::Start(at) => at,
            SeekFrom::End(delta) => self.len.checked_add_signed(delta).ok_or_else(before)?,
            SeekFrom::Current(delta) => {
                let at = self.inner.stream_position()?.saturating_sub(self.skip);
                at.checked_add_signed(delta).ok_or_else(before)?
            }
        };
        let absolute = self.inner.seek(SeekFrom::Start(target + self.skip))?;
        self.cipher
            .try_seek(absolute)
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "seek past the cipher's end"))?;
        Ok(absolute - self.skip)
    }
}

impl<R: Read + Seek + Send + Sync> symphonia::core::io::MediaSource for AesCtrStream<R> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 16] = [7; 16];
    /// Ends in 0xff, so the counter carries into the next byte within a few blocks.
    const IV: [u8; 16] = [
        0x72, 0xe0, 0x67, 0xfb, 0xdd, 0xcb, 0xcf, 0x77, 0xeb, 0xe8, 0xbc, 0x64, 0x3f, 0x63, 0x0d,
        0xff,
    ];
    const SKIP: u32 = 167;

    fn plain() -> Vec<u8> {
        (0..4096u32).map(|byte| (byte % 251) as u8).collect()
    }

    fn encrypted() -> Vec<u8> {
        let mut bytes = plain();
        Cipher::new(&KEY.into(), &IV.into()).apply_keystream(&mut bytes);
        bytes
    }

    fn stream() -> AesCtrStream<std::io::Cursor<Vec<u8>>> {
        let bytes = encrypted();
        let total = bytes.len() as u64;
        AesCtrStream::new(std::io::Cursor::new(bytes), total, KEY, IV, SKIP).expect("stream")
    }

    #[test]
    fn it_reads_the_audio_past_the_header() {
        let mut read = Vec::new();
        stream().read_to_end(&mut read).expect("read");
        assert_eq!(read, plain()[SKIP as usize..]);
    }

    #[test]
    fn it_decrypts_from_wherever_it_seeks() {
        use symphonia::core::io::MediaSource as _;
        let mut stream = stream();
        let plain = &plain()[SKIP as usize..];
        assert_eq!(stream.byte_len(), Some(plain.len() as u64));
        for (pos, at) in [
            (SeekFrom::Start(1000), 1000),
            (SeekFrom::Current(-333), 699),
            (SeekFrom::End(-40), plain.len() - 40),
            (SeekFrom::Start(0), 0),
        ] {
            assert_eq!(stream.seek(pos).expect("seek"), at as u64);
            let mut bytes = [0; 32];
            stream.read_exact(&mut bytes).expect("read");
            assert_eq!(bytes, plain[at..at + 32], "from {at}");
        }
        assert!(stream.seek(SeekFrom::End(-100_000)).is_err());
    }
}
