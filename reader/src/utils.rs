use std::fs;
use std::path::{Path, PathBuf};

fn detect_image_extension(data: &[u8]) -> &'static str {
    if data.len() >= 12 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        "png"
    } else if data.len() >= 3 && data[..3] == [0xFF, 0xD8, 0xFF] {
        "jpg"
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        "webp"
    } else if data.len() >= 6 && (data[..6] == *b"GIF87a" || data[..6] == *b"GIF89a") {
        "gif"
    } else if data.len() >= 2 && data[..2] == [0x42, 0x4D] {
        "bmp"
    } else {
        "jpg"
    }
}

fn remove_stale_cover_variants(album_id: &str, cache_dir: &Path, keep_path: &Path) {
    for extension in ["jpg", "png", "webp", "gif", "bmp", "tif"] {
        let candidate = cache_dir.join(format!("{album_id}.{extension}"));
        if candidate != keep_path {
            let _ = fs::remove_file(candidate);
        }
    }
}

pub fn find_folder_cover(dir: &Path) -> Option<PathBuf> {
    let candidates = ["cover.jpg", "cover.png", "folder.jpg", "album.jpg"];

    for name in candidates {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub fn save_cover(
    album_id: &str,
    data: &[u8],
    extension: Option<&str>,
    cache_dir: &Path,
) -> std::io::Result<PathBuf> {
    fs::create_dir_all(cache_dir)?;
    let extension = extension.unwrap_or_else(|| detect_image_extension(data));
    let path = cache_dir.join(format!("{album_id}.{extension}"));

    remove_stale_cover_variants(album_id, cache_dir, &path);
    fs::write(&path, data)?;
    Ok(path)
}
