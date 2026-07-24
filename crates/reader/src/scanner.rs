use super::metadata::{ScannedTrack, read_metadata};
use super::models::Library;
use super::utils::is_artist_image_file;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_METADATA_WORKERS: usize = 4;
const MIN_FILES_PER_WORKER: usize = 100;
const PROGRESS_INTERVAL: usize = 25;

fn normalize_artist_key(value: &str) -> Option<String> {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[tracing::instrument(name = "library.scan", skip(_cover_cache, library, on_progress), fields(dir = %dir.display()))]
pub async fn scan_directory(
    dir: PathBuf,
    _cover_cache: PathBuf,
    library: &mut Library,
    on_progress: Arc<dyn Fn(String) + Send + Sync>,
) -> std::io::Result<()> {
    library
        .local_artist_images
        .retain(|_, image_path| !image_path.starts_with(&dir));

    let existing_paths: Arc<HashSet<PathBuf>> = Arc::new(
        library
            .tracks
            .iter()
            .filter_map(|t| t.id.local_path().map(Path::to_path_buf))
            .collect(),
    );

    let (mut all_audio, artist_image_dirs) = collect_audio_files(&dir, &existing_paths).await;
    all_audio.sort_unstable();
    tracing::info!(
        new_files = all_audio.len(),
        existing = existing_paths.len(),
        "scanning library directory"
    );

    let available_workers = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_METADATA_WORKERS);
    let worker_count = all_audio
        .len()
        .div_ceil(MIN_FILES_PER_WORKER)
        .clamp(1, available_workers);
    let chunk_size = all_audio.len().max(1).div_ceil(worker_count);
    let completed = Arc::new(AtomicUsize::new(0));
    let total = all_audio.len();
    let handles: Vec<_> = all_audio
        .chunks(chunk_size)
        .map(|chunk| {
            let chunk = chunk.to_vec();
            let pr = on_progress.clone();
            let completed = completed.clone();

            tokio::task::spawn_blocking(move || {
                let mut scanned = Vec::with_capacity(chunk.len());
                for path in chunk {
                    let result = read_metadata(&path);
                    let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if (count.is_multiple_of(PROGRESS_INTERVAL) || count == total)
                        && let Some(name) = path.file_name()
                    {
                        pr(name.to_string_lossy().into_owned());
                    }
                    scanned.extend(result);
                }
                scanned
            })
        })
        .collect();

    let mut scanned_tracks = Vec::with_capacity(all_audio.len());
    for handle in handles {
        match handle.await {
            Ok(mut scanned) => scanned_tracks.append(&mut scanned),
            Err(e) => tracing::warn!("scan task failed: {e}"),
        }
    }

    merge_scanned_tracks(library, scanned_tracks);

    for (img_dir, img_path) in artist_image_dirs {
        let artists: HashSet<String> = library
            .tracks
            .iter()
            .filter(|t| t.id.local_path().is_some_and(|p| p.starts_with(&img_dir)))
            .filter_map(|t| {
                let mut set = HashSet::new();
                if let Some(a) = normalize_artist_key(&t.artist) {
                    set.insert(a);
                }
                for a in &t.artists {
                    if let Some(a) = normalize_artist_key(a) {
                        set.insert(a);
                    }
                }
                if set.is_empty() { None } else { Some(set) }
            })
            .flatten()
            .collect();

        if artists.len() == 1
            && let Some(artist) = artists.iter().next()
        {
            library
                .local_artist_images
                .entry(artist.clone())
                .or_insert(img_path);
        }
    }

    tracing::info!(total_tracks = library.tracks.len(), "library scan complete");
    Ok(())
}

fn merge_scanned_tracks(library: &mut Library, scanned_tracks: Vec<ScannedTrack>) {
    let mut track_indexes: HashMap<_, _> = library
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| (track.id.clone(), index))
        .collect();
    let mut album_indexes: HashMap<_, _> = library
        .albums
        .iter()
        .enumerate()
        .map(|(index, album)| (album.id.clone(), index))
        .collect();

    for scanned in scanned_tracks {
        if let Some(&index) = track_indexes.get(&scanned.track.id) {
            library.tracks[index] = scanned.track;
        } else {
            let index = library.tracks.len();
            track_indexes.insert(scanned.track.id.clone(), index);
            library.tracks.push(scanned.track);
        }

        if !album_indexes.contains_key(&scanned.album.id) {
            let index = library.albums.len();
            album_indexes.insert(scanned.album.id.clone(), index);
            library.albums.push(scanned.album);
        }
    }
}

async fn collect_audio_files(
    root: &Path,
    existing_paths: &HashSet<PathBuf>,
) -> (Vec<PathBuf>, Vec<(PathBuf, PathBuf)>) {
    let mut audio_files = Vec::new();
    let mut artist_image_dirs = Vec::new();
    let mut dirs = vec![root.to_path_buf()];

    while let Some(dir) = dirs.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let is_dir = tokio::fs::metadata(&path)
                .await
                .map(|t| t.is_dir())
                .unwrap_or(false);
            if is_dir {
                dirs.push(path);
            } else if is_artist_image_file(&path) {
                artist_image_dirs.push((dir.clone(), path));
            } else if is_audio_file(&path) && !existing_paths.contains(&path) {
                audio_files.push(path);
            }
        }
    }

    (audio_files, artist_image_dirs)
}

pub fn is_audio_file(path: &Path) -> bool {
    let extensions = ["mp3", "flac", "m4a", "wav", "ogg", "opus", "mp4", "mka"];
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| extensions.iter().any(|e| s.eq_ignore_ascii_case(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Album, Track, TrackId};

    fn track(path: &str, album_id: &str, title: &str) -> Track {
        Track {
            id: TrackId::Local(path.into()),
            cover: None,
            album_id: album_id.to_string(),
            title: title.to_string(),
            artist: "Artist".to_string(),
            artists: vec!["Artist".to_string()],
            album: "Album".to_string(),
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

    fn album(id: &str) -> Album {
        Album {
            id: id.to_string(),
            title: "Album".to_string(),
            artist: "Artist".to_string(),
            genre: "Unknown".to_string(),
            year: 0,
            cover_path: None,
            manual_cover: false,
        }
    }

    #[test]
    fn merge_is_indexed_and_keeps_existing_album_state() {
        let manual_cover = PathBuf::from("/covers/manual.jpg");
        let mut existing_album = album("album-a");
        existing_album.cover_path = Some(manual_cover.clone());
        existing_album.manual_cover = true;
        let mut library = Library {
            tracks: vec![track("/music/old.flac", "album-a", "Old")],
            albums: vec![existing_album],
            ..Default::default()
        };

        merge_scanned_tracks(
            &mut library,
            vec![
                ScannedTrack {
                    track: track("/music/old.flac", "album-a", "Updated"),
                    album: album("album-a"),
                },
                ScannedTrack {
                    track: track("/music/new.flac", "album-b", "New"),
                    album: album("album-b"),
                },
            ],
        );

        assert_eq!(library.tracks.len(), 2);
        assert_eq!(library.tracks[0].title, "Updated");
        assert_eq!(library.albums.len(), 2);
        assert_eq!(library.albums[0].cover_path, Some(manual_cover));
        assert!(library.albums[0].manual_cover);
    }
}
