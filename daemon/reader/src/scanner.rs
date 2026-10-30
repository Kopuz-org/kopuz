use super::metadata::{
    ScannedTrack, album_credit, album_id_is_current, folder_key, is_folder_album, make_album_id,
    read_metadata, title_key, track_is_current,
};
use super::models::{Album, Library, Track};
use super::utils::is_artist_image_file;
use std::collections::{BTreeSet, HashMap, HashSet};
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
            .filter(|track| track_is_current(track))
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
    settle_local_albums(library);

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

fn merge_album_state(mut new_album: Album, existing: &Album) -> Album {
    if new_album.cover_path.is_none() || existing.manual_cover {
        new_album.cover_path = existing.cover_path.clone();
    }
    if existing.manual_cover {
        new_album.manual_cover = true;
    }
    new_album
}

fn album_metadata_key(album: &Album) -> (String, String) {
    (
        album.title.trim().to_lowercase(),
        album.artist.trim().to_lowercase(),
    )
}

pub fn merge_scanned_tracks(library: &mut Library, scanned_tracks: Vec<ScannedTrack>) {
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
    let on_disk = albums_with_files(library);
    // Safe to carry: a chosen cover, any artist-keyed (alb2_) one, or a current album's whose files all moved away; title-only ones collided.
    let legacy_album_indexes: HashMap<_, _> = library
        .albums
        .iter()
        .enumerate()
        .filter(|(_, album)| {
            if album_id_is_current(&album.id) {
                album.manual_cover && !on_disk.contains(album.id.as_str())
            } else {
                album.manual_cover || album.id.starts_with("alb2_")
            }
        })
        .map(|(index, album)| (album_metadata_key(album), index))
        .collect();

    for scanned in scanned_tracks {
        if let Some(&index) = track_indexes.get(&scanned.track.id) {
            library.tracks[index] = scanned.track;
        } else {
            let index = library.tracks.len();
            track_indexes.insert(scanned.track.id.clone(), index);
            library.tracks.push(scanned.track);
        }

        if let Some(&index) = album_indexes.get(&scanned.album.id) {
            library.albums[index] = merge_album_state(scanned.album, &library.albums[index]);
        } else {
            let mut album = scanned.album;
            if let Some(&legacy_index) = legacy_album_indexes.get(&album_metadata_key(&album)) {
                album = merge_album_state(album, &library.albums[legacy_index]);
            }
            let index = library.albums.len();
            album_indexes.insert(album.id.clone(), index);
            library.albums.push(album);
        }
    }
}

/// Albums with at least one track still on disk.
fn albums_with_files(library: &Library) -> HashSet<String> {
    library
        .tracks
        .iter()
        .filter(|track| track.id.local_path().is_some_and(Path::exists))
        .map(|track| track.album_id.clone())
        .collect()
}

/// Two tracks claiming the same disc and track number cannot be one release.
fn positions_collide(tracks: &[&Track]) -> bool {
    let mut seen = HashSet::new();
    tracks
        .iter()
        .filter_map(|track| {
            track
                .track_number
                .map(|number| (track.disc_number.unwrap_or(1), number))
        })
        .any(|position| !seen.insert(position))
}

/// Settle what one file cannot: untagged files join the folder's lone tagged release, colliding same-titled albums split by artist.
pub(crate) fn settle_local_albums(library: &mut Library) {
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (index, track) in library.tracks.iter().enumerate() {
        let Some(path) = track.id.local_path() else {
            continue;
        };
        // No album tag at all: its id is the folder alone, and there is no title to group by.
        if !is_folder_album(&track.album_id) || track.album_id == make_album_id("", None, path) {
            continue;
        }
        groups
            .entry((folder_key(path), title_key(track.album.trim())))
            .or_default()
            .push(index);
    }

    let tagged_by_folder_title = tagged_releases(library);
    let mut moves: Vec<(usize, String)> = Vec::new();
    for (key, members) in &groups {
        let tagged = tagged_by_folder_title.get(key);
        match tagged.map(BTreeSet::len) {
            Some(1) => {
                let release = tagged
                    .and_then(|ids| ids.iter().next())
                    .cloned()
                    .unwrap_or_default();
                moves.extend(members.iter().map(|&index| (index, release.clone())));
            }
            Some(_) => {}
            None => {
                let tracks: Vec<&Track> = members
                    .iter()
                    .map(|&index| &library.tracks[index])
                    .collect();
                let split = positions_collide(&tracks);
                for &index in members {
                    let track = &library.tracks[index];
                    let Some(path) = track.id.local_path() else {
                        continue;
                    };
                    let base = make_album_id(&track.album, None, path);
                    let id = if split {
                        format!("{base}_{}", title_key(track.artist.trim()))
                    } else {
                        base
                    };
                    moves.push((index, id));
                }
            }
        }
    }

    let mut album_rows: HashMap<String, usize> = library
        .albums
        .iter()
        .enumerate()
        .map(|(index, album)| (album.id.clone(), index))
        .collect();
    for (index, id) in moves {
        let old = std::mem::replace(&mut library.tracks[index].album_id, id.clone());
        if old == id || album_rows.contains_key(&id) {
            continue;
        }
        let Some(&row) = album_rows.get(&old) else {
            continue;
        };
        // The old row's cover may be another artist's, so a new album finds its own.
        let mut album = library.albums[row].clone();
        album.id = id.clone();
        album.cover_path = None;
        album.manual_cover = false;
        album_rows.insert(id, library.albums.len());
        library.albums.push(album);
    }

    let mut members: HashMap<&str, Vec<&Track>> = HashMap::new();
    for track in &library.tracks {
        members
            .entry(track.album_id.as_str())
            .or_default()
            .push(track);
    }
    let credits: Vec<(usize, String)> = library
        .albums
        .iter()
        .enumerate()
        .filter(|(_, album)| is_folder_album(&album.id))
        .filter_map(|(index, album)| {
            let tracks = members.get(album.id.as_str())?;
            Some((index, album_credit(album, tracks.iter().copied())))
        })
        .collect();
    for (index, credit) in credits {
        library.albums[index].artist = credit;
    }
}

/// The tagged releases in each (album folder, title): the albums an untagged file there could belong to.
fn tagged_releases(library: &Library) -> HashMap<(String, String), BTreeSet<String>> {
    let mut releases: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
    for track in &library.tracks {
        let Some(path) = track.id.local_path() else {
            continue;
        };
        if is_folder_album(&track.album_id) || !album_id_is_current(&track.album_id) {
            continue;
        }
        releases
            .entry((folder_key(path), title_key(track.album.trim())))
            .or_default()
            .insert(track.album_id.clone());
    }
    releases
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
            credits: Vec::new(),
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
            replay_gain: config::ReplayGainInfo::default(),
            explicit: false,
            plays: None,
            counterpart: None,
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
            artist_id: None,
            artist_key: None,
        }
    }

    #[test]
    fn merge_is_indexed_and_keeps_existing_album_state() {
        let manual_cover = PathBuf::from("/covers/manual.jpg");
        let mut existing_album = album("album-a");
        existing_album.title = "Old Album".to_string();
        existing_album.genre = "Old Genre".to_string();
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
                    album: {
                        let mut updated = album("album-a");
                        updated.title = "Updated Album".to_string();
                        updated.genre = "Updated Genre".to_string();
                        updated
                    },
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
        assert_eq!(library.albums[0].title, "Updated Album");
        assert_eq!(library.albums[0].genre, "Updated Genre");
        assert_eq!(library.albums[0].cover_path, Some(manual_cover));
        assert!(library.albums[0].manual_cover);
    }

    #[test]
    fn merge_carries_cover_state_from_matching_legacy_album() {
        let manual_cover = PathBuf::from("/covers/manual.jpg");
        let mut legacy_album = album("alb2_6_yaşar_divane");
        legacy_album.title = "Divane".to_string();
        legacy_album.artist = "Yaşar".to_string();
        legacy_album.cover_path = Some(manual_cover.clone());
        legacy_album.manual_cover = true;
        let mut scanned_album = album("alb3_6_yaşar_divane");
        scanned_album.title = "Divane".to_string();
        scanned_album.artist = "Yaşar".to_string();
        let mut library = Library {
            tracks: vec![track("/music/divane.flac", "alb2_6_yaşar_divane", "Divane")],
            albums: vec![legacy_album],
            ..Default::default()
        };

        merge_scanned_tracks(
            &mut library,
            vec![ScannedTrack {
                track: track("/music/divane.flac", "alb3_6_yaşar_divane", "Divane"),
                album: scanned_album,
            }],
        );

        assert_eq!(library.tracks[0].album_id, "alb3_6_yaşar_divane");
        assert_eq!(library.albums.len(), 2);
        assert_eq!(library.albums[1].cover_path, Some(manual_cover));
        assert!(library.albums[1].manual_cover);
    }

    /// A file as a scan files it: the id its own tags give, and an album row for that id.
    fn scanned(
        library: &mut Library,
        path: &str,
        title: &str,
        album_artist: Option<&str>,
        artist: &str,
        number: u32,
    ) {
        let id = make_album_id(title, album_artist, Path::new(path));
        let mut track = track(path, &id, &format!("t{number}"));
        track.album = title.to_string();
        track.artist = artist.to_string();
        track.track_number = Some(number);
        if !library.albums.iter().any(|row| row.id == id) {
            let mut row = album(&id);
            row.title = title.to_string();
            row.artist = album_artist.unwrap_or(artist).to_string();
            row.cover_path = Some(PathBuf::from(format!("/covers/{artist}.jpg")));
            library.albums.push(row);
        }
        library.tracks.push(track);
    }

    fn album_of<'a>(library: &'a Library, path: &str) -> &'a Album {
        let id = &library
            .tracks
            .iter()
            .find(|track| track.id.local_path() == Some(Path::new(path)))
            .expect("track")
            .album_id;
        library
            .albums
            .iter()
            .find(|album| &album.id == id)
            .expect("album row")
    }

    #[test]
    fn same_titled_albums_flat_in_one_folder_split_by_artist() {
        let mut library = Library::default();
        for (artist, file) in [("Metallica", "m"), ("2Pac", "p"), ("Guns N' Roses", "g")] {
            for number in 1..=2 {
                let path = format!("/music/{file}{number}.mp3");
                scanned(&mut library, &path, "Greatest Hits", None, artist, number);
            }
        }
        settle_local_albums(&mut library);

        let metallica = album_of(&library, "/music/m1.mp3");
        let pac = album_of(&library, "/music/p1.mp3");
        assert_ne!(metallica.id, pac.id);
        assert_eq!(album_of(&library, "/music/m2.mp3").id, metallica.id);
        assert_eq!(
            (metallica.artist.as_str(), pac.artist.as_str()),
            ("Metallica", "2Pac")
        );
        assert_eq!(
            metallica.cover_path, None,
            "a split album finds its own cover"
        );
    }

    #[test]
    fn an_untagged_compilation_stays_one_album_credited_to_various_artists() {
        let mut library = Library::default();
        scanned(&mut library, "/music/hits/1.mp3", "Hits", None, "Ana", 1);
        scanned(&mut library, "/music/hits/2.mp3", "Hits", None, "Bo", 2);
        settle_local_albums(&mut library);

        let first = album_of(&library, "/music/hits/1.mp3");
        assert_eq!(album_of(&library, "/music/hits/2.mp3").id, first.id);
        assert_eq!(first.artist, crate::VARIOUS_ARTISTS);
    }

    #[test]
    fn untagged_files_join_the_tagged_release_beside_them() {
        let mut library = Library::default();
        scanned(
            &mut library,
            "/music/load/1.flac",
            "Load",
            Some("Metallica"),
            "Metallica",
            1,
        );
        scanned(
            &mut library,
            "/music/load/2.flac",
            "Load",
            None,
            "Metallica",
            2,
        );
        settle_local_albums(&mut library);

        assert_eq!(
            album_of(&library, "/music/load/2.flac").id,
            album_of(&library, "/music/load/1.flac").id
        );
    }

    #[test]
    fn settling_twice_changes_nothing() {
        let mut library = Library::default();
        scanned(&mut library, "/music/a1.mp3", "Greatest Hits", None, "A", 1);
        scanned(&mut library, "/music/b1.mp3", "Greatest Hits", None, "B", 1);
        settle_local_albums(&mut library);
        let once: Vec<String> = library.tracks.iter().map(|t| t.album_id.clone()).collect();
        settle_local_albums(&mut library);
        let twice: Vec<String> = library.tracks.iter().map(|t| t.album_id.clone()).collect();
        assert_eq!(once, twice);
    }

    #[test]
    fn an_artist_keyed_legacy_album_carries_its_automatic_cover() {
        let cover = PathBuf::from("/covers/alb2_6_yaşar_divane.jpg");
        let mut legacy_album = album("alb2_6_yaşar_divane");
        legacy_album.title = "Divane".to_string();
        legacy_album.artist = "Yaşar".to_string();
        legacy_album.cover_path = Some(cover.clone());
        let mut scanned_album = album("alb3_6_yaşar_divane");
        scanned_album.title = "Divane".to_string();
        scanned_album.artist = "Yaşar".to_string();
        let mut library = Library {
            albums: vec![legacy_album],
            ..Default::default()
        };

        merge_scanned_tracks(
            &mut library,
            vec![ScannedTrack {
                track: track("/music/divane.flac", "alb3_6_yaşar_divane", "Divane"),
                album: scanned_album,
            }],
        );

        assert_eq!(library.albums[1].cover_path, Some(cover));
        assert!(!library.albums[1].manual_cover);
    }

    /// A mono 16-bit PCM WAV with no tags.
    fn silent_wav() -> Vec<u8> {
        let samples = [0u8; 1600];
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8_000u32.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(&samples);
        wav
    }

    #[tokio::test]
    async fn a_rescan_retitles_an_untagged_album_stored_as_unknown_album() {
        let root = std::env::temp_dir().join(format!("kopuz-retitle-{}", std::process::id()));
        let folder = root.join("Untagged");
        std::fs::create_dir_all(&folder).expect("folder");
        let path = folder.join("01.wav");
        std::fs::write(&path, silent_wav()).expect("wav");
        let id = make_album_id("", None, &path);
        let mut stored = track(path.to_str().expect("utf-8 path"), &id, "01");
        stored.album = "Unknown Album".to_string();
        let mut row = album(&id);
        row.title = "Unknown Album".to_string();
        let mut library = Library {
            tracks: vec![stored],
            albums: vec![row],
            ..Default::default()
        };

        let scan = scan_directory(
            root.clone(),
            root.join("covers"),
            &mut library,
            Arc::new(|_| {}),
        )
        .await;
        let _ = std::fs::remove_dir_all(&root);
        scan.expect("scan");

        assert_eq!(library.tracks.len(), 1);
        assert_eq!(library.tracks[0].album, "Untagged");
        assert_eq!(library.tracks[0].album_id, id);
        assert_eq!(
            album_of(&library, path.to_str().expect("utf-8 path")).title,
            "Untagged"
        );
    }

    #[test]
    fn merge_does_not_carry_ambiguous_legacy_automatic_cover() {
        let mut legacy_album = album("alb_divane");
        legacy_album.title = "Divane".to_string();
        legacy_album.artist = "Yaşar".to_string();
        legacy_album.cover_path = Some(PathBuf::from("/covers/wrong-artist.jpg"));
        let mut scanned_album = album("alb3_6_yaşar_divane");
        scanned_album.title = "Divane".to_string();
        scanned_album.artist = "Yaşar".to_string();
        let mut library = Library {
            albums: vec![legacy_album],
            ..Default::default()
        };

        merge_scanned_tracks(
            &mut library,
            vec![ScannedTrack {
                track: track("/music/divane.flac", "alb3_6_yaşar_divane", "Divane"),
                album: scanned_album,
            }],
        );

        assert_eq!(library.albums[1].cover_path, None);
        assert!(!library.albums[1].manual_cover);
    }
}
