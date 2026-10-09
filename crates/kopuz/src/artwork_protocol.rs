//! The `artwork://` scheme the webview loads pictures from.
//!
//! Two shapes. `api?<kind>=<id>&v=<version>` is a library entity, which the
//! daemon resolves and serves the bytes for: a server cover is signed with
//! credentials that never leave it, and a version in the URL is what makes
//! the response safe to cache forever.
//!
//! `local?p=<path>` is one file this process was told to show -- the custom
//! background someone picked in settings. It is the only path a frontend
//! still reads from disk itself.
//!
//! `video?track=<key>` is the picture of a queued music video, answered a
//! byte range at a time from the daemon, since a video element seeks by range
//! and the stream URL is signed for the daemon's session.

#[cfg(not(target_os = "android"))]
use tracing::Instrument;

#[cfg(not(target_os = "android"))]
use dioxus::desktop::RequestAsyncResponder;

#[cfg(not(target_os = "android"))]
fn mime_for_path(file_path: &str) -> &'static str {
    let extension = std::path::Path::new(file_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("png") {
        "image/png"
    } else if extension.eq_ignore_ascii_case("gif") {
        "image/gif"
    } else if extension.eq_ignore_ascii_case("webp") {
        "image/webp"
    } else if extension.eq_ignore_ascii_case("bmp") {
        "image/bmp"
    } else if extension.eq_ignore_ascii_case("avif") {
        "image/avif"
    } else if extension.eq_ignore_ascii_case("svg") {
        "image/svg+xml"
    } else if extension.eq_ignore_ascii_case("tif") || extension.eq_ignore_ascii_case("tiff") {
        "image/tiff"
    } else if extension.eq_ignore_ascii_case("ico") {
        "image/x-icon"
    } else {
        "image/jpeg"
    }
}

#[cfg(not(target_os = "android"))]
pub fn serve(uri: http::Uri, range: Option<String>, responder: RequestAsyncResponder) {
    fn resp(
        status: u16,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> http::Response<std::borrow::Cow<'static, [u8]>> {
        let mut builder = http::Response::builder()
            .status(status)
            .header("Access-Control-Allow-Origin", "*");
        for (key, value) in headers {
            builder = builder.header(*key, *value);
        }
        builder
            .body(std::borrow::Cow::from(body))
            .unwrap_or_else(|_| {
                http::Response::builder()
                    .status(500)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(std::borrow::Cow::from(Vec::new()))
                    .expect("static fallback response")
            })
    }

    let response = async move {
        let query = uri.query().unwrap_or_default();
        let decode = |encoded: &str| {
            percent_encoding::percent_decode_str(encoded)
                .decode_utf8_lossy()
                .into_owned()
        };
        if uri.path().trim_start_matches('/') == "video" || uri.host() == Some("video") {
            let Some(key) = query
                .split('&')
                .find_map(|part| part.strip_prefix("track="))
                .map(&decode)
            else {
                return resp(400, &[], Vec::new());
            };
            let (start, end) = range.as_deref().and_then(byte_range).unwrap_or((0, None));
            return match video_blocks::read(key, start, end).await {
                Ok(chunk) => {
                    let total = chunk
                        .total
                        .map_or_else(|| "*".to_string(), |total| total.to_string());
                    if chunk.bytes.is_empty() {
                        return resp(
                            416,
                            &[("Content-Range", &format!("bytes */{total}"))],
                            Vec::new(),
                        );
                    }
                    let last = chunk.start + chunk.bytes.len() as u64 - 1;
                    resp(
                        206,
                        &[
                            ("Content-Type", chunk.content_type.as_str()),
                            ("Accept-Ranges", "bytes"),
                            (
                                "Content-Range",
                                &format!("bytes {}-{last}/{total}", chunk.start),
                            ),
                            ("Cache-Control", "no-store"),
                        ],
                        chunk.bytes,
                    )
                }
                Err(error) => {
                    tracing::debug!(%error, "no video for track");
                    resp(404, &[], Vec::new())
                }
            };
        }
        let file_path = query
            .split('&')
            .find_map(|part| part.strip_prefix("p="))
            .map(&decode)
            .unwrap_or_default();
        // A library entity: the daemon resolves it, because a server cover
        // is signed with credentials that never leave it.
        if let Some(request) = entity_request(&uri) {
            return match api::ArtworkApi::artwork(crate::backend::api().as_ref(), request).await {
                Ok(data) => resp(
                    200,
                    &[
                        ("Content-Type", data.content_type.as_str()),
                        ("Cache-Control", "public, max-age=31536000, immutable"),
                    ],
                    data.bytes,
                ),
                Err(error) => {
                    tracing::debug!(%error, "no artwork for entity");
                    resp(404, &[], Vec::new())
                }
            };
        }

        if file_path.is_empty() {
            return resp(400, &[], Vec::new());
        }

        #[cfg(target_os = "windows")]
        let file_path = file_path.replace('/', "\\");

        #[cfg(not(target_os = "windows"))]
        let file_path = match file_path.strip_prefix('~') {
            Some(rest) => match std::env::var("HOME") {
                Ok(home) => format!("{home}{rest}"),
                Err(_) => file_path,
            },
            None => file_path,
        };

        // One file, served as it is: the background is painted full-bleed,
        // so there is nothing to resize and nothing worth caching a copy of.
        match tokio::fs::read(&file_path).await {
            Ok(bytes) => resp(
                200,
                &[
                    ("Content-Type", mime_for_path(&file_path)),
                    ("Cache-Control", "public, max-age=31536000"),
                ],
                bytes,
            ),
            Err(error) => {
                tracing::warn!(path = %file_path, %error, "background image not found");
                resp(404, &[], Vec::new())
            }
        }
    };
    tokio::spawn(
        async move {
            let response = response.await;
            responder.respond(response);
        }
        .in_current_span(),
    );
}

/// Video ranges, read ahead a block at a time. AVFoundation reads a video
/// one sample at a time, a few KB per request, and each of those would
/// otherwise be a daemon call and a googlevideo fetch.
#[cfg(not(target_os = "android"))]
mod video_blocks {
    use std::collections::VecDeque;
    use std::sync::LazyLock;

    const BLOCK: u64 = 1 << 20;
    /// A few seconds behind and ahead of the playhead at 1080p.
    const KEEP: usize = 24;

    struct Block {
        key: String,
        index: u64,
        chunk: api::VideoChunk,
    }

    /// Held across a fetch, so readers of a block being fetched wait for it
    /// rather than fetch it again.
    static BLOCKS: LazyLock<tokio::sync::Mutex<VecDeque<Block>>> = LazyLock::new(Default::default);

    pub async fn read(
        key: String,
        start: u64,
        end: Option<u64>,
    ) -> Result<api::VideoChunk, api::ApiError> {
        let index = start / BLOCK;
        let mut blocks = BLOCKS.lock().await;
        let found = blocks
            .iter()
            .position(|block| block.index == index && block.key == key);
        let block = match found.and_then(|at| blocks.remove(at)) {
            Some(block) => block,
            None => {
                let request = api::VideoRequest {
                    key: key.clone(),
                    start: index * BLOCK,
                    length: Some(BLOCK),
                };
                let chunk = api::PlayerApi::video(crate::backend::api().as_ref(), request).await?;
                Block { key, index, chunk }
            }
        };
        let answer = slice(&block.chunk, start, end);
        blocks.push_back(block);
        while blocks.len() > KEEP {
            blocks.pop_front();
        }
        Ok(answer)
    }

    /// The part of `block` from `start` to `end` inclusive, up to the block's
    /// own end; a reader asks again for the rest. Empty past the stream's end.
    pub(super) fn slice(block: &api::VideoChunk, start: u64, end: Option<u64>) -> api::VideoChunk {
        let from = (start.saturating_sub(block.start) as usize).min(block.bytes.len());
        let to = end
            .map_or(block.bytes.len(), |end| {
                (end.saturating_sub(block.start) as usize).saturating_add(1)
            })
            .clamp(from, block.bytes.len());
        api::VideoChunk {
            content_type: block.content_type.clone(),
            start,
            total: block.total,
            bytes: block.bytes[from..to].to_vec(),
        }
    }
}

/// The first range of a `Range: bytes=` header, as a start and an inclusive
/// end. Suffix ranges are not something a video element sends.
#[cfg(not(target_os = "android"))]
fn byte_range(header: &str) -> Option<(u64, Option<u64>)> {
    let (start, end) = header
        .trim()
        .strip_prefix("bytes=")?
        .split(',')
        .next()?
        .split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = match end.trim() {
        "" => None,
        end => Some(end.parse().ok().filter(|end| *end >= start)?),
    };
    Some((start, end))
}

pub(crate) fn entity_request(uri: &http::Uri) -> Option<api::ArtworkRequest> {
    let query = uri.query()?;
    let target = query.split('&').find_map(|part| {
        let (kind, id) = part.split_once('=')?;
        let id = percent_encoding::percent_decode_str(id)
            .decode_utf8_lossy()
            .into_owned();
        match kind {
            "track" => Some(api::ArtworkTarget::Track(id)),
            "album" => Some(api::ArtworkTarget::Album(id)),
            "artist" => Some(api::ArtworkTarget::Artist(id)),
            "playlist" => Some(api::ArtworkTarget::Playlist(id)),
            "catalog" => Some(api::ArtworkTarget::Catalog(id)),
            "station" => Some(api::ArtworkTarget::Station(id)),
            _ => None,
        }
    })?;
    Some(api::ArtworkRequest {
        target,
        hq: query.split('&').any(|part| part == "hq=1"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_and_desktop_urls_resolve_every_artwork_entity() {
        for origin in [
            "http://127.0.0.1:49152/session/api",
            "artwork://dioxus.localhost/api",
            "artwork://api",
        ] {
            for kind in ["track", "album", "artist", "playlist", "catalog", "station"] {
                let uri = format!("{origin}?{kind}=provider%3Aa%26b%20%2Bc&v=42&hq=1")
                    .parse()
                    .unwrap();
                let request = entity_request(&uri).unwrap();
                assert_eq!(request.target.kind(), kind);
                assert_eq!(request.target.id(), "provider:a&b +c");
                assert!(request.hq);
            }
        }
        let local = "https://artwork.dioxus.localhost/local?p=%2Fcover.jpg"
            .parse()
            .unwrap();
        assert!(entity_request(&local).is_none());
    }

    #[test]
    #[cfg(not(target_os = "android"))]
    fn a_small_read_is_answered_from_its_block() {
        let block = api::VideoChunk {
            content_type: "video/mp4".into(),
            start: 1 << 20,
            total: Some(3 << 20),
            bytes: (0..=255u8).cycle().take(1 << 20).collect(),
        };
        let read = video_blocks::slice(&block, (1 << 20) + 10, Some((1 << 20) + 17));
        assert_eq!((read.start, read.bytes.len()), ((1 << 20) + 10, 8));
        assert_eq!(read.bytes[0], 10);
        // A read running past the block stops at its end.
        let tail = video_blocks::slice(&block, (2 << 20) - 4, Some(5 << 20));
        assert_eq!(tail.bytes.len(), 4);
        let open = video_blocks::slice(&block, (2 << 20) - 100, None);
        assert_eq!(open.bytes.len(), 100);
        assert_eq!(open.total, Some(3 << 20));
    }

    #[test]
    #[cfg(not(target_os = "android"))]
    fn a_video_element_range_reads_as_start_and_end() {
        assert_eq!(byte_range("bytes=0-1"), Some((0, Some(1))));
        assert_eq!(byte_range("bytes=1048576-"), Some((1_048_576, None)));
        assert_eq!(byte_range("bytes=10-20, 30-40"), Some((10, Some(20))));
        assert_eq!(byte_range("bytes=20-10"), None);
        assert_eq!(byte_range("bytes=-500"), None);
        assert_eq!(byte_range("items=0-1"), None);
    }

    #[test]
    #[cfg(not(target_os = "android"))]
    fn artwork_mime_preserves_common_formats() {
        assert_eq!(mime_for_path("/covers/art.png"), "image/png");
        assert_eq!(mime_for_path("/covers/art.WEBP"), "image/webp");
        assert_eq!(mime_for_path("/covers/art.jpg"), "image/jpeg");
        assert_eq!(mime_for_path("/covers/art"), "image/jpeg");
    }
}
