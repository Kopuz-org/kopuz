use super::*;
use crate::*;

pub fn intent_to_proto(value: &api::Intent) -> Intent {
    let kind = match value {
        api::Intent::Stopped => intent::Kind::Stopped(Unit {}),
        api::Intent::Loading { token, from_token } => intent::Kind::Loading(intent::Loading {
            token: *token,
            from_token: *from_token,
        }),
        api::Intent::Committed { token } => {
            intent::Kind::Committed(intent::Committed { token: *token })
        }
    };
    Intent { kind: Some(kind) }
}

pub fn intent_from_proto(value: Option<&Intent>) -> api::Intent {
    match value.and_then(|intent| intent.kind.as_ref()) {
        Some(intent::Kind::Loading(loading)) => api::Intent::Loading {
            token: loading.token,
            from_token: loading.from_token,
        },
        Some(intent::Kind::Committed(committed)) => api::Intent::Committed {
            token: committed.token,
        },
        Some(intent::Kind::Stopped(_)) | None => api::Intent::Stopped,
    }
}

pub fn now_playing_to_proto(value: &api::NowPlaying) -> NowPlaying {
    NowPlaying {
        key: value.key.clone(),
        uid: value.uid.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        duration_ms: value.duration_ms,
        khz: value.khz,
        bitrate: u32::from(value.bitrate),
        kind: track_kind_to_proto(value.kind) as i32,
        seekable: value.seekable,
    }
}

pub fn now_playing_from_proto(value: &NowPlaying) -> api::NowPlaying {
    api::NowPlaying {
        key: value.key.clone(),
        uid: value.uid.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        duration_ms: value.duration_ms,
        khz: value.khz,
        bitrate: value.bitrate.min(u32::from(u16::MAX)) as u16,
        kind: track_kind_from_proto(value.kind),
        seekable: value.seekable,
    }
}

pub fn anchor_to_proto(value: &api::PositionAnchor) -> PositionAnchor {
    PositionAnchor {
        ms: value.ms,
        at_ms: value.at_ms,
        playing: value.playing,
    }
}

pub fn anchor_from_proto(value: &PositionAnchor) -> api::PositionAnchor {
    api::PositionAnchor {
        ms: value.ms,
        at_ms: value.at_ms,
        playing: value.playing,
    }
}

pub fn buffered_to_proto(value: &api::BufferedRange) -> BufferedRange {
    BufferedRange {
        start: value.start,
        end: value.end,
        total: value.total,
    }
}

pub fn buffered_from_proto(value: &BufferedRange) -> api::BufferedRange {
    api::BufferedRange {
        start: value.start,
        end: value.end,
        total: value.total,
    }
}

pub fn queue_summary_to_proto(value: &api::QueueSummary) -> QueueSummary {
    QueueSummary {
        rev: value.rev,
        length: value.length,
        index: value.index,
        shuffle: value.shuffle,
        r#loop: loop_to_proto(value.loop_mode) as i32,
    }
}

pub fn queue_summary_from_proto(value: Option<&QueueSummary>) -> api::QueueSummary {
    let value = value.cloned().unwrap_or_default();
    api::QueueSummary {
        rev: value.rev,
        length: value.length,
        index: value.index,
        shuffle: value.shuffle,
        loop_mode: loop_from_proto(value.r#loop),
    }
}

pub fn player_state_to_proto(value: &api::PlayerState) -> PlayerState {
    PlayerState {
        rev: value.rev,
        now_ms: value.now_ms,
        phase: phase_to_proto(value.phase) as i32,
        intent: Some(intent_to_proto(&value.intent)),
        track: value.track.as_ref().map(now_playing_to_proto),
        position: value.position.as_ref().map(anchor_to_proto),
        queue: Some(queue_summary_to_proto(&value.queue)),
        volume: value.volume,
        buffered: value.buffered.iter().map(buffered_to_proto).collect(),
        fading: value.fading.as_ref().map(|fading| FadingState {
            from_token: fading.from_token,
            track: Some(now_playing_to_proto(&fading.track)),
            position_ms: fading.position_ms,
        }),
        external: value.external.as_ref().map(|external| ExternalPlayback {
            kind: external.kind.clone(),
            device: external.device.clone(),
        }),
        error: value.error.as_ref().map(error_body_to_proto),
    }
}

pub fn player_state_from_proto(value: &PlayerState) -> api::PlayerState {
    api::PlayerState {
        rev: value.rev,
        now_ms: value.now_ms,
        phase: phase_from_proto(value.phase),
        intent: intent_from_proto(value.intent.as_ref()),
        track: value.track.as_ref().map(now_playing_from_proto),
        position: value.position.as_ref().map(anchor_from_proto),
        queue: queue_summary_from_proto(value.queue.as_ref()),
        volume: value.volume,
        buffered: value.buffered.iter().map(buffered_from_proto).collect(),
        fading: value.fading.as_ref().map(|fading| api::FadingState {
            from_token: fading.from_token,
            track: fading
                .track
                .as_ref()
                .map(now_playing_from_proto)
                .unwrap_or_default(),
            position_ms: fading.position_ms,
        }),
        external: value
            .external
            .as_ref()
            .map(|external| api::ExternalPlayback {
                kind: external.kind.clone(),
                device: external.device.clone(),
            }),
        error: value.error.as_ref().map(error_body_from_proto),
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::sample_state;
    use super::*;

    #[test]
    fn player_state_round_trips() {
        let state = sample_state();
        let back = player_state_from_proto(&player_state_to_proto(&state));
        assert_eq!(state, back);
    }
}
