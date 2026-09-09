//! ICY (Shoutcast/Icecast) in-stream metadata.
//!
//! With `Icy-MetaData: 1` and an `icy-metaint` answer, the body carries a
//! metadata block every `metaint` bytes: a length byte `n`, then `n * 16`
//! bytes of NUL-padded `StreamTitle='…';` text.

/// Now-playing info from one metadata block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IcyMeta {
    pub title: String,
    /// Some stations put a per-song cover URL in `StreamUrl`.
    pub cover_url: Option<String>,
}

/// Splits raw body chunks into audio bytes and now-playing updates.
pub struct IcyDeinterleaver {
    metaint: usize,
    state: State,
    meta_buf: Vec<u8>,
}

enum State {
    /// Audio bytes left before the next metadata block.
    Audio(usize),
    /// At the 1-byte metadata length.
    MetaLen,
    /// Metadata bytes still expected.
    Meta(usize),
}

impl IcyDeinterleaver {
    pub fn new(metaint: usize) -> Self {
        Self {
            metaint,
            state: State::Audio(metaint),
            meta_buf: Vec::new(),
        }
    }

    pub fn push(&mut self, mut data: &[u8], audio: &mut Vec<u8>) -> Option<IcyMeta> {
        let mut last_meta = None;
        while !data.is_empty() {
            match self.state {
                State::Audio(remaining) => {
                    let take = remaining.min(data.len());
                    audio.extend_from_slice(&data[..take]);
                    data = &data[take..];
                    self.state = if remaining == take {
                        State::MetaLen
                    } else {
                        State::Audio(remaining - take)
                    };
                }
                State::MetaLen => {
                    let len = data[0] as usize * 16;
                    data = &data[1..];
                    if len == 0 {
                        self.state = State::Audio(self.metaint);
                    } else {
                        self.meta_buf.clear();
                        self.state = State::Meta(len);
                    }
                }
                State::Meta(needed) => {
                    let take = (needed - self.meta_buf.len()).min(data.len());
                    self.meta_buf.extend_from_slice(&data[..take]);
                    data = &data[take..];
                    if self.meta_buf.len() == needed {
                        if let Some(meta) = parse_stream_meta(&self.meta_buf) {
                            last_meta = Some(meta);
                        }
                        self.state = State::Audio(self.metaint);
                    }
                }
            }
        }
        last_meta
    }
}

/// Values may contain apostrophes, so terminate at the `';` before the
/// next key, or the last in the block.
fn extract_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}='");
    let start = text.find(&prefix)? + prefix.len();
    let rest = &text[start..];
    let end = rest
        .find("';StreamTitle")
        .or_else(|| rest.find("';StreamUrl"))
        .or_else(|| rest.rfind("';"))
        // Some servers drop the trailing semicolon on the last field.
        .or_else(|| rest.ends_with('\'').then(|| rest.len() - 1))?;
    Some(rest[..end].trim())
}

/// `None` on a missing/empty title (jingles send `StreamTitle='';`).
/// `StreamUrl` is kept only when it looks like cover art, not a homepage.
pub fn parse_stream_meta(block: &[u8]) -> Option<IcyMeta> {
    let text = String::from_utf8_lossy(block);
    let text = text.trim_end_matches('\0');

    let title = extract_field(text, "StreamTitle").filter(|t| !t.is_empty())?;

    let cover_url = extract_field(text, "StreamUrl")
        .filter(|u| {
            let path = u.split(['?', '#']).next().unwrap_or(u).to_ascii_lowercase();
            u.starts_with("https://")
                && [".jpg", ".jpeg", ".png", ".gif", ".webp"]
                    .iter()
                    .any(|ext| path.ends_with(ext))
        })
        .map(|u| u.to_string());

    Some(IcyMeta {
        title: title.to_string(),
        cover_url,
    })
}

/// Split `Artist - Title`; artist is `None` without a separator.
pub fn split_artist_title(raw: &str) -> (Option<String>, String) {
    match raw.split_once(" - ") {
        Some((artist, title)) if !artist.trim().is_empty() && !title.trim().is_empty() => {
            (Some(artist.trim().to_string()), title.trim().to_string())
        }
        _ => (None, raw.trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_block(text: &str) -> Vec<u8> {
        let mut block = text.as_bytes().to_vec();
        let padded = block.len().div_ceil(16) * 16;
        block.resize(padded, 0);
        let mut out = vec![(padded / 16) as u8];
        out.extend_from_slice(&block);
        out
    }

    #[test]
    fn deinterleaves_audio_and_titles() {
        let metaint = 8;
        let mut stream = Vec::new();
        stream.extend_from_slice(b"AAAAAAAA");
        stream.extend_from_slice(&meta_block("StreamTitle='Artist - Song';"));
        stream.extend_from_slice(b"BBBBBBBB");
        stream.push(0); // empty metadata block
        stream.extend_from_slice(b"CCCCCCCC");

        let mut parser = IcyDeinterleaver::new(metaint);
        let mut audio = Vec::new();

        // Feed one byte at a time to exercise every split point.
        let mut titles = Vec::new();
        for b in &stream {
            if let Some(m) = parser.push(std::slice::from_ref(b), &mut audio) {
                titles.push(m.title);
            }
        }

        assert_eq!(audio, b"AAAAAAAABBBBBBBBCCCCCCCC");
        assert_eq!(titles, vec!["Artist - Song".to_string()]);
    }

    #[test]
    fn deinterleaves_across_large_chunks() {
        let metaint = 4;
        let mut stream = Vec::new();
        stream.extend_from_slice(b"1234");
        stream.extend_from_slice(&meta_block("StreamTitle='One';"));
        stream.extend_from_slice(b"5678");
        stream.extend_from_slice(&meta_block("StreamTitle='Two';"));
        stream.extend_from_slice(b"90ab");
        stream.push(0);

        let mut parser = IcyDeinterleaver::new(metaint);
        let mut audio = Vec::new();
        let meta = parser.push(&stream, &mut audio);

        assert_eq!(audio, b"1234567890ab");
        // Whole stream in one chunk: only the last update is notable.
        assert_eq!(meta.map(|m| m.title), Some("Two".to_string()));
    }

    #[test]
    fn parses_stream_meta_variants() {
        assert_eq!(
            parse_stream_meta(b"StreamTitle='A - B';StreamUrl='';\0\0"),
            Some(IcyMeta {
                title: "A - B".to_string(),
                cover_url: None,
            })
        );
        // Apostrophe inside the title.
        assert_eq!(
            parse_stream_meta(b"StreamTitle='It's Raining';\0").map(|m| m.title),
            Some("It's Raining".to_string())
        );
        assert_eq!(parse_stream_meta(b"StreamTitle='';"), None);
        assert_eq!(parse_stream_meta(b"garbage"), None);
    }

    #[test]
    fn keeps_stream_url_only_when_it_is_cover_art() {
        let meta = parse_stream_meta(
            b"StreamTitle='X - Y';StreamUrl='https://somafm.com/logos/512/groovesalad512.jpg'",
        )
        .unwrap();
        assert_eq!(
            meta.cover_url.as_deref(),
            Some("https://somafm.com/logos/512/groovesalad512.jpg")
        );

        // Homepage links and plain-http images are rejected.
        let homepage = parse_stream_meta(b"StreamTitle='X';StreamUrl='https://somafm.com/';");
        assert_eq!(homepage.unwrap().cover_url, None);
        let http = parse_stream_meta(b"StreamTitle='X';StreamUrl='http://x.example/a.jpg';");
        assert_eq!(http.unwrap().cover_url, None);
    }

    #[test]
    fn splits_artist_and_title() {
        assert_eq!(
            split_artist_title("Daft Punk - Around the World"),
            (
                Some("Daft Punk".to_string()),
                "Around the World".to_string()
            )
        );
        assert_eq!(
            split_artist_title("Just a Title"),
            (None, "Just a Title".to_string())
        );
        assert_eq!(split_artist_title(" - x"), (None, "- x".to_string()));
    }
}
