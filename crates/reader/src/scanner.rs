use super::metadata::read;
use super::models::Library;
use super::utils::is_artist_image_file;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

fn normalize_artist_key(value: &str) -> Option<String> {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[tracing::instrument(name = "library.scan", skip(cover_cache, library, on_progress), fields(dir = %dir.display()))]
pub async fn scan_directory(
    dir: PathBuf,
    cover_cache: PathBuf,
    library: &mut Library,
    on_progress: Arc<dyn Fn(String) + Send + Sync>,
) -> std::io::Result<()> {
    library
        .local_artist_images
        .retain(|_, image_path| !image_path.starts_with(&dir));

    let existing_paths: Arc<HashSet<PathBuf>> =
        Arc::new(library.tracks.iter().map(|t| t.path.clone()).collect());

    let (all_audio, artist_image_dirs) = collect_audio_files(&dir, &existing_paths).await;
    tracing::info!(
        new_files = all_audio.len(),
        existing = existing_paths.len(),
        "scanning library directory"
    );

    let lib_arc = Arc::new(Mutex::new(std::mem::take(library)));

    let handles: Vec<_> = all_audio
        .chunks(100)
        .map(|chunk| {
            let chunk = chunk.to_vec();
            let cc = cover_cache.clone();
            let pr = on_progress.clone();
            let lb = lib_arc.clone();

            tokio::task::spawn_blocking(move || {
                let mut lib = lb.blocking_lock();
                for path in chunk {
                    if let Some(name) = path.file_name() {
                        pr(name.to_string_lossy().into_owned());
                    }
                    read(&path, &cc, &mut lib);
                }
            })
        })
        .collect();

    for handle in handles {
        if let Err(e) = handle.await {
            tracing::warn!("scan task failed: {e}");
        }
    }

    *library = Arc::try_unwrap(lib_arc)
        .unwrap_or_else(|_| panic!("library Arc should be uniquely owned after scanning"))
        .into_inner();

    for (img_dir, img_path) in artist_image_dirs {
        let artists: HashSet<String> = library
            .tracks
            .iter()
            .filter(|t| t.path.starts_with(&img_dir))
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
