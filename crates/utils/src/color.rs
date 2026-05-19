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

pub async fn get_palette_from_url(url: &str) -> Option<Vec<Color>> {
    let bytes = if url.starts_with("http") {
        reqwest::get(url).await.ok()?.bytes().await.ok()?.to_vec()
    } else {
        let path = if url.starts_with("artwork://local") {
            let decoded = percent_encoding::percent_decode_str(&url[15..]).decode_utf8_lossy();
            decoded.to_string()
        } else {
            url.to_string()
        };
        std::fs::read(path).ok()?
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

pub fn get_background_style(colors: Option<&[Color]>) -> String {
    if let Some(colors) = colors {
        if !colors.is_empty() {
            let bg_color = &colors[0];
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
    }
    "background-color: var(--color-black); background-image: none;".to_string()
}
