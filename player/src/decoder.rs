use std::io::{Read, Seek};
use std::path::Path;
use symphonia::core::io::MediaSource;
use symphonia::core::probe::Hint;

struct ReadSeekSource {
    inner: Box<dyn ReadSeekSendSync>,
    len: Option<u64>,
}

trait ReadSeekSendSync: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> ReadSeekSendSync for T {}

impl ReadSeekSource {
    fn new(inner: Box<dyn ReadSeekSendSync>, len: Option<u64>) -> Self {
        Self { inner, len }
    }
}

impl Read for ReadSeekSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for ReadSeekSource {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl MediaSource for ReadSeekSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

pub fn open_file(path: &Path) -> Result<(Box<dyn MediaSource>, Hint), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata().ok().map(|m| m.len());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let source: Box<dyn MediaSource> = Box::new(ReadSeekSource::new(Box::new(file), len));
    Ok((source, hint))
}

pub fn from_stream(
    mut stream: impl Read + Seek + Send + Sync + 'static,
) -> (Box<dyn MediaSource>, Hint) {
    let len = stream.seek(std::io::SeekFrom::End(0)).ok();
    let _ = stream.seek(std::io::SeekFrom::Start(0));

    let source: Box<dyn MediaSource> = Box::new(ReadSeekSource::new(Box::new(stream), len));
    let hint = Hint::new();
    (source, hint)
}

pub fn from_stream_with_len(
    stream: impl Read + Seek + Send + Sync + 'static,
    len: Option<u64>,
) -> (Box<dyn MediaSource>, Hint) {
    let source: Box<dyn MediaSource> = Box::new(ReadSeekSource::new(Box::new(stream), len));
    let hint = Hint::new();
    (source, hint)
}

/// a read-only source wrapper for non-seekable streams source (e.g. internet radio).
struct ReadOnlySource {
    inner: Box<dyn Read + Send + Sync>,
}

impl Read for ReadOnlySource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for ReadOnlySource {
    fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "seek not supported on radio stream",
        ))
    }
}

impl MediaSource for ReadOnlySource {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// Create a media source from a non-seekable stream with an explicit format hint.
/// Used for internet radio streams where seeking is not possible.
pub fn from_stream_with_hint(
    stream: impl Read + Send + Sync + 'static,
    extension: &str,
) -> (Box<dyn MediaSource>, Hint) {
    let source: Box<dyn MediaSource> = Box::new(ReadOnlySource {
        inner: Box::new(stream),
    });
    let mut hint = Hint::new();
    hint.with_extension(extension);
    (source, hint)
}
