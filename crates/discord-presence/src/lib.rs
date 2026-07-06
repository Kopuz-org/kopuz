//! Discord Rich Presence integration for Kopuz: publishes now-playing state
//! (track, artist, album art) to the Discord client via RPC.

pub mod cover_art;

use std::sync::{Arc, OnceLock};

/// A cheaply-cloneable, thread-safe handle to a `Presence` that may not be
/// connected yet.
///
/// The IPC handshake blocks (up to tens of seconds when Discord is slow to ACK
/// a rapid reconnect), so the connection is made off the startup path on a
/// background thread and published here once ready. Consumers hold the handle
/// and poll [`get`](PresenceHandle::get) each tick, picking up the connection
/// whenever it lands (or never, if Discord isn't running).
#[derive(Clone, Default)]
pub struct PresenceHandle(Arc<OnceLock<Arc<Presence>>>);

impl PresenceHandle {
    /// The connected presence, or `None` until the background connect succeeds.
    pub fn get(&self) -> Option<Arc<Presence>> {
        self.0.get().cloned()
    }

    /// Publish the connected presence. Later calls are ignored (connect once).
    pub fn set(&self, presence: Presence) {
        let _ = self.0.set(Arc::new(presence));
    }
}

#[cfg(not(target_os = "android"))]
use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{self, Assets, Timestamps},
};
#[cfg(not(target_os = "android"))]
use std::sync::Mutex;
#[cfg(not(target_os = "android"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_os = "android"))]
#[derive(Debug)]
pub struct Presence {
    client: Mutex<DiscordIpcClient>,
}

#[cfg(not(target_os = "android"))]
impl Presence {
    pub fn new(client_id: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut client = DiscordIpcClient::new(client_id);
        client.connect()?;
        Ok(Self {
            client: Mutex::new(client),
        })
    }

    pub fn disconnect(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .close()?;
        Ok(())
    }

    pub fn set_now_playing(
        &self,
        title: &str,
        artist: &str,
        album: &str,
        elapsed_secs: u64,
        duration_secs: u64,
        cover_url: Option<&str>,
        source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        let start_time = now - elapsed_secs as i64;
        let end_time = start_time + duration_secs as i64;

        let timestamps = if duration_secs == u64::MAX {
            Timestamps::new().start(start_time)
        } else {
            Timestamps::new().start(start_time).end(end_time)
        };

        let state = artist.to_string();
        let name = source.map(|s| format!("Kopuz - on {s}"));

        let mut activity = activity::Activity::new()
            .details(title)
            .state(&state)
            .status_display_type(activity::StatusDisplayType::State)
            .timestamps(timestamps)
            .activity_type(activity::ActivityType::Listening);

        if let Some(ref name) = name {
            activity = activity.name(name);
        }

        if let Some(url) = cover_url {
            let assets = Assets::new().large_image(url).large_text(album);
            activity = activity.assets(assets);
        }

        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_activity(activity)?;
        Ok(())
    }

    pub fn set_paused(
        &self,
        title: &str,
        artist: &str,
        album: &str,
        cover_url: Option<&str>,
        source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let state = format!("{artist} • Paused");
        let name = source.map(|s| format!("Kopuz - on {s}"));
        let mut activity = activity::Activity::new()
            .details(title)
            .state(&state)
            .status_display_type(activity::StatusDisplayType::State)
            .activity_type(activity::ActivityType::Listening);

        if let Some(ref name) = name {
            activity = activity.name(name);
        }

        if let Some(url) = cover_url {
            let assets = Assets::new().large_image(url).large_text(album);
            activity = activity.assets(assets);
        }

        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_activity(activity)?;
        Ok(())
    }

    pub fn clear_activity(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear_activity()?;
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for Presence {
    fn drop(&mut self) {
        let mut client = match self.client.lock() {
            Ok(c) => c,
            Err(poisoned) => poisoned.into_inner(),
        };
        let _ = client.close();
    }
}

// Android has no Discord IPC; this no-op stub keeps the `Presence` API surface so the
// shared player-task code compiles unchanged. The app never constructs it on Android
// (`Presence::new` errors), so the context stays `None` and every call site is skipped.
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct Presence;

#[cfg(target_os = "android")]
impl Presence {
    pub fn new(_client_id: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Err("Discord presence is not available on Android".into())
    }

    pub fn disconnect(&self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    pub fn set_now_playing(
        &self,
        _title: &str,
        _artist: &str,
        _album: &str,
        _elapsed_secs: u64,
        _duration_secs: u64,
        _cover_url: Option<&str>,
        _source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    pub fn set_paused(
        &self,
        _title: &str,
        _artist: &str,
        _album: &str,
        _cover_url: Option<&str>,
        _source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    pub fn clear_activity(&self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}
