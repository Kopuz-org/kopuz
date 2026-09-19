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

#[cfg(not(target_os = "android"))]
use dioxus::desktop::RequestAsyncResponder;
#[cfg(target_os = "android")]
use dioxus::mobile::RequestAsyncResponder;

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

pub fn serve(uri: http::Uri, responder: RequestAsyncResponder) {
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
            // Wry's Android request bridge panics after ten seconds without a
            // response. A slow provider must fail the image, not the app.
            #[cfg(target_os = "android")]
            let response = tokio::time::timeout(std::time::Duration::from_secs(8), response)
                .await
                .unwrap_or_else(|_| resp(504, &[("Cache-Control", "no-store")], Vec::new()));
            #[cfg(not(target_os = "android"))]
            let response = response.await;
            responder.respond(response);
        }
        .in_current_span(),
    );
}

fn entity_request(uri: &http::Uri) -> Option<api::ArtworkRequest> {
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
            "https://artwork.dioxus.localhost/api",
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
    fn artwork_mime_preserves_common_formats() {
        assert_eq!(mime_for_path("/covers/art.png"), "image/png");
        assert_eq!(mime_for_path("/covers/art.WEBP"), "image/webp");
        assert_eq!(mime_for_path("/covers/art.jpg"), "image/jpeg");
        assert_eq!(mime_for_path("/covers/art"), "image/jpeg");
    }
}
