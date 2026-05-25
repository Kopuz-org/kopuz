use reader::Track;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

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

impl PersistedQueueState {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let state = serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(path, data)
    }
}

#[cfg(test)]
mod tests {
    use super::PersistedQueueState;
    use reader::Track;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("kopuz_{name}_{unique}.json"))
    }

    fn sample_track(path: &str, album_id: &str, title: &str) -> Track {
        Track {
            path: PathBuf::from(path),
            album_id: album_id.to_string(),
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration: 180,
            khz: 44_100,
            bitrate: 320,
            track_number: Some(1),
            disc_number: Some(1),
            musicbrainz_release_id: None,
            playlist_item_id: None,
            artists: vec!["Artist".to_string()],
        }
    }

    #[test]
    fn load_missing_file_returns_default_state() {
        let path = temp_file("queue_state_missing");

        let state = PersistedQueueState::load(&path).unwrap();

        assert_eq!(state, PersistedQueueState::default());
    }

    #[test]
    fn deserialize_missing_version_uses_default_version() {
        let state: PersistedQueueState = serde_json::from_str(
            r#"{
                "queue": [],
                "current_queue_index": 2,
                "progress_secs": 42,
                "shuffle_order": [1, 0],
                "shuffle_enabled": true
            }"#,
        )
        .unwrap();

        assert_eq!(state.version, 1);
        assert_eq!(state.current_queue_index, 2);
        assert_eq!(state.progress_secs, 42);
        assert_eq!(state.shuffle_order, vec![1, 0]);
        assert!(state.shuffle_enabled);
    }

    #[test]
    fn deserialize_empty_uses_defaults() {
        let state: PersistedQueueState = serde_json::from_str(r#"{}"#).unwrap();

        assert_eq!(state.version, 1);
        assert_eq!(state.current_queue_index, 0);
        assert_eq!(state.progress_secs, 0);
        assert!(state.shuffle_order.is_empty());
        assert!(!state.shuffle_enabled);
    }

    #[test]
    fn deserialize_with_missing_track_defaults_preserves_queue() {
        let state: PersistedQueueState = serde_json::from_str(
            r#"{
                "version": 2,
                "queue": [{
                    "path": "/music/track.flac",
                    "album_id": "alb",
                    "title": "Track",
                    "artist": "Artist",
                    "album": "Album",
                    "duration": 180,
                    "khz": 44100,
                    "track_number": 1,
                    "disc_number": 1
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(state.version, 2);
        assert_eq!(state.queue.len(), 1);
        assert_eq!(state.queue[0].bitrate, 0);
        assert!(state.queue[0].musicbrainz_release_id.is_none());
        assert!(state.queue[0].playlist_item_id.is_none());
        assert!(state.queue[0].artists.is_empty());
    }

    #[test]
    fn save_and_load_round_trip_state() {
        let path = temp_file("queue_state_round_trip");
        let state = PersistedQueueState {
            version: 1,
            current_queue_index: 3,
            progress_secs: 120,
            shuffle_order: vec![2, 0, 1],
            shuffle_enabled: true,
            ..PersistedQueueState::default()
        };

        state.save(&path).unwrap();
        let loaded = PersistedQueueState::load(&path).unwrap();

        assert_eq!(loaded, state);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_and_load_round_trip_with_tracks_preserves_queue_contents() {
        let path = temp_file("queue_state_tracks_round_trip");
        let state = PersistedQueueState {
            version: 2,
            queue: vec![
                sample_track("/music/one.flac", "alb-one", "One"),
                sample_track("/music/two.flac", "alb-two", "Two"),
            ],
            current_queue_index: 1,
            progress_secs: 87,
            shuffle_order: vec![1, 0],
            shuffle_enabled: true,
        };

        state.save(&path).unwrap();
        let loaded = PersistedQueueState::load(&path).unwrap();

        assert_eq!(loaded, state);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_invalid_json_returns_error() {
        let path = temp_file("queue_state_invalid");
        fs::write(&path, "invalid json").unwrap();

        let result = PersistedQueueState::load(&path);
        assert!(result.is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_creates_parent_directories() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kopuz_nested_{unique}"));
        let path = dir.join("some/nested/path/queue_state.json");

        let state = PersistedQueueState::default();
        state.save(&path).unwrap();

        assert!(path.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_writes_parent_directory_for_deep_nested_path() {
        let base = temp_file("queue_state_nested_parent");
        let path = base.join(Path::new("a/b/c/state.json"));

        PersistedQueueState::default().save(&path).unwrap();

        assert!(path.exists());
        let _ = fs::remove_dir_all(base);
    }
}
