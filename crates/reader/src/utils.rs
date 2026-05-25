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
    let candidates = ["cover", "folder", "album"];
    let extensions = ["jpg", "jpeg", "png", "webp"];

    let entries = std::fs::read_dir(dir).ok()?;
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

        if candidates.iter().any(|c| c.eq_ignore_ascii_case(stem))
            && extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
        {
            return Some(path);
        }
    }
    None
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
    use super::{detect_image_extension, find_folder_cover, is_artist_image_file, save_cover};
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kopuz_{name}_{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png_bytes() -> &'static [u8] {
        b"\x89PNG\r\n\x1a\nrest"
    }

    fn setup() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn touch(dir: &Path, name: &str) {
        File::create(dir.join(name)).unwrap();
    }

    #[test]
    fn detect_image_extension_recognizes_common_formats() {
        assert_eq!(detect_image_extension(png_bytes()), "png");
        assert_eq!(detect_image_extension(&[0xFF, 0xD8, 0xFF, 0x00]), "jpg");
        assert_eq!(detect_image_extension(b"RIFFxxxxWEBPrest"), "webp");
        assert_eq!(detect_image_extension(b"GIF89arest"), "gif");
        assert_eq!(detect_image_extension(b"GIF87arest"), "gif");
        assert_eq!(detect_image_extension(&[0x42, 0x4D, 0x00]), "bmp");
    }

    #[test]
    fn detect_image_extension_falls_back_to_jpg_for_unknown_bytes() {
        assert_eq!(detect_image_extension(b"not-an-image"), "jpg");
        assert_eq!(detect_image_extension(b""), "jpg");
        assert_eq!(detect_image_extension(b"12"), "jpg"); // Very short
    }

    #[test]
    fn detect_image_extension_rejects_partial_headers() {
        assert_eq!(detect_image_extension(b"\x89PNG\r\n"), "jpg");
        assert_eq!(detect_image_extension(b"GIF8"), "jpg");
        assert_eq!(detect_image_extension(b"RIFFxxxxWEB"), "jpg");
    }

    #[test]
    fn find_folder_cover_finds_cover_jpg() {
        let tmp = setup();
        touch(tmp.path(), "cover.jpg");
        assert!(find_folder_cover(tmp.path()).is_some());
    }

    #[test]
    fn find_folder_cover_matches_case_insensitive_stem_and_extension() {
        let tmp = setup();
        touch(tmp.path(), "Cover.JPG");
        let result = find_folder_cover(tmp.path()).unwrap();
        assert_eq!(result.file_name().unwrap(), "Cover.JPG");
    }

    #[test]
    fn find_folder_cover_finds_folder_and_album_stems() {
        let tmp = setup();
        touch(tmp.path(), "folder.png");
        let result = find_folder_cover(tmp.path()).unwrap();
        assert_eq!(result.file_name().unwrap(), "folder.png");

        let tmp2 = setup();
        touch(tmp2.path(), "album.webp");
        let result = find_folder_cover(tmp2.path()).unwrap();
        assert_eq!(result.file_name().unwrap(), "album.webp");
    }

    #[test]
    fn find_folder_cover_skips_wrong_stem_and_extension() {
        let tmp = setup();
        touch(tmp.path(), "artwork.jpg");
        touch(tmp.path(), "cover.bmp");
        assert!(find_folder_cover(tmp.path()).is_none());
    }

    #[test]
    fn find_folder_cover_skips_no_extension_and_extra_dot_stem() {
        let tmp = setup();
        touch(tmp.path(), "cover");
        touch(tmp.path(), "cover.old.jpg");
        assert!(find_folder_cover(tmp.path()).is_none());
    }

    #[test]
    fn find_folder_cover_ignores_subdirectories_named_like_images() {
        let tmp = setup();
        fs::create_dir(tmp.path().join("cover.jpg")).unwrap();
        assert!(find_folder_cover(tmp.path()).is_none());
    }

    #[test]
    fn find_folder_cover_returns_none_for_empty_and_missing_dirs() {
        let tmp = setup();
        assert!(find_folder_cover(tmp.path()).is_none());

        let missing = tmp.path().join("missing");
        assert!(find_folder_cover(&missing).is_none());
    }

    #[test]
    fn find_folder_cover_returns_matching_path() {
        let tmp = setup();
        touch(tmp.path(), "folder.jpeg");
        let result = find_folder_cover(tmp.path()).unwrap();
        assert_eq!(result.file_name().unwrap(), "folder.jpeg");
    }

    #[cfg(unix)]
    #[test]
    fn find_folder_cover_accepts_valid_symlink_and_skips_broken_one() {
        let tmp = setup();
        let dir = tmp.path();

        let real = dir.join("real_cover.jpg");
        File::create(&real).unwrap();
        std::os::unix::fs::symlink(&real, dir.join("cover.jpg")).unwrap();

        let result = find_folder_cover(dir).unwrap();
        assert_eq!(result.file_name().unwrap(), "cover.jpg");

        let broken_tmp = setup();
        std::os::unix::fs::symlink(
            "/nonexistent/cover.jpg",
            broken_tmp.path().join("cover.jpg"),
        )
        .unwrap();
        assert!(find_folder_cover(broken_tmp.path()).is_none());
    }

    #[test]
    fn find_folder_cover_returns_none_if_no_cover_found() {
        let dir = temp_dir("folder_cover_none");
        fs::write(dir.join("random.jpg"), b"random").unwrap();

        let cover = find_folder_cover(&dir);

        assert_eq!(cover, None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn is_artist_image_file_is_case_insensitive_and_name_specific() {
        assert!(is_artist_image_file(Path::new("/music/ARTIST.PNG")));
        assert!(is_artist_image_file(Path::new("/music/artist.jpeg")));
        assert!(!is_artist_image_file(Path::new("/music/artist.gif")));
        assert!(!is_artist_image_file(Path::new("/music/folder.jpg")));
        // Edge cases
        assert!(!is_artist_image_file(Path::new("")));
        assert!(!is_artist_image_file(Path::new("/music/")));
        assert!(!is_artist_image_file(Path::new("/music/myartist.jpg")));
    }

    #[test]
    fn is_artist_image_file_rejects_extra_suffixes_and_missing_extensions() {
        assert!(!is_artist_image_file(Path::new("/music/artist.jpg.bak")));
        assert!(!is_artist_image_file(Path::new("/music/artist.")));
        assert!(!is_artist_image_file(Path::new("/music/artist")));
        assert!(!is_artist_image_file(Path::new("/music/artist .jpg")));
    }

    #[test]
    fn save_cover_respects_explicit_extension_and_removes_stale_variants() {
        let dir = temp_dir("save_cover_cleanup");
        let stale = dir.join("alb_test.jpg");
        fs::write(&stale, b"old").unwrap();

        let saved = save_cover("alb_test", png_bytes(), Some("png"), &dir).unwrap();

        assert_eq!(saved, dir.join("alb_test.png"));
        assert_eq!(fs::read(&saved).unwrap(), png_bytes());
        assert!(!stale.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_cover_detects_extension_when_not_provided() {
        let tmp = setup();
        let saved = save_cover("alb_auto", png_bytes(), None, tmp.path()).unwrap();

        assert_eq!(saved.file_name().unwrap(), "alb_auto.png");
        assert_eq!(fs::read(saved).unwrap(), png_bytes());
    }

    #[test]
    fn save_cover_creates_missing_cache_directory() {
        let tmp = setup();
        let nested = tmp.path().join("covers").join("nested");

        let saved = save_cover("alb_nested", png_bytes(), Some("png"), &nested).unwrap();

        assert!(nested.exists());
        assert_eq!(saved, nested.join("alb_nested.png"));
    }

    #[test]
    fn save_cover_removes_multiple_stale_variants_but_keeps_target_file() {
        let tmp = setup();
        for ext in ["jpg", "gif", "bmp", "tif"] {
            fs::write(tmp.path().join(format!("alb_multi.{ext}")), b"old").unwrap();
        }

        let saved = save_cover("alb_multi", b"RIFFxxxxWEBPrest", None, tmp.path()).unwrap();

        assert_eq!(saved.file_name().unwrap(), "alb_multi.webp");
        assert!(saved.exists());
        for ext in ["jpg", "gif", "bmp", "tif"] {
            assert!(!tmp.path().join(format!("alb_multi.{ext}")).exists());
        }
    }
}
