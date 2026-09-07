//! Sample `api` values shared by the round-trip tests.

pub(super) fn sample_state() -> api::PlayerState {
    api::PlayerState {
        rev: 41,
        now_ms: 182_734,
        phase: api::Phase::Playing,
        intent: api::Intent::Loading {
            token: 7,
            from_token: Some(6),
        },
        track: Some(api::NowPlaying {
            key: "k".into(),
            uid: "ytmusic:k".into(),
            title: "t".into(),
            artist: "a".into(),
            album: "al".into(),
            duration_ms: Some(223_000),
            khz: 44,
            bitrate: 320,
            kind: api::TrackKind::Normal,
            seekable: true,
        }),
        position: Some(api::PositionAnchor {
            ms: 63_210,
            at_ms: 182_734,
            playing: true,
        }),
        queue: api::QueueSummary {
            rev: 39,
            length: 42,
            index: Some(3),
            shuffle: true,
            loop_mode: api::LoopMode::Queue,
        },
        volume: 0.8,
        buffered: vec![api::BufferedRange {
            start: 0,
            end: 4096,
            total: Some(8192),
        }],
        fading: Some(api::FadingState {
            from_token: 6,
            track: api::NowPlaying::default(),
            position_ms: 1000,
        }),
        external: Some(api::ExternalPlayback {
            kind: "spotify".into(),
            device: None,
        }),
        error: Some(api::ErrorBody {
            code: api::ErrorCode::SourceUnreachable,
            message: "m".into(),
        }),
    }
}
