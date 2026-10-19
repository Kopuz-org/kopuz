//! The station record each radio row of the queue carries, so a saved queue plays without the registry that listed it.

use std::collections::{BTreeMap, HashMap};

use radio::manifest::StationManifest;
use reader::Track;
use server::playback_ref::PlaybackItemRef;

use super::Session;

/// The station a radio row plays; `None` for every other row.
pub(crate) fn queued_station(track: &Track) -> Option<String> {
    let key = track.id.uid();
    let item = PlaybackItemRef::parse(&key);
    item.is_radio()
        .then(|| item.primary_id().map(str::to_string))
        .flatten()
}

/// Whether a saved queue holds a radio row stored before rows carried their station.
pub(crate) fn has_unrecorded_station(snapshot: &db::QueueSnapshot) -> bool {
    snapshot.queue.iter().any(|track| {
        queued_station(track).is_some() && !snapshot.stations.contains_key(&*track.id.key())
    })
}

/// The snapshot's station records that parse, by station id.
fn records(snapshot: &db::QueueSnapshot) -> HashMap<String, StationManifest> {
    snapshot
        .stations
        .values()
        .filter_map(|json| match serde_json::from_str::<StationManifest>(json) {
            Ok(manifest) => Some((manifest.id.clone(), manifest)),
            Err(error) => {
                tracing::debug!(%error, "a queued station record does not parse");
                None
            }
        })
        .collect()
}

/// Drop each radio row whose station `known` cannot name, keeping the playhead on the row that was current or the one after it.
fn drop_unknown_stations(snapshot: &mut db::QueueSnapshot, known: impl Fn(&str) -> bool) {
    let keep: Vec<bool> = snapshot
        .queue
        .iter()
        .map(|track| match queued_station(track) {
            Some(station) if !known(&station) => {
                tracing::debug!(%station, title = %track.title, "dropping a queued station nothing can resolve");
                false
            }
            _ => true,
        })
        .collect();
    if keep.iter().all(|kept| *kept) {
        return;
    }
    let mut renumbered = Vec::with_capacity(keep.len());
    let mut next = 0;
    for kept in &keep {
        renumbered.push(next);
        next += usize::from(*kept);
    }
    let shuffled = snapshot.shuffle_enabled && !snapshot.shuffle_order.is_empty();
    let current = snapshot.current_queue_index;
    let current_dropped = match shuffled {
        true => snapshot
            .shuffle_order
            .get(current)
            .is_some_and(|at| !keep.get(*at).copied().unwrap_or(true)),
        false => !keep.get(current).copied().unwrap_or(true),
    };
    snapshot.current_queue_index = match shuffled {
        true => snapshot
            .shuffle_order
            .iter()
            .take(current)
            .filter(|at| keep.get(**at).copied().unwrap_or(false))
            .count(),
        false => keep.iter().take(current).filter(|kept| **kept).count(),
    };
    if current_dropped {
        snapshot.progress_secs = 0;
    }
    snapshot.shuffle_order = snapshot
        .shuffle_order
        .iter()
        .filter(|at| keep.get(**at).copied().unwrap_or(false))
        .map(|at| renumbered[*at])
        .collect();
    let mut kept = keep.iter();
    snapshot
        .queue
        .retain(|_| kept.next().copied().unwrap_or(true));
}

impl Session {
    /// A station by id: the registry's when it lists one, else the record its queue row was saved with.
    pub(super) fn station(&self, id: &str) -> Option<&StationManifest> {
        self.station_registry
            .get(id)
            .or_else(|| self.queue_stations.get(id))
    }

    /// Before the registry is replaced, keep the record of every station the queue still plays.
    pub(super) fn keep_queued_stations(&mut self) {
        let queued: Vec<String> = self
            .model
            .items()
            .iter()
            .filter_map(queued_station)
            .collect();
        let mut kept = HashMap::with_capacity(queued.len());
        for id in queued {
            if let Some(station) = self.station(&id) {
                kept.insert(id, station.clone());
            }
        }
        self.queue_stations = kept;
    }

    /// The record of each radio row, by track key, for the saved queue.
    pub(super) fn station_records(&self) -> BTreeMap<String, String> {
        let mut records = BTreeMap::new();
        for track in self.model.items() {
            let Some(id) = queued_station(track) else {
                continue;
            };
            let Some(station) = self.station(&id) else {
                continue;
            };
            match serde_json::to_string(station) {
                Ok(json) => {
                    records.insert(track.id.key().into_owned(), json);
                }
                Err(error) => tracing::warn!(%error, %id, "a queued station could not be saved"),
            }
        }
        records
    }

    /// Take a saved queue's station records, and drop the radio rows neither they nor the registry can resolve.
    pub(super) fn adopt_station_records(&mut self, snapshot: &mut db::QueueSnapshot) {
        self.queue_stations = records(snapshot);
        let registry = &self.station_registry;
        let saved = &self.queue_stations;
        drop_unknown_stations(snapshot, |id| {
            registry.get(id).is_some() || saved.contains_key(id)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str) -> Track {
        Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(key)),
            cover: None,
            album_id: String::new(),
            title: key.into(),
            artist: String::new(),
            album: String::new(),
            duration: 60,
            khz: 44,
            bitrate: 320,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: Vec::new(),
            replay_gain: config::ReplayGainInfo::default(),
            credits: Vec::new(),
            counterpart: None,
            explicit: false,
            plays: None,
        }
    }

    fn titles(snapshot: &db::QueueSnapshot) -> Vec<&str> {
        snapshot.queue.iter().map(|t| t.title.as_str()).collect()
    }

    #[test]
    fn an_unknown_station_row_is_dropped_and_the_playhead_moves_on() {
        let mut snapshot = db::QueueSnapshot {
            queue: ["/a", "radio:gone:main", "/b", "radio:kept:main"]
                .into_iter()
                .map(row)
                .collect(),
            current_queue_index: 1,
            progress_secs: 30,
            ..Default::default()
        };
        drop_unknown_stations(&mut snapshot, |id| id == "kept");
        assert_eq!(titles(&snapshot), ["/a", "/b", "radio:kept:main"]);
        assert_eq!(
            (snapshot.current_queue_index, snapshot.progress_secs),
            (1, 0)
        );
    }

    #[test]
    fn a_shuffled_queue_keeps_its_order_without_the_dropped_row() {
        let mut snapshot = db::QueueSnapshot {
            queue: ["/a", "radio:gone:main", "/b"]
                .into_iter()
                .map(row)
                .collect(),
            shuffle_enabled: true,
            shuffle_order: vec![2, 1, 0],
            current_queue_index: 2,
            progress_secs: 30,
            ..Default::default()
        };
        drop_unknown_stations(&mut snapshot, |_| false);
        assert_eq!(titles(&snapshot), ["/a", "/b"]);
        assert_eq!(snapshot.shuffle_order, [1, 0]);
        assert_eq!(
            (snapshot.current_queue_index, snapshot.progress_secs),
            (1, 30)
        );
    }
}
