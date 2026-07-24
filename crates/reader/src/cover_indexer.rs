use crate::metadata::extract_embedded_cover;
use crate::models::Library;
use crate::utils::{find_folder_cover, save_cover};
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PROGRESS_INTERVAL: usize = 25;

#[derive(Debug, Default)]
pub struct LocalCoverIndexReport {
    pub attempted: usize,
    pub found: usize,
    pub missing: usize,
}

/// Populate missing local album covers after the foreground metadata scan.
///
/// Only one representative track is opened per album, keeping artwork I/O
/// bounded by album count instead of track count. Callers can run this after
/// persisting metadata so the library becomes usable before artwork is ready.
pub async fn index_local_covers(
    library: &mut Library,
    cover_cache: PathBuf,
    on_progress: Arc<dyn Fn(String) + Send + Sync>,
) -> LocalCoverIndexReport {
    let representatives: HashMap<_, _> = library
        .tracks
        .iter()
        .filter_map(|track| {
            track
                .id
                .local_path()
                .map(|path| (track.album_id.as_str(), path))
        })
        .fold(HashMap::new(), |mut paths, (album_id, path)| {
            paths.entry(album_id).or_insert_with(|| path.to_path_buf());
            paths
        });
    let candidates: Vec<_> = library
        .albums
        .iter()
        .enumerate()
        .filter(|(_, album)| {
            !album.manual_cover && album.cover_path.as_ref().is_none_or(|path| !path.exists())
        })
        .filter_map(|(index, album)| {
            representatives
                .get(album.id.as_str())
                .map(|path| (index, album.id.clone(), album.title.clone(), path.clone()))
        })
        .collect();

    if candidates.is_empty() {
        return LocalCoverIndexReport::default();
    }

    let attempted = candidates.len();
    let results = tokio::task::spawn_blocking(move || {
        candidates
            .into_iter()
            .enumerate()
            .map(|(position, (index, album_id, title, path))| {
                if (position + 1).is_multiple_of(PROGRESS_INTERVAL) || position + 1 == attempted {
                    on_progress(format!("Indexing cover: {title}"));
                }
                let cover = resolve_local_cover(&path, &album_id, &cover_cache);
                (index, cover)
            })
            .collect::<Vec<_>>()
    })
    .await;

    let Ok(results) = results else {
        tracing::warn!("local cover indexing task failed");
        return LocalCoverIndexReport {
            attempted,
            found: 0,
            missing: attempted,
        };
    };

    let mut report = LocalCoverIndexReport {
        attempted,
        ..Default::default()
    };
    for (index, cover) in results {
        if let Some(cover) = cover {
            library.albums[index].cover_path = Some(cover);
            report.found += 1;
        } else {
            report.missing += 1;
        }
    }
    report
}

fn resolve_local_cover(track_path: &Path, album_id: &str, cover_cache: &Path) -> Option<PathBuf> {
    if let Ok(tagged) = Probe::open(track_path).and_then(Probe::read) {
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
        if let Some(picture) = extract_embedded_cover(&tagged, tag) {
            let extension = picture.mime_type().and_then(|mime_type| mime_type.ext());
            return save_cover(album_id, picture.data(), extension, cover_cache).ok();
        }
    }

    let parent = track_path.parent()?;
    let stem = track_path.file_stem().and_then(|stem| stem.to_str());
    if let Some(stem) = stem {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            let candidate = parent.join(stem).with_extension(extension);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    find_folder_cover(parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Album, Track, TrackId};
    use std::fs::File;

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("kopuz_{name}_{nanos}"))
    }

    fn album(id: &str, cover_path: Option<PathBuf>, manual_cover: bool) -> Album {
        Album {
            id: id.to_string(),
            title: id.to_string(),
            artist: "Artist".to_string(),
            genre: "Unknown".to_string(),
            year: 0,
            cover_path,
            manual_cover,
        }
    }

    fn track(path: PathBuf, album_id: &str) -> Track {
        Track {
            id: TrackId::Local(path),
            cover: None,
            album_id: album_id.to_string(),
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            artists: vec!["Artist".to_string()],
            album: album_id.to_string(),
            duration: 1,
            khz: 44_100,
            bitrate: 1_000,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
        }
    }

    #[tokio::test]
    async fn indexes_same_stem_cover_without_overwriting_manual_cover() {
        let dir = unique_test_dir("cover_index");
        let cache = dir.join("cache");
        std::fs::create_dir_all(&dir).unwrap();
        let track_path = dir.join("track.flac");
        let same_stem = dir.join("track.jpg");
        let manual = dir.join("manual.jpg");
        File::create(&track_path).unwrap();
        File::create(&same_stem).unwrap();
        File::create(&manual).unwrap();

        let mut library = Library {
            tracks: vec![
                track(track_path.clone(), "indexed"),
                track(track_path, "manual"),
            ],
            albums: vec![
                album("indexed", None, false),
                album("manual", Some(manual.clone()), true),
            ],
            ..Default::default()
        };

        let report = index_local_covers(&mut library, cache, Arc::new(|_| {})).await;

        assert_eq!(report.attempted, 1);
        assert_eq!(report.found, 1);
        assert_eq!(library.albums[0].cover_path, Some(same_stem));
        assert_eq!(library.albums[1].cover_path, Some(manual));
        assert!(library.albums[1].manual_cover);

        let _ = std::fs::remove_dir_all(dir);
    }
}
