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

/// A picture named by a path or a URL rather than a ref, which is what the
/// user's own custom background and a playlist's picked cover still are.
pub fn stored(value: Option<&str>, size: Size) -> Option<CoverUrl> {
    let stored = value.map(str::trim).filter(|value| !value.is_empty())?;
    Some(utils::cover_url_from_string(match size {
        Size::Full => at_full_size(stored),
        Size::Thumb => stored.to_string(),
    }))
}

pub fn for_track(track: &api::TrackInfo, size: Size) -> Option<CoverUrl> {
    url(track.artwork.as_ref(), size)
}

pub fn for_album(album: &api::AlbumInfo, size: Size) -> Option<CoverUrl> {
    url(album.artwork.as_ref(), size)
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

#[cfg(test)]
mod tests {
    use super::at_full_size;

    #[test]
    fn our_own_urls_carry_the_hq_flag() {
        assert_eq!(
            at_full_size("artwork://local?p=%2Fcover.jpg"),
            "artwork://local?p=%2Fcover.jpg&hq=1"
        );
        assert_eq!(
            at_full_size("http://artwork.dioxus.localhost/local?p=C%3A%5Ccover.jpg"),
            "http://artwork.dioxus.localhost/local?p=C%3A%5Ccover.jpg&hq=1"
        );
    }

    /// A picture that is not the daemon's to serve is left exactly as it is:
    /// resizing a provider's URL is the daemon's business now, and guessing at
    /// one here would produce a request nothing answers.
    #[test]
    fn a_url_from_elsewhere_is_left_alone() {
        let original = "https://music.example/rest/getCoverArt.view?id=cover-1&size=80";
        assert_eq!(at_full_size(original), original);
    }
}
