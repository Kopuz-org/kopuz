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

use tracing::Instrument;

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
pub fn serve(uri: http::Uri, responder: dioxus::desktop::RequestAsyncResponder) {
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

    tokio::spawn(
        async move {
            let query = uri.query().unwrap_or_default();
            let decode = |encoded: &str| {
                percent_encoding::percent_decode_str(encoded)
                    .decode_utf8_lossy()
                    .into_owned()
            };
            let file_path = query
                .split('&')
                .find_map(|part| part.strip_prefix("p="))
                .map(&decode)
                .unwrap_or_default();
            let high_quality = query.split('&').any(|part| part == "hq=1");

            // A library entity: the daemon resolves it, because a server cover
            // is signed with credentials that never leave it.
            if let Some(target) = query.split('&').find_map(|part| {
                let (kind, id) = part.split_once('=')?;
                let id = decode(id);
                match kind {
                    "track" => Some(api::ArtworkTarget::Track(id)),
                    "album" => Some(api::ArtworkTarget::Album(id)),
                    "artist" => Some(api::ArtworkTarget::Artist(id)),
                    "playlist" => Some(api::ArtworkTarget::Playlist(id)),
                    "catalog" => Some(api::ArtworkTarget::Catalog(id)),
                    "station" => Some(api::ArtworkTarget::Station(id)),
                    _ => None,
                }
            }) {
                let request = api::ArtworkRequest {
                    target,
                    hq: high_quality,
                };
                match api::ArtworkApi::artwork(crate::backend::api().as_ref(), request).await {
                    Ok(data) => responder.respond(resp(
                        200,
                        &[
                            ("Content-Type", data.content_type.as_str()),
                            ("Cache-Control", "public, max-age=31536000, immutable"),
                        ],
                        data.bytes,
                    )),
                    Err(error) => {
                        tracing::debug!(%error, "no artwork for entity");
                        responder.respond(resp(404, &[], Vec::new()));
                    }
                }
                return;
            }

            if file_path.is_empty() {
                responder.respond(resp(400, &[], Vec::new()));
                return;
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
                Ok(bytes) => responder.respond(resp(
                    200,
                    &[
                        ("Content-Type", mime_for_path(&file_path)),
                        ("Cache-Control", "public, max-age=31536000"),
                    ],
                    bytes,
                )),
                Err(error) => {
                    tracing::warn!(path = %file_path, %error, "background image not found");
                    responder.respond(resp(404, &[], Vec::new()));
                }
            }
        }
        .in_current_span(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_mime_preserves_common_formats() {
        assert_eq!(mime_for_path("/covers/art.png"), "image/png");
        assert_eq!(mime_for_path("/covers/art.WEBP"), "image/webp");
        assert_eq!(mime_for_path("/covers/art.jpg"), "image/jpeg");
        assert_eq!(mime_for_path("/covers/art"), "image/jpeg");
    }
}
