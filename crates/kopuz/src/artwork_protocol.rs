use tracing::Instrument;

#[cfg(not(target_arch = "wasm32"))]
fn thumb_cache_path(file_path: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file_path.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir().join(format!("rusic_thumb_{hash:016x}.jpg"))
}

#[cfg(not(target_arch = "wasm32"))]
fn hq_cache_path(file_path: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    "hq".hash(&mut hasher);
    file_path.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir().join(format!("rusic_hq_{hash:016x}.jpg"))
}

#[cfg(not(target_arch = "wasm32"))]
fn make_thumbnail(raw: &[u8], cache_path: &std::path::Path) -> Option<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    let img = image::load_from_memory(raw).ok()?;
    const MAX: u32 = 400;
    let img = if img.width() > MAX || img.height() > MAX {
        img.thumbnail(MAX, MAX)
    } else {
        img
    };
    let mut out: Vec<u8> = Vec::new();
    img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, 75))
        .ok()?;
    let _ = std::fs::write(cache_path, &out);
    Some(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn make_hq_image(raw: &[u8], cache_path: &std::path::Path) -> Option<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    const SIZE_LIMIT: usize = 2 * 1024 * 1024;
    const MAX_DIM: u32 = 1920;
    const QUALITY: u8 = 85;

    if raw.len() <= SIZE_LIMIT {
        return None;
    }
    let img = image::load_from_memory(raw).ok()?;
    let img = if img.width() > MAX_DIM || img.height() > MAX_DIM {
        img.thumbnail(MAX_DIM, MAX_DIM)
    } else {
        img
    };
    let mut out: Vec<u8> = Vec::new();
    img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, QUALITY))
        .ok()?;
    let _ = std::fs::write(cache_path, &out);
    Some(out)
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
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
            let high_quality = query.split('&').any(|kv| kv == "hq=1");

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

            if high_quality {
                let hq_path = hq_cache_path(&file_path);
                if hq_path.exists()
                    && let Ok(b) = tokio::fs::read(&hq_path).await
                {
                    responder.respond(resp(
                        200,
                        &[
                            ("Content-Type", "image/jpeg"),
                            ("Cache-Control", "public, max-age=31536000"),
                        ],
                        b,
                    ));
                    return;
                }
                match tokio::fs::read(&file_path).await {
                    Ok(raw) => {
                        let file_path_clone = file_path.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            make_hq_image(&raw, &hq_path)
                                .map(|b| (b, "image/jpeg"))
                                .unwrap_or_else(|| {
                                    let mime = if file_path_clone.ends_with(".png") {
                                        "image/png"
                                    } else {
                                        "image/jpeg"
                                    };
                                    (raw, mime)
                                })
                        })
                        .await;
                        match result {
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
                    Err(_) => responder.respond(resp(404, &[], Vec::new())),
                }
                return;
            }

            let thumb_path = thumb_cache_path(&file_path);

            let (bytes, mime) = if thumb_path.exists() {
                match tokio::fs::read(&thumb_path).await {
                    Ok(b) => (b, "image/jpeg"),
                    Err(_) => {
                        let _ = std::fs::remove_file(&thumb_path);
                        match tokio::fs::read(&file_path).await {
                            Ok(b) => (
                                b,
                                if file_path.ends_with(".png") {
                                    "image/png"
                                } else {
                                    "image/jpeg"
                                },
                            ),
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
                            match make_thumbnail(&raw, &thumb_path_clone) {
                                Some(b) => Ok(b),
                                None => Err(raw),
                            }
                        })
                        .await
                        {
                            Ok(Ok(b)) => (b, "image/jpeg"),
                            Ok(Err(raw)) => (
                                raw,
                                if file_path.ends_with(".png") {
                                    "image/png"
                                } else {
                                    "image/jpeg"
                                },
                            ),
                            Err(_) => {
                                responder.respond(resp(500, &[], Vec::new()));
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[artwork] not found {}: {}", file_path, e);
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
