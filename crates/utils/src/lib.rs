pub mod color;
#[cfg(not(target_arch = "wasm32"))]
pub mod hls_source;
pub mod jellyfin_image;
#[cfg(not(target_arch = "wasm32"))]
pub mod logs;
pub mod lyrics;
pub mod musicbrainz;
#[cfg(not(target_arch = "wasm32"))]
pub mod range_source;
#[cfg(not(target_arch = "wasm32"))]
pub mod stream_buffer;
pub mod subsonic_image;
pub mod themes;
use std::path::Path;
use std::sync::Arc;

pub type CoverUrl = Arc<str>;

pub fn cover_url_from_string(url: String) -> CoverUrl {
    Arc::from(url)
}

pub fn map_cover_url(url: Option<String>) -> Option<CoverUrl> {
    url.map(cover_url_from_string)
}

/// Cross-platform async sleep that works on both native (tokio) and WASM (gloo-timers).
pub async fn sleep(duration: std::time::Duration) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(duration).await;
    }
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::sleep(duration).await;
    }
}

fn format_artwork_url_impl(path: Option<&impl AsRef<Path>>, size: Option<u32>) -> Option<CoverUrl> {
    let p = path?;
    let p = p.as_ref();
    let p_str = p.to_string_lossy();

    let abs_path = if let Some(stripped) = p_str.strip_prefix("./") {
        std::env::current_dir().unwrap_or_default().join(stripped)
    } else {
        p.to_path_buf()
    };

    let abs_str = abs_path.to_string_lossy();
    let abs_str = if abs_str.starts_with('~') {
        if let Ok(home) = std::env::var("HOME") {
            std::borrow::Cow::Owned(abs_str.replacen('~', &home, 1))
        } else {
            abs_str
        }
    } else {
        abs_str
    };

    // Android WebView is unreliable with custom URL schemes (artwork://) and the
    // http localhost shim, so inline the cover as a base64 data URL instead.
    #[cfg(target_os = "android")]
    {
        use base64::{Engine as _, engine::general_purpose};
        return match std::fs::read(&*abs_str) {
            Ok(bytes) => {
                let mime = if abs_str.ends_with(".png") {
                    "image/png"
                } else if abs_str.ends_with(".gif") {
                    "image/gif"
                } else if abs_str.ends_with(".webp") {
                    "image/webp"
                } else {
                    "image/jpeg"
                };
                let b64 = general_purpose::STANDARD.encode(&bytes);
                Some(cover_url_from_string(format!("data:{mime};base64,{b64}")))
            }
            Err(_) => None,
        };
    }

    const QUERY_VAL: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'&')
        .add(b'+')
        .add(b'=')
        .add(b'?')
        .add(b'<')
        .add(b'>')
        .add(b'`')
        .add(b'\\')
        .add(b':');

    if cfg!(target_os = "windows") {
        let mut url = format!(
            "http://artwork.dioxus.localhost/local?p={}",
            percent_encoding::utf8_percent_encode(&abs_str, QUERY_VAL)
        );
        if let Some(size) = size {
            url.push_str(&format!("&s={size}"));
        }
        Some(cover_url_from_string(url))
    } else {
        let mut url = format!(
            "artwork://local?p={}",
            percent_encoding::utf8_percent_encode(&abs_str, QUERY_VAL)
        );
        if let Some(size) = size {
            url.push_str(&format!("&s={size}"));
        }
        Some(cover_url_from_string(url))
    }
}

pub fn format_artwork_url(path: Option<&impl AsRef<Path>>) -> Option<CoverUrl> {
    format_artwork_url_impl(path, None)
}

pub fn format_artwork_thumb_url(path: Option<&impl AsRef<Path>>, size: u32) -> Option<CoverUrl> {
    format_artwork_url_impl(path, Some(size))
}

pub fn default_cover_url() -> CoverUrl {
    cover_url_from_string(
        "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='400' height='400' viewBox='0 0 400 400'%3E%3Crect width='400' height='400' fill='%231e1b2e'/%3E%3Ccircle cx='200' cy='180' r='70' fill='none' stroke='%233d3466' stroke-width='6'/%3E%3Cpath d='M155 280 Q200 240 245 280' fill='none' stroke='%233d3466' stroke-width='6' stroke-linecap='round'/%3E%3C/svg%3E".to_string()
    )
}
