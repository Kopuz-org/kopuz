//! What an integration plugin may do to kopuz, carried into the daemon's own
//! API, the one the apps use: the plugin host checks each call against the
//! plugin's manifest first.

use std::sync::{Arc, Weak};

use api::{
    LibraryApi, Page, PlayerApi, PlayerCommand, PlaylistApi, QueueContext, QueueMode,
    SetQueueRequest, TrackFilter, TrackInfo,
};
use plugin_host::wit::{library, playback};
use plugin_host::{Answer, Bridge};

/// Weak, since the API holds the integrations that hold this.
pub struct ApiBridge<A> {
    api: Weak<A>,
}

impl<A> ApiBridge<A> {
    pub fn new(api: Weak<A>) -> Self {
        Self { api }
    }
}

impl<A: Send + Sync + 'static> ApiBridge<A> {
    fn api(&self) -> Result<Arc<A>, String> {
        self.api
            .upgrade()
            .ok_or_else(|| "kopuz is shutting down".to_string())
    }
}

fn track(info: TrackInfo) -> library::Track {
    library::Track {
        key: info.key,
        title: info.title,
        artist: info.artist,
        album: info.album,
        album_id: info.album_id,
        duration_ms: info.duration_ms,
        track_number: info.track_number,
        disc_number: info.disc_number,
        musicbrainz_recording_id: info.musicbrainz_recording_id,
        musicbrainz_release_id: info.musicbrainz_release_id,
    }
}

fn page(offset: u32, limit: u32) -> Page {
    Page {
        offset,
        limit: limit.min(plugin_host::MAX_PAGE),
    }
}

fn command(command: playback::Command) -> PlayerCommand {
    match command {
        playback::Command::Play => PlayerCommand::Play,
        playback::Command::Pause => PlayerCommand::Pause,
        playback::Command::Toggle => PlayerCommand::Toggle,
        playback::Command::Next => PlayerCommand::Next,
        playback::Command::Previous => PlayerCommand::Previous,
        playback::Command::Stop => PlayerCommand::Stop,
        playback::Command::Seek(position_ms) => PlayerCommand::Seek { position_ms },
        playback::Command::Volume(volume) => PlayerCommand::SetVolume {
            volume: volume.clamp(0.0, 1.0),
        },
    }
}

/// `keys` the library returned rows for, in the order they were asked, each once.
fn known_keys(keys: Vec<String>, known: &[api::TrackInfo]) -> Vec<String> {
    let held: std::collections::HashSet<&str> =
        known.iter().map(|track| track.key.as_str()).collect();
    let mut seen = std::collections::HashSet::new();
    keys.into_iter()
        .filter(|key| held.contains(key.as_str()) && seen.insert(key.clone()))
        .collect()
}

fn queue_request(keys: Vec<String>, mode: playback::QueueMode) -> SetQueueRequest {
    let (mode, start_index) = match mode {
        playback::QueueMode::Replace => (QueueMode::Replace, Some(0)),
        playback::QueueMode::PlayNext => (QueueMode::PlayNext, None),
        playback::QueueMode::Append => (QueueMode::Append, None),
    };
    SetQueueRequest {
        mode,
        context: QueueContext::Tracks { keys },
        start_index,
        shuffle: None,
    }
}

impl<A> Bridge for ApiBridge<A>
where
    A: PlayerApi + LibraryApi + PlaylistApi + Send + Sync + 'static,
{
    fn control(&self, order: playback::Command) -> Answer<()> {
        let api = self.api();
        Box::pin(async move {
            api?.player_command(command(order))
                .await
                .map(|_| ())
                .map_err(|error| error.message)
        })
    }

    fn queue(&self, keys: Vec<String>, mode: playback::QueueMode) -> Answer<()> {
        let api = self.api();
        Box::pin(async move {
            let api = api?;
            // Only what the library holds: a key it does not would be read as a path on
            // disk or looked up at the source, which is more than reading the library allows.
            let known = api
                .tracks_by_keys(keys.clone())
                .await
                .map_err(|error| error.message)?;
            let keys = known_keys(keys, &known);
            if keys.is_empty() {
                return Err("none of those tracks are in the library".into());
            }
            api.set_queue(queue_request(keys, mode))
                .await
                .map(|_| ())
                .map_err(|error| error.message)
        })
    }

    fn search(&self, query: String, limit: u32) -> Answer<Vec<library::Track>> {
        let api = self.api();
        Box::pin(async move {
            // An empty search filters nothing, which would hand back whatever comes first.
            if query.trim().is_empty() {
                return Ok(Vec::new());
            }
            let filter = TrackFilter {
                search: Some(query),
                ..TrackFilter::default()
            };
            let found = api?
                .tracks(filter, page(0, limit))
                .await
                .map_err(|error| error.message)?;
            Ok(found.items.into_iter().map(track).collect())
        })
    }

    fn tracks(&self, keys: Vec<String>) -> Answer<Vec<library::Track>> {
        let api = self.api();
        Box::pin(async move {
            let found = api?
                .tracks_by_keys(keys)
                .await
                .map_err(|error| error.message)?;
            Ok(found.into_iter().map(track).collect())
        })
    }

    fn album_tracks(&self, album_id: String) -> Answer<Vec<library::Track>> {
        let api = self.api();
        Box::pin(async move {
            let found = api?
                .album_tracks(album_id, page(0, plugin_host::MAX_PAGE))
                .await
                .map_err(|error| error.message)?;
            Ok(found.items.into_iter().map(track).collect())
        })
    }

    fn artist_tracks(&self, artist_key: String) -> Answer<Vec<library::Track>> {
        let api = self.api();
        Box::pin(async move {
            let found = api?
                .artist_tracks(artist_key, page(0, plugin_host::MAX_PAGE))
                .await
                .map_err(|error| error.message)?;
            Ok(found.items.into_iter().map(track).collect())
        })
    }

    fn tracks_page(&self, offset: u32, limit: u32) -> Answer<library::TrackPage> {
        let api = self.api();
        Box::pin(async move {
            let found = api?
                .tracks(TrackFilter::default(), page(offset, limit))
                .await
                .map_err(|error| error.message)?;
            Ok(library::TrackPage {
                total: found.total,
                tracks: found.items.into_iter().map(track).collect(),
            })
        })
    }

    fn playlists(&self) -> Answer<Vec<library::Playlist>> {
        let api = self.api();
        Box::pin(async move {
            let catalog = api?.playlists().await.map_err(|error| error.message)?;
            Ok(catalog
                .playlists
                .into_iter()
                .map(|playlist| library::Playlist {
                    id: playlist.id,
                    name: playlist.name,
                    track_keys: playlist.track_keys,
                })
                .collect())
        })
    }

    fn favorites(&self) -> Answer<Vec<String>> {
        let api = self.api();
        Box::pin(async move { Ok(api?.favorites().await.map_err(|error| error.message)?.refs) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugins_command_is_the_players() {
        assert_eq!(command(playback::Command::Next), PlayerCommand::Next);
        assert_eq!(
            command(playback::Command::Seek(30_000)),
            PlayerCommand::Seek {
                position_ms: 30_000
            }
        );
        assert_eq!(
            command(playback::Command::Volume(7.0)),
            PlayerCommand::SetVolume { volume: 1.0 },
            "a volume past full is full"
        );
    }

    #[test]
    fn only_keys_the_library_holds_are_queued() {
        let held = |key: &str| api::TrackInfo {
            key: key.into(),
            ..Default::default()
        };
        let asked = ["b", "/etc/passwd", "a", "b"].map(String::from).to_vec();
        assert_eq!(known_keys(asked, &[held("a"), held("b")]), ["b", "a"]);
    }

    #[test]
    fn replacing_the_queue_plays_its_first_track_and_adding_does_not() {
        let keys = vec!["a".to_string(), "b".to_string()];
        let replace = queue_request(keys.clone(), playback::QueueMode::Replace);
        assert_eq!(replace.mode, QueueMode::Replace);
        assert_eq!(replace.start_index, Some(0));
        assert_eq!(replace.context, QueueContext::Tracks { keys: keys.clone() });
        let next = queue_request(keys, playback::QueueMode::PlayNext);
        assert_eq!(next.mode, QueueMode::PlayNext);
        assert_eq!(next.start_index, None);
    }
}
