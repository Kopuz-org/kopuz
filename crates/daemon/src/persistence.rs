//! Queue persistence shared by headless sessions and the frontend API. Both
//! paths write the same snapshot row and use the same progress rounding.

use async_trait::async_trait;
use reader::Track;

#[async_trait]
pub trait QueueStore: Send + Sync {
    async fn load(&self) -> Option<db::QueueSnapshot>;
    async fn save(&self, snapshot: db::QueueSnapshot);
}

pub struct DbQueueStore {
    db: db::Db,
}

impl DbQueueStore {
    pub fn new(db: db::Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl QueueStore for DbQueueStore {
    async fn load(&self) -> Option<db::QueueSnapshot> {
        match self.db.load_queue().await {
            Ok(snapshot) => {
                let fallback = snapshot.clone();
                tokio::task::spawn_blocking(move || sanitize_queue_snapshot(snapshot))
                    .await
                    .unwrap_or(Some(fallback))
            }
            Err(error) => {
                tracing::warn!(%error, "queue snapshot load failed");
                None
            }
        }
    }

    async fn save(&self, snapshot: db::QueueSnapshot) {
        if let Err(error) = self.db.save_queue(&snapshot).await {
            tracing::warn!(%error, "queue snapshot save failed");
        }
    }
}

fn is_restorable(track: &Track) -> bool {
    track.id.service().is_some() || track.id.local_path().is_some_and(|path| path.exists())
}

fn restorable_flags(queue: &[Track]) -> Vec<bool> {
    const THREADS: usize = 32;
    let chunk_size = queue.len().div_ceil(THREADS).max(1);
    let chunks: Vec<&[Track]> = queue.chunks(chunk_size).collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|tracks| scope.spawn(move || tracks.iter().map(is_restorable).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .zip(&chunks)
            .flat_map(|(handle, tracks)| handle.join().unwrap_or_else(|_| vec![true; tracks.len()]))
            .collect()
    })
}

pub fn sanitize_queue_snapshot(snapshot: db::QueueSnapshot) -> Option<db::QueueSnapshot> {
    if snapshot.queue.is_empty() {
        return None;
    }

    let original_index = snapshot
        .current_queue_index
        .min(snapshot.queue.len().saturating_sub(1));
    let flags = restorable_flags(&snapshot.queue);
    let selected_survived = flags.get(original_index).copied().unwrap_or(false);
    let survivors: Vec<(usize, Track)> = snapshot
        .queue
        .into_iter()
        .enumerate()
        .filter(|(index, _)| flags[*index])
        .collect();
    if survivors.is_empty() {
        return None;
    }

    let restored_index = if selected_survived {
        survivors
            .iter()
            .position(|(index, _)| *index == original_index)
            .unwrap_or_default()
    } else {
        survivors
            .iter()
            .enumerate()
            .min_by_key(|(_, (index, _))| (index.abs_diff(original_index), *index > original_index))
            .map(|(index, _)| index)
            .unwrap_or_default()
    };

    let mut physical_remap = vec![None; flags.len()];
    for (new_index, (old_index, _)) in survivors.iter().enumerate() {
        physical_remap[*old_index] = Some(new_index);
    }
    let shuffle_order = snapshot
        .shuffle_order
        .into_iter()
        .filter_map(|index| physical_remap.get(index).copied().flatten())
        .collect();
    let queue: Vec<Track> = survivors.into_iter().map(|(_, track)| track).collect();
    let progress_secs = if selected_survived {
        queue
            .get(restored_index)
            .map(|track| snapshot.progress_secs.min(track.duration))
            .unwrap_or_default()
    } else {
        0
    };

    Some(db::QueueSnapshot {
        version: snapshot.version,
        queue,
        current_queue_index: restored_index,
        progress_secs,
        shuffle_order,
        shuffle_enabled: snapshot.shuffle_enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: reader::TrackId) -> Track {
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
    fn every_server_source_is_restorable_without_a_local_file() {
        for service in [
            config::MusicService::Jellyfin,
            config::MusicService::Subsonic,
            config::MusicService::Custom,
            config::MusicService::YtMusic,
            config::MusicService::SoundCloud,
        ] {
            let track = track(reader::TrackId::Server {
                service,
                item_id: "item".to_string(),
            });
            assert!(is_restorable(&track));
        }
    }
}
