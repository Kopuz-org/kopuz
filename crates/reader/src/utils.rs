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
    let candidates = [
        "cover",
        "folder",
        "album",
        "thumbnail",
        "default",
        "hqdefault",
        "maxresdefault",
        "preview",
    ];
    let extensions = ["jpg", "jpeg", "png", "webp"];

    let entries = std::fs::read_dir(dir).ok()?;
    let mut fallback_image = None;
    let mut has_multiple_fallbacks = false;
    let mut best_idx: Option<usize> = None;
    let mut best_path = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };

        if extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
            if let Some(pos) = candidates.iter().position(|c| c.eq_ignore_ascii_case(stem)) {
                if best_idx.is_none() || pos < best_idx.unwrap() {
                    best_idx = Some(pos);
                    best_path = Some(path);
                }
            } else if is_safe_fallback_cover(stem) {
                if fallback_image.is_some() {
                    has_multiple_fallbacks = true;
                    fallback_image = None;
                } else if !has_multiple_fallbacks {
                    fallback_image = Some(path);
                }
            }
        }
    }
    best_path.or(fallback_image)
}

fn is_safe_fallback_cover(stem: &str) -> bool {
    !matches!(
        stem.to_ascii_lowercase().as_str(),
        "artist"
            | "background"
            | "back"
            | "banner"
            | "booklet"
            | "cd"
            | "disc"
            | "label"
            | "logo"
            | "spine"
    )
}

pub fn is_artist_image_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "artist.jpg" | "artist.jpeg" | "artist.png" | "artist.webp"
            )
        })
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_find_folder_cover() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir_path = std::env::temp_dir().join(format!("kopuz_test_dir_{nanos}"));
        std::fs::create_dir_all(&dir_path).unwrap();

        // 1. Empty directory
        assert!(find_folder_cover(&dir_path).is_none());

        // 2. Directory with only non-image files
        File::create(dir_path.join("song.mp3")).unwrap();
        File::create(dir_path.join("readme.txt")).unwrap();
        assert!(find_folder_cover(&dir_path).is_none());

        // 3. Directory with generic/fallback image
        let random_image = dir_path.join("random_picture.jpg");
        File::create(&random_image).unwrap();
        assert_eq!(find_folder_cover(&dir_path), Some(random_image.clone()));

        // 4. Directory with high-priority candidate image
        let cover_image = dir_path.join("cover.png");
        File::create(&cover_image).unwrap();
        // Should prefer "cover.png" over "random_picture.jpg"
        assert_eq!(find_folder_cover(&dir_path), Some(cover_image.clone()));

        // 5. Directory with other candidate (e.g. hqdefault)
        // Clean up and recreate with hqdefault
        std::fs::remove_file(cover_image).unwrap();
        std::fs::remove_file(random_image).unwrap();
        let hq_image = dir_path.join("hqdefault.webp");
        File::create(&hq_image).unwrap();
        assert_eq!(find_folder_cover(&dir_path), Some(hq_image));

        // 6. Ambiguous generic images should not be chosen arbitrarily
        std::fs::remove_file(dir_path.join("hqdefault.webp")).unwrap();
        File::create(dir_path.join("random_picture.jpg")).unwrap();
        File::create(dir_path.join("other_picture.png")).unwrap();
        assert!(find_folder_cover(&dir_path).is_none());

        // Clean up
        let _ = std::fs::remove_dir_all(&dir_path);
    }
}
