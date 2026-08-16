use reader::Track;
use serde::{Deserialize, Serialize};

pub const SAVE_DEBOUNCE_MS: u64 = 1200;
pub const PROGRESS_STEP_SECS: u64 = 5;

fn default_queue_state_version() -> u8 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistedQueueState {
    #[serde(default = "default_queue_state_version")]
    pub version: u8,
    #[serde(default)]
    pub queue: Vec<Track>,
    #[serde(default)]
    pub current_queue_index: usize,
    #[serde(default)]
    pub progress_secs: u64,
    #[serde(default)]
    pub shuffle_order: Vec<usize>,
    #[serde(default)]
    pub shuffle_enabled: bool,
}

impl Default for PersistedQueueState {
    fn default() -> Self {
        Self {
            version: default_queue_state_version(),
            queue: Vec::new(),
            current_queue_index: 0,
            progress_secs: 0,
            shuffle_order: Vec::new(),
            shuffle_enabled: false,
        }
    }
}

pub async fn persist_snapshot(db: db::Db, queue_state: Option<PersistedQueueState>) {
    let snap = queue_state.map(snapshot).unwrap_or_default();
    if let Err(e) = db.save_queue(&snap).await {
        tracing::error!("Failed to save queue state: {}", e);
    }
}

pub fn snapshot(q: PersistedQueueState) -> db::QueueSnapshot {
    db::QueueSnapshot {
        version: q.version,
        queue: q.queue,
        current_queue_index: q.current_queue_index,
        progress_secs: q.progress_secs,
        shuffle_order: q.shuffle_order,
        shuffle_enabled: q.shuffle_enabled,
    }
}

fn is_streamable_queue_track(track: &Track) -> bool {
    // Defer to the same scheme parser playback uses, so the restore filter can't
    // drift from the list of server sources (this copy had already fallen behind
    // on ytmusic/soundcloud). Parsing is case-exact, matching playback: uid()
    // emits lowercase scheme prefixes, so a mis-cased id that would play back as
    // Local is correctly excluded here too.
    hooks::playback_ref::PlaybackItemRef::parse(&track.id.uid()).is_server()
}

fn is_restorable_queue_track(track: &Track) -> bool {
    is_streamable_queue_track(track) || track.id.local_path().is_some_and(|p| p.exists())
}

/// Per-track `exists()` stats dominate restore on slow/external volumes
/// (milliseconds each × thousands of tracks), so they run fanned out across
/// threads. A panicked chunk defaults to keeping its tracks — playback
/// already tolerates missing files at play time.
fn restorable_flags(queue: &[Track]) -> Vec<bool> {
    const THREADS: usize = 32;
    let chunk_size = queue.len().div_ceil(THREADS).max(1);
    let chunks: Vec<&[Track]> = queue.chunks(chunk_size).collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|tracks| {
                scope.spawn(move || {
                    tracks
                        .iter()
                        .map(is_restorable_queue_track)
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(&chunks)
            .flat_map(|(handle, tracks)| handle.join().unwrap_or_else(|_| vec![true; tracks.len()]))
            .collect()
    })
}

pub fn sanitize(state: PersistedQueueState) -> Option<PersistedQueueState> {
    if state.queue.is_empty() {
        return None;
    }

    let original_index = state
        .current_queue_index
        .min(state.queue.len().saturating_sub(1));
    let mut selected_track_survived = false;
    let flags = restorable_flags(&state.queue);
    let survivors: Vec<(usize, Track)> = state
        .queue
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| {
            let keep = flags[*idx];
            if keep && *idx == original_index {
                selected_track_survived = true;
            }
            keep
        })
        .collect();

    if survivors.is_empty() {
        return None;
    }

    let restored_index = if selected_track_survived {
        survivors
            .iter()
            .position(|(idx, _)| *idx == original_index)
            .unwrap_or(0)
    } else {
        survivors
            .iter()
            .enumerate()
            .min_by_key(|(_, (idx, _))| (idx.abs_diff(original_index), *idx > original_index))
            .map(|(restored_idx, _)| restored_idx)
            .unwrap_or(0)
    };

    let old_queue_len = survivors
        .iter()
        .map(|(old_idx, _)| *old_idx)
        .max()
        .map_or(0, |m| m + 1);

    let mut old_to_new_index: Vec<Option<usize>> = vec![None; old_queue_len];
    for (new_idx, (old_idx, _)) in survivors.iter().enumerate() {
        old_to_new_index[*old_idx] = Some(new_idx);
    }

    let shuffle_order: Vec<usize> = state
        .shuffle_order
        .into_iter()
        .filter_map(|old_idx| old_to_new_index.get(old_idx).and_then(|&new_idx| new_idx))
        .collect();

    let queue: Vec<_> = survivors.into_iter().map(|(_, track)| track).collect();
    let progress_secs = if selected_track_survived {
        queue
            .get(restored_index)
            .map(|track| state.progress_secs.min(track.duration))
            .unwrap_or(0)
    } else {
        0
    };

    Some(PersistedQueueState {
        version: state.version,
        queue,
        current_queue_index: restored_index,
        progress_secs,
        shuffle_order,
        shuffle_enabled: state.shuffle_enabled,
    })
}

pub fn build_snapshot(
    queue: &[Track],
    current_queue_index: usize,
    current_song_progress: u64,
    is_playing: bool,
    shuffle_order: &[usize],
    shuffle_enabled: bool,
) -> Option<PersistedQueueState> {
    if queue.is_empty() {
        return None;
    }

    let current_idx = current_queue_index.min(queue.len() - 1);
    let progress_secs = queue
        .get(current_idx)
        .map(|track| current_song_progress.min(track.duration))
        .unwrap_or(0);
    let progress_secs = if is_playing {
        progress_secs - (progress_secs % PROGRESS_STEP_SECS)
    } else {
        progress_secs
    };

    Some(PersistedQueueState {
        version: 1,
        queue: queue.to_vec(),
        current_queue_index: current_idx,
        progress_secs,
        shuffle_order: shuffle_order.to_vec(),
        shuffle_enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reader::TrackId;

    fn track(id: TrackId) -> Track {
        Track {
            id,
            cover: None,
            album_id: String::new(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: Vec::new(),
        }
    }

    #[test]
    fn streamable_covers_every_server_source_and_excludes_local() {
        for service in [
            config::MusicService::Jellyfin,
            config::MusicService::Subsonic,
            config::MusicService::Custom,
            config::MusicService::YtMusic,
            config::MusicService::SoundCloud,
        ] {
            let t = track(TrackId::Server {
                service,
                item_id: "x".into(),
            });
            assert!(
                is_streamable_queue_track(&t),
                "server source {service:?} must be streamable"
            );
        }

        let local = track(TrackId::Local("/music/a.flac".into()));
        assert!(
            !is_streamable_queue_track(&local),
            "local is not streamable"
        );
    }
}
