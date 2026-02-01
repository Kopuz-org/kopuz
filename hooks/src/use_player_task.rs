use crate::use_player_controller::PlayerController;
use dioxus::prelude::*;

pub fn use_player_task(ctrl: PlayerController) {
    use_future(move || {
        let mut ctrl = ctrl;
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                #[cfg(target_os = "macos")]
                {
                    use player::systemint::{SystemEvent, poll_event};
                    while let Some(event) = poll_event() {
                        match event {
                            SystemEvent::Play => ctrl.resume(),
                            SystemEvent::Pause => ctrl.pause(),
                            SystemEvent::Toggle => ctrl.toggle(),
                            SystemEvent::Next => ctrl.play_next(),
                            SystemEvent::Prev => ctrl.play_prev(),
                        }
                    }
                }

                #[cfg(target_os = "linux")]
                {
                    use player::systemint::{SystemEvent, poll_event};
                    while let Some(event) = poll_event() {
                        match event {
                            SystemEvent::Play => ctrl.resume(),
                            SystemEvent::Pause => ctrl.pause(),
                            SystemEvent::Toggle => ctrl.toggle(),
                            SystemEvent::Next => ctrl.play_next(),
                            SystemEvent::Prev => ctrl.play_prev(),
                        }
                    }
                }

                if *ctrl.is_playing.read() {
                    let pos = ctrl.player.read().get_position();
                    ctrl.current_song_progress.set(pos.as_secs());

                    if ctrl.player.read().is_empty() {
                        ctrl.play_next();
                    }
                }
            }
        }
    });
}
