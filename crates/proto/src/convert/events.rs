use super::*;
use crate::*;

pub fn event_to_proto(value: &api::ApiEvent) -> Event {
    let kind = match value {
        api::ApiEvent::PlayerState(state) => event::Kind::PlayerState(player_state_to_proto(state)),
        api::ApiEvent::PlayerPosition {
            token,
            position_ms,
            at_ms,
            playing,
        } => event::Kind::Position(PositionEvent {
            token: *token,
            position_ms: *position_ms,
            at_ms: *at_ms,
            playing: *playing,
        }),
        api::ApiEvent::PlayerBuffered { token, ranges } => event::Kind::Buffered(BufferedEvent {
            token: *token,
            ranges: ranges.iter().map(buffered_to_proto).collect(),
        }),
        api::ApiEvent::QueueChanged { rev, length, index } => {
            event::Kind::QueueChanged(QueueChanged {
                rev: *rev,
                length: *length,
                index: *index,
            })
        }
        api::ApiEvent::LibraryInvalidated { table, generation } => {
            event::Kind::LibraryInvalidated(LibraryInvalidated {
                table: table_to_proto(*table) as i32,
                generation: *generation,
            })
        }
        api::ApiEvent::JobProgress(progress) => event::Kind::JobProgress(JobProgress {
            id: progress.id.clone(),
            kind: job_kind_to_proto(progress.kind) as i32,
            phase: progress.phase.clone(),
            current: progress.current,
            total: progress.total,
            message: progress.message.clone(),
        }),
        api::ApiEvent::JobFinished {
            id,
            kind,
            ok,
            error,
        } => event::Kind::JobFinished(JobFinished {
            id: id.clone(),
            kind: job_kind_to_proto(*kind) as i32,
            ok: *ok,
            error: error.as_ref().map(error_body_to_proto),
        }),
        api::ApiEvent::ConfigChanged { keys } => {
            event::Kind::ConfigChanged(ConfigChanged { keys: keys.clone() })
        }
        api::ApiEvent::SourceStatus { source, state } => {
            event::Kind::SourceStatus(SourceStatusEvent {
                source: source.clone(),
                state: source_state_to_proto(*state) as i32,
            })
        }
        api::ApiEvent::Notice {
            level,
            code,
            message,
        } => event::Kind::Notice(Notice {
            level: notice_level_to_proto(*level) as i32,
            code: code.clone(),
            message: message.clone(),
        }),
        api::ApiEvent::Resync => event::Kind::Resync(Unit {}),
    };
    Event { kind: Some(kind) }
}

pub fn event_from_proto(value: &Event) -> Option<api::ApiEvent> {
    Some(match value.kind.as_ref()? {
        event::Kind::PlayerState(state) => {
            api::ApiEvent::PlayerState(Box::new(player_state_from_proto(state)))
        }
        event::Kind::Position(position) => api::ApiEvent::PlayerPosition {
            token: position.token,
            position_ms: position.position_ms,
            at_ms: position.at_ms,
            playing: position.playing,
        },
        event::Kind::Buffered(buffered) => api::ApiEvent::PlayerBuffered {
            token: buffered.token,
            ranges: buffered.ranges.iter().map(buffered_from_proto).collect(),
        },
        event::Kind::QueueChanged(changed) => api::ApiEvent::QueueChanged {
            rev: changed.rev,
            length: changed.length,
            index: changed.index,
        },
        event::Kind::LibraryInvalidated(invalidated) => api::ApiEvent::LibraryInvalidated {
            table: table_from_proto(invalidated.table),
            generation: invalidated.generation,
        },
        event::Kind::JobProgress(progress) => api::ApiEvent::JobProgress(api::JobProgress {
            id: progress.id.clone(),
            kind: job_kind_from_proto(progress.kind),
            phase: progress.phase.clone(),
            current: progress.current,
            total: progress.total,
            message: progress.message.clone(),
        }),
        event::Kind::JobFinished(finished) => api::ApiEvent::JobFinished {
            id: finished.id.clone(),
            kind: job_kind_from_proto(finished.kind),
            ok: finished.ok,
            error: finished.error.as_ref().map(error_body_from_proto),
        },
        event::Kind::ConfigChanged(changed) => api::ApiEvent::ConfigChanged {
            keys: changed.keys.clone(),
        },
        event::Kind::SourceStatus(status) => api::ApiEvent::SourceStatus {
            source: status.source.clone(),
            state: source_state_from_proto(status.state),
        },
        event::Kind::Notice(notice) => api::ApiEvent::Notice {
            level: notice_level_from_proto(notice.level),
            code: notice.code.clone(),
            message: notice.message.clone(),
        },
        event::Kind::Resync(_) => api::ApiEvent::Resync,
    })
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::sample_state;
    use super::*;

    #[test]
    fn every_event_round_trips() {
        let events = vec![
            api::ApiEvent::PlayerState(Box::new(sample_state())),
            api::ApiEvent::PlayerPosition {
                token: 1,
                position_ms: 2,
                at_ms: 3,
                playing: true,
            },
            api::ApiEvent::PlayerBuffered {
                token: 1,
                ranges: vec![api::BufferedRange {
                    start: 1,
                    end: 2,
                    total: None,
                }],
            },
            api::ApiEvent::QueueChanged {
                rev: 1,
                length: 2,
                index: Some(0),
            },
            api::ApiEvent::LibraryInvalidated {
                table: api::Table::Favorites,
                generation: 4,
            },
            api::ApiEvent::JobProgress(api::JobProgress {
                id: "j".into(),
                kind: api::JobKind::Scan,
                phase: "scanning".into(),
                current: Some(1),
                total: Some(2),
                message: Some("f".into()),
            }),
            api::ApiEvent::JobFinished {
                id: "j".into(),
                kind: api::JobKind::Download,
                ok: false,
                error: Some(api::ErrorBody {
                    code: api::ErrorCode::Conflict,
                    message: "busy".into(),
                }),
            },
            api::ApiEvent::ConfigChanged {
                keys: vec!["volume".into()],
            },
            api::ApiEvent::SourceStatus {
                source: "jellyfin".into(),
                state: api::SourceState::AuthExpired,
            },
            api::ApiEvent::Notice {
                level: api::NoticeLevel::Warning,
                code: "c".into(),
                message: None,
            },
            api::ApiEvent::Resync,
        ];
        for event in events {
            let back = event_from_proto(&event_to_proto(&event)).expect("event survives");
            assert_eq!(event, back);
        }
    }

    #[test]
    fn unknown_event_kind_is_ignorable() {
        assert!(event_from_proto(&Event { kind: None }).is_none());
    }
}
