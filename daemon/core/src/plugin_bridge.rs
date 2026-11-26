//! What an integration plugin may do to kopuz, carried into the session: the
//! plugin host checks each call against the plugin's manifest first.

use api::{PlayerCommand, QueueContext, QueueMode, SetQueueRequest};
use plugin_host::wit::playback;
use plugin_host::{Answer, Bridge};

use crate::session::SessionHandle;

pub struct SessionBridge {
    session: SessionHandle,
}

impl SessionBridge {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
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

impl Bridge for SessionBridge {
    fn control(&self, order: playback::Command) -> Answer<()> {
        let session = self.session.clone();
        Box::pin(async move {
            session
                .player_command(command(order))
                .await
                .map(|_| ())
                .map_err(|error| error.message)
        })
    }

    fn queue(&self, keys: Vec<String>, mode: playback::QueueMode) -> Answer<()> {
        let session = self.session.clone();
        Box::pin(async move {
            if keys.is_empty() {
                return Err("there are no tracks to queue".into());
            }
            session
                .set_queue(queue_request(keys, mode))
                .await
                .map(|_| ())
                .map_err(|error| error.message)
        })
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
