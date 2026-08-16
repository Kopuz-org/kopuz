use mpris_server::{
    LoopStatus, Metadata, PlaybackStatus, PlayerInterface, Property, RootInterface, Server, Time,
    zbus::fdo,
};
use std::sync::{
    Arc, Mutex, OnceLock,
    mpsc::{self, Receiver, Sender},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Off,
    Playlist,
    Track,
}

impl RepeatMode {
    fn from_mpris(status: LoopStatus) -> Self {
        match status {
            LoopStatus::None => Self::Off,
            LoopStatus::Track => Self::Track,
            LoopStatus::Playlist => Self::Playlist,
        }
    }

    fn to_mpris(self) -> LoopStatus {
        match self {
            Self::Off => LoopStatus::None,
            Self::Track => LoopStatus::Track,
            Self::Playlist => LoopStatus::Playlist,
        }
    }
}

#[derive(Debug)]
pub enum SystemEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    /// Absolute target position in seconds.
    Seek(f64),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
}

/// MPRIS SetPosition requires a `mpris:trackid`; we expose a constant one
/// (position always refers to the current track).
const TRACK_ID: &str = "/org/kopuz/track/current";

struct MprisState {
    metadata: Metadata,
    status: PlaybackStatus,
    position: Time,
    shuffle: bool,
    repeat: RepeatMode,
}

static TX: OnceLock<Sender<SystemEvent>> = OnceLock::new();
static RX: OnceLock<Mutex<Receiver<SystemEvent>>> = OnceLock::new();
static STATE: OnceLock<Arc<Mutex<MprisState>>> = OnceLock::new();
static NOTIFY: OnceLock<tokio::sync::mpsc::UnboundedSender<bool>> = OnceLock::new();

fn tx() -> Sender<SystemEvent> {
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        RX.set(Mutex::new(rx)).ok();
        tx
    })
    .clone()
}

fn state() -> Arc<Mutex<MprisState>> {
    STATE
        .get_or_init(|| {
            Arc::new(Mutex::new(MprisState {
                metadata: Metadata::new(),
                status: PlaybackStatus::Stopped,
                position: Time::ZERO,
                shuffle: false,
                repeat: RepeatMode::Off,
            }))
        })
        .clone()
}

fn notify() {
    if let Some(tx) = NOTIFY.get() {
        let _ = tx.send(true);
    }
}

struct P(Arc<Mutex<MprisState>>, Sender<SystemEvent>);

impl RootInterface for P {
    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }
    async fn quit(&self) -> fdo::Result<()> {
        Ok(())
    }
    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_fullscreen(&self, _: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn identity(&self) -> fdo::Result<String> {
        Ok("Kopuz".into())
    }
    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("kopuz".into())
    }
    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
}

impl PlayerInterface for P {
    async fn next(&self) -> fdo::Result<()> {
        self.1.send(SystemEvent::Next).ok();
        Ok(())
    }
    async fn previous(&self) -> fdo::Result<()> {
        self.1.send(SystemEvent::Prev).ok();
        Ok(())
    }
    async fn pause(&self) -> fdo::Result<()> {
        self.1.send(SystemEvent::Pause).ok();
        Ok(())
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        self.1.send(SystemEvent::Toggle).ok();
        Ok(())
    }
    async fn stop(&self) -> fdo::Result<()> {
        self.1.send(SystemEvent::Pause).ok();
        Ok(())
    }
    async fn play(&self) -> fdo::Result<()> {
        self.1.send(SystemEvent::Play).ok();
        Ok(())
    }
    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        // MPRIS Seek is relative to the current position.
        let current = self.0.lock().map(|s| s.position).unwrap_or(Time::ZERO);
        let target = (current.as_micros() + offset.as_micros()).max(0);
        self.1.send(SystemEvent::Seek(target as f64 / 1e6)).ok();
        Ok(())
    }
    async fn set_position(&self, _: mpris_server::TrackId, position: Time) -> fdo::Result<()> {
        // The trackid is constant (always the current track), so any request
        // that reached us refers to it.
        let target = position.as_micros().max(0);
        self.1.send(SystemEvent::Seek(target as f64 / 1e6)).ok();
        Ok(())
    }
    async fn open_uri(&self, _: String) -> fdo::Result<()> {
        Ok(())
    }
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self
            .0
            .lock()
            .map(|s| s.status)
            .unwrap_or(PlaybackStatus::Stopped))
    }
    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(self
            .0
            .lock()
            .map(|s| s.repeat.to_mpris())
            .unwrap_or(LoopStatus::None))
    }
    async fn set_loop_status(&self, status: LoopStatus) -> mpris_server::zbus::Result<()> {
        let repeat = RepeatMode::from_mpris(status);
        if let Ok(mut s) = self.0.lock() {
            s.repeat = repeat;
        }
        self.1.send(SystemEvent::SetRepeat(repeat)).ok();
        notify();
        Ok(())
    }
    async fn rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }
    async fn set_rate(&self, _: f64) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.0.lock().map(|s| s.shuffle).unwrap_or(false))
    }
    async fn set_shuffle(&self, shuffle: bool) -> mpris_server::zbus::Result<()> {
        if let Ok(mut s) = self.0.lock() {
            s.shuffle = shuffle;
        }
        self.1.send(SystemEvent::SetShuffle(shuffle)).ok();
        notify();
        Ok(())
    }
    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self
            .0
            .lock()
            .map(|s| s.metadata.clone())
            .unwrap_or_default())
    }
    async fn volume(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }
    async fn set_volume(&self, _: f64) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn position(&self) -> fdo::Result<Time> {
        Ok(self.0.lock().map(|s| s.position).unwrap_or(Time::ZERO))
    }
    async fn minimum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }
    async fn maximum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }
    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

pub fn update_position(position: f64) {
    setup();
    if let Ok(mut s) = state().lock() {
        s.position = Time::from_micros((position * 1e6) as i64);
    }
}

/// Push the current shuffle / repeat state (changed in the UI) to MPRIS
/// clients so their toggles stay in sync with Kopuz.
pub fn update_modes(shuffle: bool, repeat: RepeatMode) {
    setup();
    let changed = if let Ok(mut s) = state().lock() {
        let changed = s.shuffle != shuffle || s.repeat != repeat;
        s.shuffle = shuffle;
        s.repeat = repeat;
        changed
    } else {
        false
    };
    if changed {
        notify();
    }
}

fn setup() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let (ntx, mut nrx) = tokio::sync::mpsc::unbounded_channel();
        NOTIFY.set(ntx).ok();
        let st = state();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    if let Ok(srv) = Server::new("kopuz", P(st.clone(), tx())).await {
                        while let Some(seeked) = nrx.recv().await {
                            if seeked {
                                let (metadata, status, position, shuffle, repeat) = match st.lock()
                                {
                                    Ok(s) => (
                                        s.metadata.clone(),
                                        s.status,
                                        s.position,
                                        s.shuffle,
                                        s.repeat.to_mpris(),
                                    ),
                                    Err(_) => continue,
                                };
                                srv.properties_changed([
                                    Property::Metadata(metadata),
                                    Property::PlaybackStatus(status),
                                    Property::Shuffle(shuffle),
                                    Property::LoopStatus(repeat),
                                ])
                                .await
                                .ok();
                                srv.emit(mpris_server::Signal::Seeked { position })
                                    .await
                                    .ok();
                            }
                        }
                    }
                });
        });
    });
}

pub fn poll_event() -> Option<SystemEvent> {
    setup();
    RX.get()?.lock().ok()?.try_recv().ok()
}

pub fn update_now_playing(
    title: &str,
    artist: &str,
    album: &str,
    duration: f64,
    position: f64,
    playing: bool,
    artwork_path: Option<&str>,
) {
    setup();
    if let Ok(mut s) = state().lock() {
        let mut b = Metadata::builder()
            .title(title)
            .artist([artist])
            .album(album)
            .length(Time::from_micros((duration * 1e6) as i64));
        if let Ok(trackid) = mpris_server::TrackId::try_from(TRACK_ID) {
            b = b.trackid(trackid);
        }
        if let Some(art) = artwork_path {
            // MPRIS art_url accepts any URI. Pass remote URLs (Jellyfin
            // thumbs, YT Music covers) through unchanged so clients can
            // fetch them directly; only wrap actual local file paths
            // with file://.
            b = b.art_url(
                if art.starts_with("http://")
                    || art.starts_with("https://")
                    || art.starts_with("file://")
                {
                    art.to_string()
                } else if art.starts_with('/') {
                    format!("file://{art}")
                } else {
                    format!(
                        "file://{}/{art}",
                        std::env::current_dir().unwrap_or_default().display()
                    )
                },
            );
        }
        s.metadata = b.build();
        s.status = if playing {
            PlaybackStatus::Playing
        } else {
            PlaybackStatus::Paused
        };
        s.position = Time::from_micros((position * 1e6) as i64);
    }
    NOTIFY.get().map(|tx| tx.send(true));
}
