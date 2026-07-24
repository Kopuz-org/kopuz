use tracing::Instrument;

fn thumb_cache_path(file_path: &str, max_size: u32) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file_path.hash(&mut hasher);
    max_size.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir().join(format!("rusic_thumb_{hash:016x}.jpg"))
}

fn make_thumbnail(raw: &[u8], max_size: u32, cache_path: &std::path::Path) -> Option<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    let img = image::load_from_memory(raw).ok()?;
    let img = img.thumbnail(max_size, max_size);
    let mut out: Vec<u8> = Vec::new();
    img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, 85))
        .ok()?;
    let _ = std::fs::write(cache_path, &out);
    Some(out)
}

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
        let mut b = http::Response::builder().status(status);
        b = b.header("Access-Control-Allow-Origin", "*");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(std::borrow::Cow::from(body)).unwrap_or_else(|_| {
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
            let file_path: String = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("p="))
                .map(|encoded| {
                    percent_encoding::percent_decode_str(encoded)
                        .decode_utf8_lossy()
                        .into_owned()
                })
                .unwrap_or_default();
            let max_size = query.split('&').find_map(|part| {
                part.strip_prefix("s=")
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|size| *size > 0)
            });
            if file_path.is_empty() {
                responder.respond(resp(400, &[], Vec::new()));
                return;
            }

            #[cfg(target_os = "windows")]
            let file_path = file_path.replace('/', "\\");

            #[cfg(not(target_os = "windows"))]
            let file_path = if file_path.starts_with('~') {
                if let Ok(home) = std::env::var("HOME") {
                    file_path.replacen('~', &home, 1)
                } else {
                    file_path
                }
            } else {
                file_path
            };

            let Some(max_size) = max_size else {
                match tokio::fs::read(&file_path).await {
                    Ok(raw) => responder.respond(resp(
                        200,
                        &[
                            ("Content-Type", mime_for_path(&file_path)),
                            ("Cache-Control", "public, max-age=31536000"),
                        ],
                        raw,
                    )),
                    Err(error) => {
                        tracing::warn!(path = %file_path, %error, "artwork not found");
                        responder.respond(resp(404, &[], Vec::new()));
                    }
                }
                return;
            };

            let thumb_path = thumb_cache_path(&file_path, max_size);

            let (bytes, mime) = if thumb_path.exists() {
                match tokio::fs::read(&thumb_path).await {
                    Ok(b) => (b, "image/jpeg"),
                    Err(_) => {
                        let _ = std::fs::remove_file(&thumb_path);
                        match tokio::fs::read(&file_path).await {
                            Ok(b) => (b, mime_for_path(&file_path)),
                            Err(_) => {
                                responder.respond(resp(404, &[], Vec::new()));
                                return;
                            }
                        }
                    }
                }
            } else {
                match tokio::fs::read(&file_path).await {
                    Ok(raw) => {
                        let thumb_path_clone = thumb_path.clone();
                        match tokio::task::spawn_blocking(move || {
                            match make_thumbnail(&raw, max_size, &thumb_path_clone) {
                                Some(b) => Ok(b),
                                None => Err(raw),
                            }
                        })
                        .await
                        {
                            Ok(Ok(b)) => (b, "image/jpeg"),
                            Ok(Err(raw)) => (raw, mime_for_path(&file_path)),
                            Err(_) => {
                                responder.respond(resp(500, &[], Vec::new()));
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(path = %file_path, error = %e, "artwork not found");
                        responder.respond(resp(404, &[], Vec::new()));
                        return;
                    }
                }
            };

            responder.respond(resp(
                200,
                &[
                    ("Content-Type", mime),
                    ("Cache-Control", "public, max-age=31536000"),
                ],
                bytes,
            ));
        }
        .in_current_span(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimized_cache_is_separate_for_each_size() {
        assert_ne!(
            thumb_cache_path("/music/cover.png", 512),
            thumb_cache_path("/music/cover.png", 1024)
        );
    }

    #[test]
    fn original_artwork_mime_preserves_common_formats() {
        assert_eq!(mime_for_path("cover.PNG"), "image/png");
        assert_eq!(mime_for_path("cover.webp"), "image/webp");
        assert_eq!(mime_for_path("cover.svg"), "image/svg+xml");
        assert_eq!(mime_for_path("cover.jpg"), "image/jpeg");
    }
}
