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
    let mut dummy = [0; 1];
    let _ = stream.read(&mut dummy);
    let len = stream.seek(std::io::SeekFrom::End(0)).ok();
    let _ = stream.seek(std::io::SeekFrom::Start(0));

    let source: Box<dyn MediaSource> = Box::new(ReadSeekSource::new(Box::new(stream), len));
    let hint = Hint::new();
    (source, hint)
}
