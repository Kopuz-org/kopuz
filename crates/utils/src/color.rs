use color_thief::{ColorFormat, get_palette};
use image::ImageReader;
use reqwest;
use std::io::Cursor;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

fn local_artwork_path(url: &str) -> Option<String> {
    let query = url
        .strip_prefix("artwork://local")
        .or_else(|| url.strip_prefix("http://artwork.dioxus.localhost/local"))?;
    let query = query.strip_prefix('?').unwrap_or(query);
    let raw = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("p="))
        .unwrap_or(query);
    Some(
        percent_encoding::percent_decode_str(raw)
            .decode_utf8_lossy()
            .to_string(),
    )
}

pub async fn get_palette_from_url(url: &str) -> Option<Vec<Color>> {
    let bytes = if let Some(path) = local_artwork_path(url) {
        std::fs::read(path).ok()?
    } else if url.starts_with("http") {
        reqwest::get(url).await.ok()?.bytes().await.ok()?.to_vec()
    } else {
        std::fs::read(url).ok()?
    };

    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;

    let rgb = img.to_rgb8();
    let pixels = rgb.as_raw();

    let palette = get_palette(pixels, ColorFormat::Rgb, 10, 8).ok()?;

    Some(
        palette
            .into_iter()
            .map(|p| Color::new(p.r, p.g, p.b))
            .collect(),
    )
}

/// Caps a palette color's luminance so light text stays readable on the
/// gradient built from it; dark colors pass through unchanged.
fn dim_for_text_contrast(c: &Color) -> Color {
    const MAX_LUMINANCE: f32 = 90.0;
    let luminance = 0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32;
    if luminance <= MAX_LUMINANCE {
        return c.clone();
    }
    let scale = MAX_LUMINANCE / luminance;
    Color::new(
        (c.r as f32 * scale) as u8,
        (c.g as f32 * scale) as u8,
        (c.b as f32 * scale) as u8,
    )
}

pub fn get_background_style(colors: Option<&[Color]>) -> String {
    if let Some(colors) = colors
        && !colors.is_empty()
    {
        let bg_color = dim_for_text_contrast(&colors[0]);
        let mut bg_image_parts = Vec::new();
        let positions = [
            "0% 0%",
            "100% 0%",
            "100% 100%",
            "0% 100%",
            "50% 50%",
            "25% 0%",
            "75% 100%",
        ];
        for (i, c) in colors.iter().skip(1).enumerate().take(positions.len()) {
            let pos = positions[i];
            let c = dim_for_text_contrast(c);
            bg_image_parts.push(format!(
                "radial-gradient(circle at {}, rgba({}, {}, {}, 0.8) 0%, transparent 80%)",
                pos, c.r, c.g, c.b
            ));
        }

        if bg_image_parts.is_empty() {
            return format!(
                "background-color: rgb({}, {}, {}); background-image: none;",
                bg_color.r, bg_color.g, bg_color.b
            );
        } else {
            return format!(
                "background-color: rgb({}, {}, {}); background-image: {};",
                bg_color.r,
                bg_color.g,
                bg_color.b,
                bg_image_parts.join(", ")
            );
        }
    }
    "background-color: var(--color-black); background-image: none;".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_artwork_path_extracts_percent_encoded_p_param() {
        assert_eq!(
            local_artwork_path(
                "artwork://local?p=%2FMusic%2FMaster%20of%20Puppets%2Fcover.jpg&s=56"
            ),
            Some("/Music/Master of Puppets/cover.jpg".to_string())
        );
    }

    #[test]
    fn local_artwork_path_handles_windows_localhost_shim() {
        assert_eq!(
            local_artwork_path(
                "http://artwork.dioxus.localhost/local?p=C%3A%5CMusic%5Ccover.jpg&s=56"
            ),
            Some("C:\\Music\\cover.jpg".to_string())
        );
    }

    #[test]
    fn dim_for_text_contrast_dims_bright_colors() {
        let dimmed = dim_for_text_contrast(&Color::new(240, 180, 220));
        let luminance =
            0.2126 * dimmed.r as f32 + 0.7152 * dimmed.g as f32 + 0.0722 * dimmed.b as f32;
        assert!(luminance <= 90.0);
        assert!(dimmed.r > dimmed.g && dimmed.b > dimmed.g);
    }

    #[test]
    fn dim_for_text_contrast_keeps_dark_colors() {
        let dark = Color::new(40, 20, 60);
        assert_eq!(dim_for_text_contrast(&dark), dark);
    }

    #[test]
    fn local_artwork_path_ignores_remote_urls() {
        assert_eq!(local_artwork_path("https://example.com/cover.jpg"), None);
        assert_eq!(local_artwork_path("http://example.com/cover.jpg"), None);
    }
}
