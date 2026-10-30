use super::*;
use crate::*;

pub fn network_status_to_proto(value: &api::NetworkStatus) -> NetworkStatus {
    NetworkStatus {
        internet_down: value.internet_down,
        left_source: value.left_source.clone(),
        offline_mode: offline_mode_to_proto(value.offline_mode) as i32,
    }
}

pub fn network_status_from_proto(value: &NetworkStatus) -> api::NetworkStatus {
    api::NetworkStatus {
        internet_down: value.internet_down,
        left_source: value.left_source.clone(),
        offline_mode: offline_mode_from_proto(value.offline_mode),
    }
}

fn offline_mode_to_proto(value: api::OfflineMode) -> OfflineMode {
    match value {
        api::OfflineMode::Off => OfflineMode::Off,
        api::OfflineMode::Automatic => OfflineMode::Automatic,
        api::OfflineMode::Requested => OfflineMode::Requested,
    }
}

/// A mode this build does not know is still a mode that is on, so its listings stay offline.
fn offline_mode_from_proto(value: i32) -> api::OfflineMode {
    match OfflineMode::try_from(value) {
        Ok(OfflineMode::Off) => api::OfflineMode::Off,
        Ok(OfflineMode::Automatic) => api::OfflineMode::Automatic,
        Ok(OfflineMode::Requested) | Err(_) => api::OfflineMode::Requested,
    }
}

pub fn update_status_to_proto(value: &api::UpdateStatus) -> UpdateStatus {
    let state = match value {
        api::UpdateStatus::Current => update_status::State::Current(Unit {}),
        api::UpdateStatus::Available(update) => {
            update_status::State::Available(available_update_to_proto(update))
        }
        api::UpdateStatus::Downloading {
            version,
            done,
            total,
        } => update_status::State::Downloading(UpdateDownload {
            version: version.clone(),
            done: *done,
            total: *total,
        }),
        api::UpdateStatus::Ready { version } => update_status::State::Ready(version.clone()),
        api::UpdateStatus::Failed(update) => {
            update_status::State::Failed(available_update_to_proto(update))
        }
    };
    UpdateStatus { state: Some(state) }
}

/// A state this build does not know says nothing it could act on.
pub fn update_status_from_proto(value: &UpdateStatus) -> api::UpdateStatus {
    match &value.state {
        None | Some(update_status::State::Current(_)) => api::UpdateStatus::Current,
        Some(update_status::State::Available(update)) => {
            api::UpdateStatus::Available(available_update_from_proto(update))
        }
        Some(update_status::State::Downloading(download)) => api::UpdateStatus::Downloading {
            version: download.version.clone(),
            done: download.done,
            total: download.total,
        },
        Some(update_status::State::Ready(version)) => api::UpdateStatus::Ready {
            version: version.clone(),
        },
        Some(update_status::State::Failed(update)) => {
            api::UpdateStatus::Failed(available_update_from_proto(update))
        }
    }
}

fn available_update_to_proto(value: &api::AvailableUpdate) -> AvailableUpdate {
    AvailableUpdate {
        version: value.version.clone(),
        release_url: value.release_url.clone(),
        installable: value.installable,
    }
}

fn available_update_from_proto(value: &AvailableUpdate) -> api::AvailableUpdate {
    api::AvailableUpdate {
        version: value.version.clone(),
        release_url: value.release_url.clone(),
        installable: value.installable,
    }
}

pub fn offline_cache_status_to_proto(value: &api::OfflineCacheStatus) -> OfflineCacheStatus {
    OfflineCacheStatus {
        enabled: value.enabled,
        max_bytes: value.max_bytes,
        used_bytes: value.used_bytes,
        tracks: value.tracks,
    }
}

pub fn offline_cache_status_from_proto(value: &OfflineCacheStatus) -> api::OfflineCacheStatus {
    api::OfflineCacheStatus {
        enabled: value.enabled,
        max_bytes: value.max_bytes,
        used_bytes: value.used_bytes,
        tracks: value.tracks,
    }
}

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
        api::ApiEvent::LibraryInvalidated { table } => {
            event::Kind::LibraryInvalidated(LibraryInvalidated {
                table: table_to_proto(*table) as i32,
            })
        }
        api::ApiEvent::JobProgress(progress) => event::Kind::JobProgress(JobProgress {
            id: progress.id.clone(),
            kind: job_kind_to_proto(progress.kind) as i32,
            phase: progress.phase.clone(),
            current: progress.current,
            total: progress.total,
            message: progress.message.clone(),
            automatic: progress.automatic,
        }),
        api::ApiEvent::JobFinished {
            id,
            kind,
            ok,
            error,
            automatic,
        } => event::Kind::JobFinished(JobFinished {
            id: id.clone(),
            kind: job_kind_to_proto(*kind) as i32,
            ok: *ok,
            error: error.as_ref().map(error_body_to_proto),
            automatic: *automatic,
        }),
        api::ApiEvent::ConfigChanged { revision } => event::Kind::ConfigChanged(ConfigChanged {
            revision: *revision,
        }),
        api::ApiEvent::FrontendPrefsChanged { frontend, keys } => {
            event::Kind::FrontendPrefsChanged(FrontendPrefsChanged {
                frontend: frontend.clone(),
                keys: keys.clone(),
            })
        }
        api::ApiEvent::SourceStatus { source, state } => {
            event::Kind::SourceStatus(SourceStatusEvent {
                source: source.clone(),
                state: source_state_to_proto(*state) as i32,
            })
        }
        api::ApiEvent::NetworkStatus(status) => {
            event::Kind::NetworkStatus(network_status_to_proto(status))
        }
        api::ApiEvent::Update(status) => event::Kind::Update(update_status_to_proto(status)),
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
        },
        event::Kind::JobProgress(progress) => api::ApiEvent::JobProgress(api::JobProgress {
            id: progress.id.clone(),
            kind: job_kind_from_proto(progress.kind),
            phase: progress.phase.clone(),
            current: progress.current,
            total: progress.total,
            message: progress.message.clone(),
            automatic: progress.automatic,
        }),
        event::Kind::JobFinished(finished) => api::ApiEvent::JobFinished {
            id: finished.id.clone(),
            kind: job_kind_from_proto(finished.kind),
            ok: finished.ok,
            error: finished.error.as_ref().map(error_body_from_proto),
            automatic: finished.automatic,
        },
        event::Kind::ConfigChanged(changed) => api::ApiEvent::ConfigChanged {
            revision: changed.revision,
        },
        event::Kind::FrontendPrefsChanged(changed) => api::ApiEvent::FrontendPrefsChanged {
            frontend: changed.frontend.clone(),
            keys: changed.keys.clone(),
        },
        event::Kind::SourceStatus(status) => api::ApiEvent::SourceStatus {
            source: status.source.clone(),
            state: source_state_from_proto(status.state),
        },
        event::Kind::NetworkStatus(status) => {
            api::ApiEvent::NetworkStatus(network_status_from_proto(status))
        }
        event::Kind::Update(status) => api::ApiEvent::Update(update_status_from_proto(status)),
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
            },
            api::ApiEvent::JobProgress(api::JobProgress {
                id: "j".into(),
                kind: api::JobKind::Scan,
                phase: "scanning".into(),
                current: Some(1),
                total: Some(2),
                message: Some("f".into()),
                automatic: true,
            }),
            api::ApiEvent::JobFinished {
                id: "j".into(),
                kind: api::JobKind::Download,
                ok: false,
                error: Some(api::ErrorBody {
                    code: api::ErrorCode::Conflict,
                    message: "busy".into(),
                    reason: Some(api::ErrorReason::SignInTimedOut),
                }),
                automatic: false,
            },
            api::ApiEvent::JobFinished {
                id: "k".into(),
                kind: api::JobKind::LibrarySync,
                ok: true,
                error: None,
                automatic: true,
            },
            api::ApiEvent::ConfigChanged { revision: 9 },
            api::ApiEvent::FrontendPrefsChanged {
                frontend: "gpui".into(),
                keys: vec!["skin".into(), "columns".into()],
            },
            api::ApiEvent::SourceStatus {
                source: "jellyfin".into(),
                state: api::SourceState::AuthExpired,
            },
            api::ApiEvent::NetworkStatus(api::NetworkStatus {
                internet_down: true,
                left_source: Some("yt".into()),
                offline_mode: api::OfflineMode::Off,
            }),
            api::ApiEvent::NetworkStatus(api::NetworkStatus {
                internet_down: true,
                left_source: None,
                offline_mode: api::OfflineMode::Automatic,
            }),
            api::ApiEvent::NetworkStatus(api::NetworkStatus {
                offline_mode: api::OfflineMode::Requested,
                ..Default::default()
            }),
            api::ApiEvent::NetworkStatus(api::NetworkStatus::default()),
            api::ApiEvent::Notice {
                level: api::NoticeLevel::Warning,
                code: "c".into(),
                message: None,
            },
            api::ApiEvent::Update(api::UpdateStatus::Current),
            api::ApiEvent::Update(api::UpdateStatus::Available(api::AvailableUpdate {
                version: "1.0.0".into(),
                release_url: "https://example.invalid/v1.0.0".into(),
                installable: true,
            })),
            api::ApiEvent::Update(api::UpdateStatus::Downloading {
                version: "1.0.0".into(),
                done: 5,
                total: None,
            }),
            api::ApiEvent::Update(api::UpdateStatus::Ready {
                version: "1.0.0".into(),
            }),
            api::ApiEvent::Update(api::UpdateStatus::Failed(api::AvailableUpdate::default())),
            api::ApiEvent::Resync,
        ];
        for event in events {
            let back = event_from_proto(&event_to_proto(&event)).expect("event survives");
            assert_eq!(event, back);
        }
    }

    #[test]
    fn an_older_daemon_is_never_offline_and_an_unknown_mode_is_on() {
        let older = network_status_from_proto(&NetworkStatus::default());
        assert_eq!(older.offline_mode, api::OfflineMode::Off);
        let newer = network_status_from_proto(&NetworkStatus {
            offline_mode: 99,
            ..Default::default()
        });
        assert!(newer.offline_mode.is_on());
    }

    #[test]
    fn the_offline_cache_status_round_trips() {
        let status = api::OfflineCacheStatus {
            enabled: true,
            max_bytes: 5_000_000_000,
            used_bytes: 123_456_789,
            tracks: 42,
        };
        assert_eq!(
            offline_cache_status_from_proto(&offline_cache_status_to_proto(&status)),
            status
        );
    }

    #[test]
    fn unknown_event_kind_is_ignorable() {
        assert!(event_from_proto(&Event { kind: None }).is_none());
    }
}
