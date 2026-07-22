//! Resolve playlist files, .m3u for example, served in place of a stream URL to
//! their first entry: the decoder can't probe them.

const PLAYLIST_CONTENT_TYPES: &[&str] = &[
    "audio/x-scpls",
    "application/pls+xml",
    "audio/x-mpegurl",
    "audio/mpegurl",
    "application/x-mpegurl",
    "application/vnd.apple.mpegurl",
];

const PLAYLIST_EXTENSIONS: &[&str] = &[".pls", ".m3u", ".m3u8"];

/// Judged by `Content-Type` first, URL extension as fallback.
pub fn is_playlist(content_type: Option<&str>, url_path: &str) -> bool {
    if let Some(ct) = content_type {
        let ct = ct
            .split(';')
            .next()
            .unwrap_or(ct)
            .trim()
            .to_ascii_lowercase();
        if PLAYLIST_CONTENT_TYPES.contains(&ct.as_str()) {
            return true;
        }
        // Concrete audio type beats the extension check: some servers serve
        // real audio from ".m3u" paths.
        if ct.starts_with("audio/") || ct.starts_with("video/") {
            return false;
        }
    }
    let path = url_path.to_ascii_lowercase();
    PLAYLIST_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
}

/// First absolute http(s) URL: `FileN=` in PLS, first non-comment M3U line.
pub fn first_stream_url(text: &str) -> Option<String> {
    let is_pls = text
        .lines()
        .next()
        .is_some_and(|l| l.trim().eq_ignore_ascii_case("[playlist]"));

    for line in text.lines() {
        let line = line.trim();
        let candidate = if is_pls {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if !key.trim().to_ascii_lowercase().starts_with("file") {
                continue;
            }
            value.trim()
        } else {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            line
        };
        if candidate.starts_with("http://") || candidate.starts_with("https://") {
            return Some(candidate.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_playlists_by_content_type_and_extension() {
        assert!(is_playlist(Some("audio/x-scpls"), "/groovesalad.pls"));
        assert!(is_playlist(
            Some("application/pls+xml; charset=utf-8"),
            "/anything"
        ));
        assert!(is_playlist(None, "/stream.m3u"));
        assert!(is_playlist(None, "/hls/master.M3U8"));
        assert!(!is_playlist(Some("audio/mpeg"), "/stream"));
        // Concrete audio content-type overrides a playlist-looking path.
        assert!(!is_playlist(Some("audio/aac"), "/legacy.m3u"));
        assert!(!is_playlist(None, "/stream.mp3"));
    }

    #[test]
    fn extracts_first_url_from_pls() {
        let pls = "[playlist]\nnumberofentries=2\nFile1=https://ice6.somafm.com/groovesalad-128-mp3\nTitle1=SomaFM\nFile2=https://ice2.somafm.com/groovesalad-128-mp3\n";
        assert_eq!(
            first_stream_url(pls).as_deref(),
            Some("https://ice6.somafm.com/groovesalad-128-mp3")
        );
    }

    #[test]
    fn extracts_first_url_from_m3u() {
        let m3u = "#EXTM3U\n#EXTINF:-1,Station\nhttps://stream.example.org/live\n";
        assert_eq!(
            first_stream_url(m3u).as_deref(),
            Some("https://stream.example.org/live")
        );
        assert_eq!(first_stream_url("relative/path.mp3\n"), None);
    }
}
