//! Byte sources the decoder reads a remote track through.

pub mod hls_source;
pub mod icy;
pub mod range_source;
pub mod stream_buffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioFormat {
    Webm,
    M4a,
}

impl AudioFormat {
    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::Webm => "webm",
            AudioFormat::M4a => "m4a",
        }
    }

    pub(crate) fn from_mime(mime: &str) -> Option<AudioFormat> {
        if mime.contains("webm") {
            Some(AudioFormat::Webm)
        } else if mime.contains("mp4") {
            Some(AudioFormat::M4a)
        } else {
            None
        }
    }
}
