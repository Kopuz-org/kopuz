//! Where a picture comes from.
//!
//! A row does not carry an image, it carries a reference: which entity the
//! picture belongs to and a version that changes when the picture does. This
//! is the only place that knows how that becomes something a view can render,
//! which is why no page builds an image URL and none holds the credentials a
//! server cover would need.
//!
//! On this frontend that rendering is a URL the artwork protocol handler
//! answers from the daemon's bytes. A frontend without a webview asks for the
//! bytes instead, with the same ref.

use api::ArtworkRef;
use utils::CoverUrl;

/// How large the picture will be drawn. A thumbnail is what a row or a grid
/// tile needs; a full one is for the pages that paint it across the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    Thumb,
    Full,
}

/// The picture for a ref, or `None` when the entity has none -- in which case
/// the view draws its placeholder and nothing is ever requested.
pub fn url(artwork: Option<&ArtworkRef>, size: Size) -> Option<CoverUrl> {
    let artwork = artwork?;
    Some(utils::format_entity_artwork_url(
        artwork.target.kind(),
        artwork.target.id(),
        artwork.version,
        size == Size::Full,
    ))
}

/// The picture a row already carries, as the row models still hold it: the
/// same reference, spelled as the URL this frontend renders.
pub fn stored(value: Option<&str>, size: Size) -> Option<CoverUrl> {
    let stored = value.map(str::trim).filter(|value| !value.is_empty())?;
    Some(utils::cover_url_from_string(match size {
        Size::Full => at_full_size(stored),
        Size::Thumb => stored.to_string(),
    }))
}

pub fn for_track(track: &reader::Track, size: Size) -> Option<CoverUrl> {
    stored(track.cover.as_deref(), size)
}

pub fn for_album(album: &reader::Album, size: Size) -> Option<CoverUrl> {
    stored(
        album.cover_path.as_deref().and_then(|path| path.to_str()),
        size,
    )
}

/// The same picture, asked for at the size a large surface wants. Only the
/// daemon's own URLs carry the flag; anything else is left alone.
pub fn at_full_size(cover: &str) -> String {
    let ours =
        cover.starts_with("artwork://") || cover.starts_with("http://artwork.dioxus.localhost/");
    match ours && !cover.contains("&hq=1") {
        true => format!("{cover}&hq=1"),
        false => cover.to_string(),
    }
}

/// The colours in a picture, for the surfaces that tint themselves with it.
/// The bytes come from the daemon, so this works for a cover only it can
/// fetch -- which a URL-reading palette never could.
pub async fn palette(
    api: &std::sync::Arc<dyn api::KopuzApi>,
    artwork: &ArtworkRef,
) -> Option<Vec<utils::color::Color>> {
    let data = api
        .artwork(api::ArtworkRequest {
            target: artwork.target.clone(),
            hq: false,
        })
        .await
        .ok()?;
    utils::color::palette_from_bytes(&data.bytes)
}

/// The size a surface that thinks in pixels is asking for.
pub fn size_for(max_width: u32) -> Size {
    match max_width > 512 {
        true => Size::Full,
        false => Size::Thumb,
    }
}
