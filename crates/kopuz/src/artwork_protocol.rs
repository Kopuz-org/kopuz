use tracing::Instrument;

fn thumb_cache_path(file_path: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file_path.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir().join(format!("rusic_thumb_{hash:016x}.jpg"))
}

fn hq_cache_path(file_path: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "hq".hash(&mut hasher);
    file_path.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir().join(format!("rusic_hq_{hash:016x}.jpg"))
}

fn make_thumbnail(raw: &[u8], cache_path: &std::path::Path) -> Option<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    let img = image::load_from_memory(raw).ok()?;
    const MAX_DIMENSION: u32 = 400;
    let img = if img.width() > MAX_DIMENSION || img.height() > MAX_DIMENSION {
        img.thumbnail(MAX_DIMENSION, MAX_DIMENSION)
    } else {
        img
    };
    let mut out = Vec::new();
    img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, 75))
        .ok()?;
    let _ = std::fs::write(cache_path, &out);
    Some(out)
}

fn make_hq_image(raw: &[u8], cache_path: &std::path::Path) -> Option<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    const SIZE_LIMIT: usize = 2 * 1024 * 1024;
    const MAX_DIMENSION: u32 = 1920;

    if raw.len() <= SIZE_LIMIT {
        return None;
    }
    let img = image::load_from_memory(raw).ok()?;
    let img = if img.width() > MAX_DIMENSION || img.height() > MAX_DIMENSION {
        img.thumbnail(MAX_DIMENSION, MAX_DIMENSION)
    } else {
        img
    };
    let mut out = Vec::new();
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
            let file_path = query
                .split('&')
                .find_map(|part| part.strip_prefix("p="))
                .map(|encoded| {
                    percent_encoding::percent_decode_str(encoded)
                        .decode_utf8_lossy()
                        .into_owned()
                })
                .unwrap_or_default();
            let high_quality = query.split('&').any(|part| part == "hq=1");

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

            // Cover paths persisted before the 0.16 identity rename still
            // point into the old app directories.
            let file_path = crate::legacy::remap_identity_path(&file_path).unwrap_or(file_path);

            if high_quality {
                let cache_path = hq_cache_path(&file_path);
                if cache_path.exists()
                    && let Ok(bytes) = tokio::fs::read(&cache_path).await
                {
                    responder.respond(resp(
                        200,
                        &[
                            ("Content-Type", "image/jpeg"),
                            ("Cache-Control", "public, max-age=31536000"),
                        ],
                        bytes,
                    ));
                    return;
                }

                match tokio::fs::read(&file_path).await {
                    Ok(raw) => {
                        let mime = mime_for_path(&file_path);
                        match tokio::task::spawn_blocking(move || {
                            make_hq_image(&raw, &cache_path)
                                .map(|bytes| (bytes, "image/jpeg"))
                                .unwrap_or((raw, mime))
                        })
                        .await
                        {
                            Ok((bytes, mime)) => responder.respond(resp(
                                200,
                                &[
                                    ("Content-Type", mime),
                                    ("Cache-Control", "public, max-age=31536000"),
                                ],
                                bytes,
                            )),
                            Err(_) => responder.respond(resp(500, &[], Vec::new())),
                        }
                    }
                    Err(error) => {
                        tracing::warn!(path = %file_path, %error, "artwork not found");
                        responder.respond(resp(404, &[], Vec::new()));
                    }
                }
                return;
            }

            let cache_path = thumb_cache_path(&file_path);
            let (bytes, mime) = if cache_path.exists() {
                match tokio::fs::read(&cache_path).await {
                    Ok(bytes) => (bytes, "image/jpeg"),
                    Err(_) => {
                        let _ = std::fs::remove_file(&cache_path);
                        match tokio::fs::read(&file_path).await {
                            Ok(bytes) => (bytes, mime_for_path(&file_path)),
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
                        let cache_path_clone = cache_path.clone();
                        match tokio::task::spawn_blocking(move || {
                            make_thumbnail(&raw, &cache_path_clone)
                                .map(Ok)
                                .unwrap_or(Err(raw))
                        })
                        .await
                        {
                            Ok(Ok(bytes)) => (bytes, "image/jpeg"),
                            Ok(Err(raw)) => (raw, mime_for_path(&file_path)),
                            Err(_) => {
                                responder.respond(resp(500, &[], Vec::new()));
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(path = %file_path, %error, "artwork not found");
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
    fn thumbnail_and_background_caches_are_separate() {
        assert_ne!(
            thumb_cache_path("/music/cover.png"),
            hq_cache_path("/music/cover.png")
        );
    }

    #[test]
    fn artwork_mime_preserves_common_formats() {
        assert_eq!(mime_for_path("cover.PNG"), "image/png");
        assert_eq!(mime_for_path("cover.webp"), "image/webp");
        assert_eq!(mime_for_path("cover.svg"), "image/svg+xml");
        assert_eq!(mime_for_path("cover.jpg"), "image/jpeg");
    }
}
