//! Discord Rich Presence integration for Kopuz: publishes now-playing state
//! (track, artist, album art) to the Discord client via RPC.

pub mod cover_art;

#[cfg(not(target_os = "android"))]
use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{self, Assets, Timestamps},
};
#[cfg(not(target_os = "android"))]
use std::sync::{Mutex, MutexGuard};
#[cfg(not(target_os = "android"))]
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(target_os = "android"))]
const RECONNECT_INTERVAL: Duration = Duration::from_secs(10);

#[cfg(not(target_os = "android"))]
#[derive(Debug, Clone)]
struct StoredActivity {
    details: String,
    state: String,
    name: Option<String>,
    timestamps: Option<(i64, Option<i64>)>,
    assets: Option<(String, String)>,
}

#[cfg(not(target_os = "android"))]
impl StoredActivity {
    fn as_activity(&self) -> activity::Activity<'_> {
        let mut act = activity::Activity::new()
            .details(&self.details)
            .state(&self.state)
            .status_display_type(activity::StatusDisplayType::State)
            .activity_type(activity::ActivityType::Listening);

        if let Some((start, end)) = self.timestamps {
            let mut timestamps = Timestamps::new().start(start);
            if let Some(end) = end {
                timestamps = timestamps.end(end);
            }
            act = act.timestamps(timestamps);
        }

        if let Some(ref name) = self.name {
            act = act.name(name);
        }

        if let Some((ref image, ref text)) = self.assets {
            act = act.assets(Assets::new().large_image(image).large_text(text));
        }

        act
    }
}

#[cfg(not(target_os = "android"))]
#[derive(Debug)]
struct Inner {
    client: Option<DiscordIpcClient>,
    last_activity: Option<StoredActivity>,
    next_retry: Option<Instant>,
}

#[cfg(not(target_os = "android"))]
#[derive(Debug)]
pub struct Presence {
    client_id: String,
    inner: Mutex<Inner>,
}

#[cfg(not(target_os = "android"))]
impl Presence {
    pub fn new(client_id: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let presence = Self {
            client_id: client_id.to_owned(),
            inner: Mutex::new(Inner {
                client: None,
                last_activity: None,
                next_retry: None,
            }),
        };

        {
            let mut inner = presence.lock();
            if presence.try_connect(&mut inner) {
                tracing::info!("Discord presence connected");
            } else {
                tracing::info!("Discord IPC unavailable at startup; will retry in the background");
            }
        }

        Ok(presence)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn try_connect(&self, inner: &mut Inner) -> bool {
        let mut client = DiscordIpcClient::new(&self.client_id);
        match client.connect() {
            Ok(()) => {
                inner.client = Some(client);
                inner.next_retry = None;
                true
            }
            Err(e) => {
                tracing::debug!("Discord IPC connect failed: {e}");
                inner.client = None;
                inner.next_retry = Some(Instant::now() + RECONNECT_INTERVAL);
                false
            }
        }
    }

    fn send_current(&self, inner: &mut Inner) -> Result<(), Box<dyn std::error::Error>> {
        let Some(client) = inner.client.as_mut() else {
            return Err("Discord IPC is not connected".into());
        };
        let result = match inner.last_activity {
            Some(ref act) => client.set_activity(act.as_activity()),
            None => client.clear_activity(),
        };
        if result.is_err() {
            inner.client = None;
            inner.next_retry = Some(Instant::now() + RECONNECT_INTERVAL);
        }
        result.map_err(Into::into)
    }

    fn set_and_deliver(&self, stored: StoredActivity) -> Result<(), Box<dyn std::error::Error>> {
        let mut inner = self.lock();
        inner.last_activity = Some(stored);

        if inner.client.is_none() {
            if !self.try_connect(&mut inner) {
                return Err("Discord IPC unavailable".into());
            }
            tracing::info!("Discord presence connected");
            return self.send_current(&mut inner);
        }

        match self.send_current(&mut inner) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::debug!("Discord activity update failed ({e}); reconnecting");
                if self.try_connect(&mut inner) {
                    tracing::info!("Discord presence reconnected");
                    self.send_current(&mut inner)
                } else {
                    Err(e)
                }
            }
        }
    }

    pub fn tick(&self) {
        let mut inner = self.lock();
        if inner.client.is_some() || inner.last_activity.is_none() {
            return;
        }
        if let Some(at) = inner.next_retry
            && Instant::now() < at
        {
            return;
        }
        if self.try_connect(&mut inner) {
            tracing::info!("Discord presence connected; restoring activity");
            if let Err(e) = self.send_current(&mut inner) {
                tracing::debug!("Failed to restore Discord activity: {e}");
            }
        }
    }

    pub fn disconnect(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut inner = self.lock();
        inner.last_activity = None;
        inner.next_retry = None;
        if let Some(mut client) = inner.client.take() {
            client.close()?;
        }
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
            (start_time, None)
        } else {
            (start_time, Some(end_time))
        };

        self.set_and_deliver(StoredActivity {
            details: title.to_owned(),
            state: artist.to_owned(),
            name: source.map(|s| format!("Kopuz - on {s}")),
            timestamps: Some(timestamps),
            assets: cover_url.map(|url| (url.to_owned(), album.to_owned())),
        })
    }

    pub fn set_paused(
        &self,
        title: &str,
        artist: &str,
        album: &str,
        cover_url: Option<&str>,
        source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.set_and_deliver(StoredActivity {
            details: title.to_owned(),
            state: format!("{artist} • Paused"),
            name: source.map(|s| format!("Kopuz - on {s}")),
            timestamps: None,
            assets: cover_url.map(|url| (url.to_owned(), album.to_owned())),
        })
    }

    pub fn clear_activity(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut inner = self.lock();
        inner.last_activity = None;
        if inner.client.is_some() {
            self.send_current(&mut inner)?;
        }
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for Presence {
    fn drop(&mut self) {
        let mut inner = self.lock();
        if let Some(mut client) = inner.client.take() {
            let _ = client.close();
        }
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

    pub fn tick(&self) {}

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

#[cfg(all(test, not(target_os = "android"), target_family = "unix"))]
mod reconnect_tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::mpsc;

    fn read_frame(sock: &mut UnixStream) -> std::io::Result<String> {
        let mut header = [0u8; 8];
        sock.read_exact(&mut header)?;
        let len = u32::from_le_bytes(header[4..].try_into().expect("4-byte slice"));
        let mut payload = vec![0u8; len as usize];
        sock.read_exact(&mut payload)?;
        Ok(String::from_utf8(payload).expect("IPC payload is JSON"))
    }

    fn write_frame(sock: &mut UnixStream, opcode: u32, payload: &str) -> std::io::Result<()> {
        sock.write_all(&opcode.to_le_bytes())?;
        sock.write_all(&(payload.len() as u32).to_le_bytes())?;
        sock.write_all(payload.as_bytes())
    }

    fn serve_one(listener: UnixListener, tx: mpsc::Sender<String>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept IPC client");
            read_frame(&mut sock).expect("read handshake");
            write_frame(&mut sock, 1, r#"{"cmd":"DISPATCH","evt":"READY"}"#)
                .expect("ack handshake");
            let payload = read_frame(&mut sock).expect("read activity frame");
            tx.send(payload).expect("forward payload");
            let _ = sock.shutdown(std::net::Shutdown::Both);
        })
    }

    #[test]
    fn reconnects_and_restores_activity() {
        let dir = std::env::temp_dir().join(format!("kopuz-drpc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create socket dir");
        unsafe {
            std::env::remove_var("SNAP");
            for key in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
                std::env::set_var(key, &dir);
            }
        }

        let presence = Presence::new("test-client-id").expect("offline construction succeeds");
        assert!(presence.lock().client.is_none());
        assert!(
            presence
                .set_now_playing("First Song", "Artist", "Album", 30, 180, None, None)
                .is_err()
        );
        assert!(presence.lock().last_activity.is_some());

        let listener = UnixListener::bind(dir.join("discord-ipc-0")).expect("bind fake IPC");
        let (tx, rx) = mpsc::channel();
        let server = serve_one(listener.try_clone().expect("clone listener"), tx.clone());
        presence.lock().next_retry = None;
        presence.tick();
        let payload = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("activity restored after tick");
        assert!(payload.contains("SET_ACTIVITY"));
        assert!(payload.contains("First Song"));
        server.join().expect("server thread");
        assert!(presence.lock().client.is_some());

        let server = serve_one(listener, tx);
        presence
            .set_now_playing("Second Song", "Artist", "Album", 0, 200, None, None)
            .expect("update resent over new connection");
        let payload = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("activity resent after reconnect");
        assert!(payload.contains("Second Song"));
        server.join().expect("server thread");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
