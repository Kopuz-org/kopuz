//! What is left of the frontend's playback orchestration: the OS surfaces
//! only this process can reach.
//!
//! Engine playback, the media widget, external playback and every listen it
//! records live in the daemon. On Linux the shuffle and repeat modes the UI
//! toggles are mirrored into MPRIS here, and on Android the media
//! notification's taps arrive through a JNI callback with no event queue, so
//! they are drained and dispatched through the controller.

#[cfg(any(target_os = "linux", target_os = "android"))]
use crate::use_player_controller::LoopMode;
use crate::use_player_controller::PlayerController;
use dioxus::prelude::*;

#[cfg(target_os = "android")]
mod android_media {
    #[derive(Debug, Clone, Copy)]
    pub(super) enum BgCmd {
        Play,
        Pause,
        Toggle,
        Next,
        Prev,
        ToggleShuffle,
        CycleRepeat,
    }

    pub(super) static BG_CMD_TX: std::sync::OnceLock<
        std::sync::Mutex<std::sync::mpsc::Sender<BgCmd>>,
    > = std::sync::OnceLock::new();
    pub(super) static BG_CMD_RX: std::sync::OnceLock<
        std::sync::Mutex<std::sync::mpsc::Receiver<BgCmd>>,
    > = std::sync::OnceLock::new();
    pub(super) static BG_NOTIFY: std::sync::OnceLock<tokio::sync::Notify> =
        std::sync::OnceLock::new();

    pub(super) fn init_bg_channel() {
        BG_CMD_TX.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<BgCmd>();
            let _ = BG_CMD_RX.set(std::sync::Mutex::new(rx));
            std::sync::Mutex::new(tx)
        });
        BG_NOTIFY.get_or_init(tokio::sync::Notify::new);
    }

    pub(super) fn send_bg_cmd(cmd: BgCmd) {
        if let Some(lock) = BG_CMD_TX.get()
            && let Ok(tx) = lock.lock()
        {
            let _ = tx.send(cmd);
        }
        if let Some(notify) = BG_NOTIFY.get() {
            notify.notify_one();
        }
    }

    pub(super) fn drain_bg_cmds() -> Vec<BgCmd> {
        let mut cmds = Vec::new();
        if let Some(lock) = BG_CMD_RX.get()
            && let Ok(rx) = lock.try_lock()
        {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
        }
        cmds
    }
}

pub fn use_player_task(ctrl: PlayerController) {
    // Keep MPRIS / the Android notification in sync with the UI's own toggles.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    use_effect(move || {
        let shuffle = *ctrl.shuffle.read();
        let repeat = match *ctrl.loop_mode.read() {
            LoopMode::None => player::systemint::RepeatMode::Off,
            LoopMode::Queue => player::systemint::RepeatMode::Playlist,
            LoopMode::Track => player::systemint::RepeatMode::Track,
        };
        player::systemint::update_modes(shuffle, repeat);
    });

    // Android routes media-notification button taps through a JNI callback (no
    // event queue) and the daemon's os_media task has no Android backend, so
    // the taps are drained here and dispatched through the controller.
    #[cfg(target_os = "android")]
    {
        use android_media::BgCmd;
        use_hook(move || {
            android_media::init_bg_channel();
            // Runs on the event loop thread, which is the only place its looper
            // can be picked up — see `capture_event_loop`.
            player::systemint::capture_event_loop();
            // The keepalive ticker pokes this while the activity is hidden, so
            // the loop below keeps draining commands and advancing the queue.
            player::systemint::set_tokio_waker(|| {
                if let Some(notify) = android_media::BG_NOTIFY.get() {
                    notify.notify_one();
                }
            });
            player::systemint::set_background_handler(move |event| {
                use player::systemint::SystemEvent;
                let cmd = match event {
                    SystemEvent::Play => BgCmd::Play,
                    SystemEvent::Pause => BgCmd::Pause,
                    SystemEvent::Toggle => BgCmd::Toggle,
                    SystemEvent::Next => BgCmd::Next,
                    SystemEvent::Prev => BgCmd::Prev,
                    SystemEvent::Stop => BgCmd::Pause,
                    SystemEvent::ToggleShuffle => BgCmd::ToggleShuffle,
                    SystemEvent::CycleRepeat => BgCmd::CycleRepeat,
                };
                android_media::send_bg_cmd(cmd);
            });
        });
        use_future(move || {
            let mut ctrl = ctrl;
            async move {
                loop {
                    for cmd in android_media::drain_bg_cmds() {
                        match cmd {
                            BgCmd::Play => ctrl.resume(),
                            BgCmd::Pause => ctrl.pause(),
                            BgCmd::Toggle => ctrl.toggle(),
                            BgCmd::Next => ctrl.play_next(),
                            BgCmd::Prev => ctrl.play_prev(),
                            BgCmd::ToggleShuffle => ctrl.toggle_shuffle(),
                            BgCmd::CycleRepeat => ctrl.toggle_loop(),
                        }
                    }
                    let notified = async {
                        if let Some(notify) = android_media::BG_NOTIFY.get() {
                            notify.notified().await;
                        } else {
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        }
                    };
                    tokio::select! {
                        _ = notified => {}
                        _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                    }
                }
            }
        });
    }
}
