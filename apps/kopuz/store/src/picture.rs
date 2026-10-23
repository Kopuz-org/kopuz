//! A thumbnail's full-size picture, put on the clipboard or written to a file the user names.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use api::{ArtworkData, ArtworkRequest, ArtworkTarget, NoticeLevel};
use gpui_kit::{App, ClipboardEntry, ClipboardItem, Image, ImageFormat, SharedString};
use link::Link;

use crate::{Notice, Notices};

/// What a picture is of, which is what its file is named after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pictured {
    Album {
        artist: SharedString,
        title: SharedString,
    },
    Track {
        artist: SharedString,
        title: SharedString,
    },
    Playlist(SharedString),
    Artist(SharedString),
}

/// Past this many bytes a name no longer fits a filesystem's 255, once an extension is added.
const STEM_MAX: usize = 200;

impl Pictured {
    /// The suggested file name without its extension: `Artist - Album`, `Artist - Title`, or the name.
    pub fn stem(&self) -> String {
        let raw = match self {
            Self::Album { artist, title } | Self::Track { artist, title } => {
                credited(artist, title)
            }
            Self::Playlist(name) | Self::Artist(name) => name.to_string(),
        };
        let stem = clean(&raw);
        match stem.is_empty() {
            true => "artwork".to_string(),
            false => stem,
        }
    }
}

fn credited(artist: &str, title: &str) -> String {
    match (artist.trim(), title.trim()) {
        ("", title) => title.to_string(),
        (artist, "") => artist.to_string(),
        (artist, title) => format!("{artist} - {title}"),
    }
}

/// Drops what a file name cannot hold on any of the desktops, and the dots Windows trims itself.
fn clean(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .filter(|c| !c.is_control())
        .collect();
    let mut stem = kept.trim().trim_end_matches('.').trim_end().to_string();
    if stem.len() > STEM_MAX {
        let mut end = STEM_MAX;
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        stem.truncate(end);
        stem = stem.trim_end().trim_end_matches('.').to_string();
    }
    stem
}

/// The extension the daemon's content type names, else the one the bytes themselves say.
pub fn extension(content_type: &str, bytes: &[u8]) -> &'static str {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        _ => match image::guess_format(bytes) {
            Ok(image::ImageFormat::Png) => "png",
            Ok(image::ImageFormat::WebP) => "webp",
            Ok(image::ImageFormat::Jpeg) | Err(_) => "jpg",
            Ok(other) => other.extensions_str().first().copied().unwrap_or("jpg"),
        },
    }
}

/// The name the save dialog opens with.
pub fn file_name(pictured: &Pictured, data: &ArtworkData) -> String {
    format!(
        "{}.{}",
        pictured.stem(),
        extension(&data.content_type, &data.bytes)
    )
}

/// Why a copy or a save did not happen.
#[derive(Debug)]
pub enum PictureError {
    Fetch(api::ApiError),
    Decode(image::ImageError),
    Prompt(String),
    Write(std::io::Error),
    NotTaken,
}

impl PictureError {
    fn notice(&self, key: &str) -> Notice {
        let detail: SharedString = match self {
            Self::Fetch(error) => crate::human(error),
            Self::Decode(error) => error.to_string().into(),
            Self::Prompt(error) => error.clone().into(),
            Self::Write(error) => error.to_string().into(),
            Self::NotTaken => "the clipboard did not take the image".into(),
        };
        let message = i18n::t_with(key, &[("error", detail.to_string().into())]);
        match self {
            Self::Fetch(error) => Notice::failure(error, message),
            _ => Notice::new(NoticeLevel::Error, message),
        }
    }
}

/// PNG as it came, or decoded and re-encoded when it is anything else.
fn as_png(data: ArtworkData) -> Result<Vec<u8>, PictureError> {
    if image::guess_format(&data.bytes).ok() == Some(image::ImageFormat::Png) {
        return Ok(data.bytes);
    }
    let decoded = image::load_from_memory(&data.bytes).map_err(PictureError::Decode)?;
    let mut png = Vec::new();
    decoded
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(PictureError::Decode)?;
    Ok(png)
}

fn full_size(
    target: ArtworkTarget,
    cx: &App,
) -> impl Future<Output = Result<ArtworkData, PictureError>> + Send + 'static + use<> {
    let call = Link::global(cx)
        .call(move |api| async move { api.artwork(ArtworkRequest { target, hq: true }).await });
    async move { call.await.map_err(PictureError::Fetch) }
}

/// Put `target`'s full-size picture on the clipboard as a PNG.
pub fn copy_picture(target: ArtworkTarget, cx: &mut App) {
    let fetch = full_size(target, cx);
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        let png = match fetch.await {
            Ok(data) => executor.spawn(async move { as_png(data) }).await,
            Err(error) => Err(error),
        };
        let image = match png {
            Ok(png) => Image::from_bytes(ImageFormat::Png, png),
            Err(error) => {
                cx.update(|cx| Notices::push(error.notice("image_copy_failed"), cx));
                return;
            }
        };
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_image(&image)));
        cx.background_executor().timer(CLIPBOARD_SETTLE).await;
        cx.update(|cx| match holds(cx.read_from_clipboard(), &image) {
            true => Notices::info(i18n::t("image_copied"), cx),
            false => Notices::push(PictureError::NotTaken.notice("image_copy_failed"), cx),
        });
    })
    .detach();
}

/// Wayland makes a new selection current only after a compositor round trip, so it is read back after this.
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(250);

/// Whether the clipboard serves `image`; the write itself reports nothing, and drops it without a focused window.
fn holds(item: Option<ClipboardItem>, image: &Image) -> bool {
    item.is_some_and(|item| {
        item.entries().iter().any(|entry| match entry {
            ClipboardEntry::Image(held) => held.format == image.format && held.bytes == image.bytes,
            _ => false,
        })
    })
}

/// `~/Pictures`, or home when there is none.
fn save_dir() -> PathBuf {
    let Some(base) = directories::BaseDirs::new() else {
        return PathBuf::from(".");
    };
    let pictures = base.home_dir().join("Pictures");
    match pictures.is_dir() {
        true => pictures,
        false => base.home_dir().to_path_buf(),
    }
}

/// Ask where to keep `target`'s full-size picture, then write its bytes there untouched.
pub fn save_picture(target: ArtworkTarget, pictured: Pictured, cx: &mut App) {
    let fetch = full_size(target, cx);
    cx.spawn(async move |cx| {
        let data = match fetch.await {
            Ok(data) => data,
            Err(error) => {
                cx.update(|cx| Notices::push(error.notice("image_save_failed"), cx));
                return;
            }
        };
        let name = file_name(&pictured, &data);
        let prompt = cx.update(|cx| cx.prompt_for_new_path(&save_dir(), Some(&name)));
        let path = match prompt.await {
            Ok(Ok(Some(path))) => path,
            Ok(Ok(None)) | Err(_) => return,
            Ok(Err(error)) => {
                let error = PictureError::Prompt(format!("{error:#}"));
                cx.update(|cx| Notices::push(error.notice("image_save_failed"), cx));
                return;
            }
        };
        let written = cx
            .background_executor()
            .spawn(async move { write(&path, &data.bytes) })
            .await;
        cx.update(|cx| match written {
            Ok(()) => Notices::info(i18n::t("image_saved"), cx),
            Err(error) => Notices::push(error.notice("image_save_failed"), cx),
        });
    })
    .detach();
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), PictureError> {
    std::fs::write(path, bytes).map_err(PictureError::Write)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(artist: &str, title: &str) -> Pictured {
        Pictured::Album {
            artist: artist.to_string().into(),
            title: title.to_string().into(),
        }
    }

    #[test]
    fn an_album_is_named_by_its_artist_and_title() {
        assert_eq!(
            album("Daft Punk", "Discovery").stem(),
            "Daft Punk - Discovery"
        );
        let track = Pictured::Track {
            artist: "Björk".into(),
            title: "Jóga".into(),
        };
        assert_eq!(track.stem(), "Björk - Jóga");
    }

    #[test]
    fn a_playlist_or_artist_is_named_by_its_name() {
        assert_eq!(Pictured::Playlist("Road trip".into()).stem(), "Road trip");
        assert_eq!(Pictured::Artist("Nina Simone".into()).stem(), "Nina Simone");
    }

    #[test]
    fn a_missing_half_drops_the_dash() {
        assert_eq!(album("", "Untitled").stem(), "Untitled");
        assert_eq!(album("Someone", "  ").stem(), "Someone");
        assert_eq!(album("", "").stem(), "artwork");
    }

    #[test]
    fn what_a_file_name_cannot_hold_is_stripped() {
        assert_eq!(
            album("AC/DC", "Back in Black").stem(),
            "ACDC - Back in Black"
        );
        assert_eq!(
            album("A\\B", "Who? <Me>: \"*|\"\u{7}").stem(),
            "AB - Who Me"
        );
        assert_eq!(Pictured::Playlist("Mix...".into()).stem(), "Mix");
        assert_eq!(Pictured::Playlist("///".into()).stem(), "artwork");
    }

    #[test]
    fn a_long_name_is_cut_on_a_character() {
        let stem = Pictured::Artist("é".repeat(300).into()).stem();
        assert!(stem.len() <= STEM_MAX);
        assert!(stem.chars().all(|c| c == 'é'));
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const JPEG: &[u8] = b"\xff\xd8\xff\xe0\0\x10JFIF\0";
    const WEBP: &[u8] = b"RIFF\0\0\0\0WEBPVP8 ";

    #[test]
    fn the_content_type_names_the_extension() {
        assert_eq!(extension("image/jpeg", PNG), "jpg");
        assert_eq!(extension("image/png", JPEG), "png");
        assert_eq!(extension("image/webp", b""), "webp");
        assert_eq!(extension("IMAGE/PNG; charset=binary", b""), "png");
        assert_eq!(extension("image/jpg", b""), "jpg");
    }

    #[test]
    fn an_unknown_content_type_falls_back_to_the_bytes() {
        assert_eq!(extension("", PNG), "png");
        assert_eq!(extension("application/octet-stream", WEBP), "webp");
        assert_eq!(extension("image/x-unknown", JPEG), "jpg");
        assert_eq!(extension("", b"GIF89a"), "gif");
        assert_eq!(extension("", b"nothing at all"), "jpg");
    }

    #[test]
    fn only_the_same_image_counts_as_copied() {
        let image = Image::from_bytes(ImageFormat::Png, PNG.to_vec());
        assert!(holds(Some(ClipboardItem::new_image(&image)), &image));
        assert!(!holds(None, &image));
        assert!(!holds(Some(ClipboardItem::new_string("x".into())), &image));
        let other = Image::from_bytes(ImageFormat::Jpeg, PNG.to_vec());
        assert!(!holds(Some(ClipboardItem::new_image(&other)), &image));
    }

    #[test]
    fn the_file_name_joins_stem_and_extension() {
        let data = ArtworkData {
            content_type: "image/png".into(),
            bytes: PNG.to_vec(),
        };
        assert_eq!(
            file_name(&album("Air", "Moon Safari"), &data),
            "Air - Moon Safari.png"
        );
    }
}
