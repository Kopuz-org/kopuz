pub mod color;
pub mod jellyfin_image;
pub mod lyrics;
pub mod stream_buffer;
pub mod subsonic_image;
use std::path::Path;

pub fn format_artwork_url(path: Option<&impl AsRef<Path>>) -> Option<String> {
    path.and_then(|p| {
        let p = p.as_ref();
        let p_str = p.to_string_lossy();

        let abs_path = if let Some(stripped) = p_str.strip_prefix("./") {
            std::env::current_dir().unwrap_or_default().join(stripped)
        } else {
            p.to_path_buf()
        };

        // Unix
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

        #[cfg(target_os = "windows")]
        {
            // Since Windows WebView2 is such a bitch with custom protocols, I decided to instead use base64 data URLs
            // TODO: Reduce overhead.
            use std::fs;

            if let Ok(bytes) = fs::read(&*abs_str) {
                // Determine MIME type, TODO: expand list
                let mime = if abs_str.ends_with(".png") {
                    "image/png"
                } else if abs_str.ends_with(".gif") {
                    "image/gif"
                } else if abs_str.ends_with(".webp") {
                    "image/webp"
                } else {
                    "image/jpeg"
                };

                // Encode to base64
                use base64::{Engine as _, engine::general_purpose};
                let b64 = general_purpose::STANDARD.encode(&bytes);
                Some(format!("data:{};base64,{}", mime, b64))
            } else {
                None
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            // catch all
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

            Some(format!(
                "artwork://local?p={}",
                percent_encoding::utf8_percent_encode(&abs_str, QUERY_VAL)
            ))
        }
    })
}
