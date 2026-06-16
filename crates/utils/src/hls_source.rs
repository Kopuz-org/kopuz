//! Minimal HLS assembler for SoundCloud Go+ AAC streams.
//!
//! SoundCloud's high-quality (256 kbps AAC) transcoding is delivered as an HLS
//! media playlist of fragmented-MP4 segments rather than one progressive file.
//! Symphonia has no HLS demuxer, but a CMAF/fMP4 HLS stream is just an optional
//! init segment (`#EXT-X-MAP`) followed by media fragments — concatenating them
//! in order yields a single valid fMP4 byte stream the `isomp4` reader decodes.
//!
//! This downloads the playlist and all its segments synchronously and returns
//! the assembled bytes. It's blocking (built for the player's `spawn_blocking`
//! decode thread, beside `range_source`) and trades a short up-front download
//! for dead-simple, fully-seekable in-memory playback.

use std::io::{Error, ErrorKind, Result};
use std::time::Duration;

/// Upper bound on the assembled stream size. A single track's AAC stream is a
/// few tens of MiB; this 1 GiB cap guards against a malformed/malicious
/// playlist driving unbounded in-memory growth.
const MAX_TOTAL_BYTES: usize = 1024 * 1024 * 1024;

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// Fetch an HLS media-playlist URL and concatenate its init + media segments
/// into one contiguous (fMP4) byte buffer.
pub fn assemble(playlist_url: &str, user_agent: Option<&str>) -> Result<Vec<u8>> {
    let http = client();

    let mut playlist_req = http.get(playlist_url);
    if let Some(ua) = user_agent {
        playlist_req = playlist_req.header("User-Agent", ua);
    }
    let playlist = playlist_req
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| Error::other(format!("HLS playlist fetch: {e}")))?;

    let segments = parse_segment_urls(&playlist, playlist_url);
    if segments.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "HLS playlist had no segments",
        ));
    }

    let mut out = Vec::new();
    for url in segments {
        let mut seg_req = http.get(&url);
        if let Some(ua) = user_agent {
            seg_req = seg_req.header("User-Agent", ua);
        }
        let bytes = seg_req
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.bytes())
            .map_err(|e| Error::other(format!("HLS segment fetch: {e}")))?;
        if out.len().saturating_add(bytes.len()) > MAX_TOTAL_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "HLS stream exceeds maximum allowed size",
            ));
        }
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

/// Extract the ordered list of absolute segment URLs from a media playlist: the
/// optional `#EXT-X-MAP` init segment first, then each media segment line.
fn parse_segment_urls(playlist: &str, playlist_url: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for raw in playlist.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            if let Some(uri) = extract_attr_uri(rest) {
                urls.push(resolve_url(playlist_url, &uri));
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        urls.push(resolve_url(playlist_url, line));
    }
    urls
}

/// Pull `URI="…"` out of an `#EXT-X-MAP` attribute list.
fn extract_attr_uri(attrs: &str) -> Option<String> {
    let start = attrs.find("URI=\"")? + 5;
    let rest = &attrs[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Resolve a possibly-relative segment reference against the playlist URL.
fn resolve_url(base: &str, reference: &str) -> String {
    if reference.starts_with("http://") || reference.starts_with("https://") {
        return reference.to_string();
    }
    if let Some(scheme_end) = base.find("://") {
        let after_scheme = &base[scheme_end + 3..];
        if let Some(rel) = reference.strip_prefix('/') {
            if let Some(host_len) = after_scheme.find('/') {
                let host = &after_scheme[..host_len];
                return format!("{}://{host}/{rel}", &base[..scheme_end]);
            }
            return format!("{base}/{rel}");
        }
        let path_part = base.split('?').next().unwrap_or(base);
        // Only treat a slash that lies after "scheme://host" as a path
        // separator — rfind on the whole URL would otherwise match the slash
        // in "https://" when the base has no path, mangling the result.
        let after_scheme_start = scheme_end + 3;
        if let Some(rel) = path_part
            .get(after_scheme_start..)
            .and_then(|host_and_path| host_and_path.rfind('/'))
        {
            let slash = after_scheme_start + rel;
            return format!("{}/{reference}", &path_part[..slash]);
        }
        // Host only, no path component: append directly.
        return format!("{path_part}/{reference}");
    }
    reference.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_map_and_segments_in_order() {
        let playlist = "#EXTM3U\n\
            #EXT-X-MAP:URI=\"https://cdn.sndcdn.com/init.mp4\"\n\
            #EXTINF:6.0,\n\
            https://cdn.sndcdn.com/seg0.mp4\n\
            #EXTINF:6.0,\n\
            https://cdn.sndcdn.com/seg1.mp4\n\
            #EXT-X-ENDLIST\n";
        let urls = parse_segment_urls(playlist, "https://api.soundcloud.com/media/x/playlist.m3u8");
        assert_eq!(
            urls,
            vec![
                "https://cdn.sndcdn.com/init.mp4",
                "https://cdn.sndcdn.com/seg0.mp4",
                "https://cdn.sndcdn.com/seg1.mp4",
            ]
        );
    }

    #[test]
    fn resolves_relative_and_absolute_paths() {
        let base = "https://cf.sndcdn.com/media/abc/def/playlist.m3u8?token=xyz";
        assert_eq!(
            resolve_url(base, "seg1.mp4"),
            "https://cf.sndcdn.com/media/abc/def/seg1.mp4"
        );
        assert_eq!(
            resolve_url(base, "/other/seg2.mp4"),
            "https://cf.sndcdn.com/other/seg2.mp4"
        );
        assert_eq!(
            resolve_url(base, "https://x.com/s.mp4"),
            "https://x.com/s.mp4"
        );
        // Base with host only (no path) must not match the scheme's slash.
        assert_eq!(
            resolve_url("https://host.com", "seg.mp4"),
            "https://host.com/seg.mp4"
        );
        assert_eq!(
            resolve_url("https://host.com?token=z", "seg.mp4"),
            "https://host.com/seg.mp4"
        );
    }
}
