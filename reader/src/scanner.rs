use super::metadata::read;
use super::models::Library;
use super::utils::is_artist_image_file;
use async_recursion::async_recursion;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;

pub async fn scan_directory(
    dir: PathBuf,
    cover_cache: PathBuf,
    library: &mut Library,
    on_progress: Arc<dyn Fn(String) + Send + Sync>,
) -> std::io::Result<()> {
    let existing_paths: HashSet<PathBuf> = library.tracks.iter().map(|t| t.path.clone()).collect();
    let existing_artists_by_dir = build_existing_artist_index(&library.tracks);
    scan_directory_internal(
        dir,
        cover_cache,
        library,
        &existing_paths,
        &existing_artists_by_dir,
        on_progress,
    )
        .await
        .map(|_| ())
}

#[async_recursion]
async fn scan_directory_internal(
    dir: PathBuf,
    cover_cache: PathBuf,
    library: &mut Library,
    existing_paths: &HashSet<PathBuf>,
    existing_artists_by_dir: &HashMap<PathBuf, HashSet<String>>,
    on_progress: Arc<dyn Fn(String) + Send + Sync>,
) -> std::io::Result<HashSet<String>> {
    let mut entries = match fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return Ok(HashSet::new()),
    };

    let mut audio_files = Vec::new();
    let mut sub_dirs = Vec::new();
    let mut artist_image = None;
    let mut artists_in_subtree = existing_artists_by_dir.get(&dir).cloned().unwrap_or_default();

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            sub_dirs.push(path);
        } else if is_audio_file(&path) {
            if !existing_paths.contains(&path) {
                audio_files.push(path);
            }
        } else if artist_image.is_none() && is_artist_image_file(&path) {
            artist_image = Some(path);
        }
    }

    if !audio_files.is_empty() {
        let mut lib = std::mem::take(library);
        let cover_cache_clone = cover_cache.clone();
        let progress = on_progress.clone();

        let (updated_lib, scanned_artists) = tokio::task::spawn_blocking(move || {
            let mut scanned_artists = HashSet::new();
            for path in audio_files {
                if let Some(name) = path.file_name() {
                    progress(name.to_string_lossy().into_owned());
                }
                if let Some(track) = read(&path, &cover_cache_clone, &mut lib) {
                    if !track.artist.trim().is_empty() {
                        scanned_artists.insert(track.artist);
                    }
                    for artist in track.artists {
                        if !artist.trim().is_empty() {
                            scanned_artists.insert(artist);
                        }
                    }
                }
            }
            (lib, scanned_artists)
        })
        .await
        .unwrap();

        *library = updated_lib;
        artists_in_subtree.extend(scanned_artists);
    }

    for sub_dir in sub_dirs {
        let child_artists = scan_directory_internal(
            sub_dir,
            cover_cache.clone(),
            library,
            existing_paths,
            existing_artists_by_dir,
            on_progress.clone(),
        )
        .await;

        if let Ok(child_artists) = child_artists {
            artists_in_subtree.extend(child_artists);
        }
    }

    if let Some(artist_image_path) = artist_image {
        for artist in &artists_in_subtree {
            library
                .local_artist_images
                .entry(artist.clone())
                .or_insert_with(|| artist_image_path.clone());
        }
    }

    Ok(artists_in_subtree)
}

fn build_existing_artist_index(
    tracks: &[super::models::Track],
) -> HashMap<PathBuf, HashSet<String>> {
    let mut artists_by_dir = HashMap::new();

    for track in tracks {
        let artists = collect_track_artists(track);
        if artists.is_empty() {
            continue;
        }

        let mut current = track.path.parent();
        while let Some(dir) = current {
            artists_by_dir
                .entry(dir.to_path_buf())
                .or_insert_with(HashSet::new)
                .extend(artists.iter().cloned());
            current = dir.parent();
        }
    }

    artists_by_dir
}

fn collect_track_artists(track: &super::models::Track) -> HashSet<String> {
    let mut artists = HashSet::new();

    if !track.artist.trim().is_empty() {
        artists.insert(track.artist.clone());
    }

    for artist in &track.artists {
        if !artist.trim().is_empty() {
            artists.insert(artist.clone());
        }
    }

    artists
}

pub fn is_audio_file(path: &Path) -> bool {
    let extensions = ["mp3", "flac", "m4a", "wav", "ogg", "opus", "mp4"];
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| extensions.contains(&s.to_lowercase().as_str()))
        .unwrap_or(false)
}
