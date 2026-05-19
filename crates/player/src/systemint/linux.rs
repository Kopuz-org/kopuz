use mpris_server::{
    Metadata, PlaybackStatus, PlayerInterface, Property, RootInterface, Server, Time, zbus::fdo,
};
use std::sync::{
    Arc, Mutex, OnceLock,
    mpsc::{self, Receiver, Sender},
};

#[derive(Debug)]
pub enum SystemEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
}

static TX: OnceLock<Sender<SystemEvent>> = OnceLock::new();
static RX: OnceLock<Mutex<Receiver<SystemEvent>>> = OnceLock::new();
static STATE: OnceLock<Arc<Mutex<(Metadata, PlaybackStatus, Time)>>> = OnceLock::new();
static NOTIFY: OnceLock<tokio::sync::mpsc::UnboundedSender<bool>> = OnceLock::new();

fn tx() -> Sender<SystemEvent> {
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        RX.set(Mutex::new(rx)).ok();
        tx
    })
    .clone()
}

fn state() -> Arc<Mutex<(Metadata, PlaybackStatus, Time)>> {
    STATE
        .get_or_init(|| {
            Arc::new(Mutex::new((
                Metadata::new(),
                PlaybackStatus::Stopped,
                Time::ZERO,
            )))
        })
        .clone()
}

struct P(
    Arc<Mutex<(Metadata, PlaybackStatus, Time)>>,
    Sender<SystemEvent>,
);

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
    async fn seek(&self, _: Time) -> fdo::Result<()> {
        Ok(())
    }
    async fn set_position(&self, _: mpris_server::TrackId, _: Time) -> fdo::Result<()> {
        Ok(())
    }
    async fn open_uri(&self, _: String) -> fdo::Result<()> {
        Ok(())
    }
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self
            .0
            .lock()
            .map(|s| s.1)
            .unwrap_or(PlaybackStatus::Stopped))
    }
    async fn loop_status(&self) -> fdo::Result<mpris_server::LoopStatus> {
        Ok(mpris_server::LoopStatus::None)
    }
    async fn set_loop_status(&self, _: mpris_server::LoopStatus) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }
    async fn set_rate(&self, _: f64) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_shuffle(&self, _: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self.0.lock().map(|s| s.0.clone()).unwrap_or_default())
    }
    async fn volume(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }
    async fn set_volume(&self, _: f64) -> mpris_server::zbus::Result<()> {
        Ok(())
    }
    async fn position(&self) -> fdo::Result<Time> {
        Ok(self.0.lock().map(|s| s.2).unwrap_or(Time::ZERO))
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
        Ok(false)
    }
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

pub fn update_position(position: f64) {
    setup();
    if let Ok(mut s) = state().lock() {
        s.2 = Time::from_micros((position * 1e6) as i64);
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
                            if let Ok(s) = st.lock() {
                                if seeked {
                                    srv.properties_changed([
                                        Property::Metadata(s.0.clone()),
                                        Property::PlaybackStatus(s.1),
                                    ])
                                    .await
                                    .ok();
                                    srv.emit(mpris_server::Signal::Seeked { position: s.2 })
                                        .await
                                        .ok();
                                }
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
        if let Some(art) = artwork_path {
            b = b.art_url(if art.starts_with('/') {
                format!("file://{art}")
            } else {
                format!(
                    "file://{}/{art}",
                    std::env::current_dir().unwrap_or_default().display()
                )
            });
        }
        *s = (
            b.build(),
            if playing {
                PlaybackStatus::Playing
            } else {
                PlaybackStatus::Paused
            },
            Time::from_micros((position * 1e6) as i64),
        );
    }
    NOTIFY.get().map(|tx| tx.send(true));
}
