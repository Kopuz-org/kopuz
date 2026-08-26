//! The Kopuz wire contract: tonic/prost types generated from
//! `proto/kopuz.proto`, plus lossless conversions to and from the
//! in-process `api` types. The daemon serves proto at its boundary and
//! thinks in `api` types everywhere else; wire clients do the reverse.
//! The round-trip tests below are the fidelity guard: every `api` value
//! must survive api -> proto -> api unchanged.

mod generated {
    #![allow(clippy::large_enum_variant)]
    tonic::include_proto!("kopuz.v1");
}
pub use generated::*;

/// The encoded file descriptor set, for gRPC server reflection (the
/// `grpcurl` story).
pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("kopuz");

pub mod convert {
    use super::*;

    pub fn phase_to_proto(value: api::Phase) -> Phase {
        match value {
            api::Phase::Idle => Phase::Idle,
            api::Phase::Playing => Phase::Playing,
            api::Phase::Paused => Phase::Paused,
            api::Phase::Ended => Phase::Ended,
        }
    }

    pub fn phase_from_proto(value: i32) -> api::Phase {
        match Phase::try_from(value).unwrap_or(Phase::Unspecified) {
            Phase::Playing => api::Phase::Playing,
            Phase::Paused => api::Phase::Paused,
            Phase::Ended => api::Phase::Ended,
            Phase::Idle | Phase::Unspecified => api::Phase::Idle,
        }
    }

    pub fn loop_to_proto(value: api::LoopMode) -> LoopMode {
        match value {
            api::LoopMode::None => LoopMode::None,
            api::LoopMode::Queue => LoopMode::Queue,
            api::LoopMode::Track => LoopMode::Track,
        }
    }

    pub fn loop_from_proto(value: i32) -> api::LoopMode {
        match LoopMode::try_from(value).unwrap_or(LoopMode::Unspecified) {
            LoopMode::Queue => api::LoopMode::Queue,
            LoopMode::Track => api::LoopMode::Track,
            LoopMode::None | LoopMode::Unspecified => api::LoopMode::None,
        }
    }

    pub fn track_kind_to_proto(value: api::TrackKind) -> TrackKind {
        match value {
            api::TrackKind::Normal => TrackKind::Normal,
            api::TrackKind::Radio => TrackKind::Radio,
        }
    }

    pub fn track_kind_from_proto(value: i32) -> api::TrackKind {
        match TrackKind::try_from(value).unwrap_or(TrackKind::Unspecified) {
            TrackKind::Radio => api::TrackKind::Radio,
            TrackKind::Normal | TrackKind::Unspecified => api::TrackKind::Normal,
        }
    }

    pub fn queue_mode_to_proto(value: api::QueueMode) -> QueueMode {
        match value {
            api::QueueMode::Replace => QueueMode::Replace,
            api::QueueMode::Append => QueueMode::Append,
            api::QueueMode::PlayNext => QueueMode::PlayNext,
            api::QueueMode::Insert => QueueMode::Insert,
        }
    }

    pub fn queue_mode_from_proto(value: i32) -> api::QueueMode {
        match QueueMode::try_from(value).unwrap_or(QueueMode::Unspecified) {
            QueueMode::Append => api::QueueMode::Append,
            QueueMode::PlayNext => api::QueueMode::PlayNext,
            QueueMode::Insert => api::QueueMode::Insert,
            QueueMode::Replace | QueueMode::Unspecified => api::QueueMode::Replace,
        }
    }

    pub fn table_to_proto(value: api::Table) -> Table {
        match value {
            api::Table::Tracks => Table::Tracks,
            api::Table::Albums => Table::Albums,
            api::Table::Playlists => Table::Playlists,
            api::Table::Favorites => Table::Favorites,
            api::Table::Folders => Table::Folders,
            api::Table::Servers => Table::Servers,
            api::Table::Recents => Table::Recents,
            api::Table::Unknown => Table::Unspecified,
        }
    }

    pub fn table_from_proto(value: i32) -> api::Table {
        match Table::try_from(value).unwrap_or(Table::Unspecified) {
            Table::Tracks => api::Table::Tracks,
            Table::Albums => api::Table::Albums,
            Table::Playlists => api::Table::Playlists,
            Table::Favorites => api::Table::Favorites,
            Table::Folders => api::Table::Folders,
            Table::Servers => api::Table::Servers,
            Table::Recents => api::Table::Recents,
            Table::Unspecified => api::Table::Unknown,
        }
    }

    pub fn job_kind_to_proto(value: api::JobKind) -> JobKind {
        match value {
            api::JobKind::Scan => JobKind::Scan,
            api::JobKind::LibrarySync => JobKind::LibrarySync,
            api::JobKind::FavoritesSync => JobKind::FavoritesSync,
            api::JobKind::PlaylistSync => JobKind::PlaylistSync,
            api::JobKind::Download => JobKind::Download,
            api::JobKind::Ytdlp => JobKind::Ytdlp,
            api::JobKind::Unknown => JobKind::Unspecified,
        }
    }

    pub fn job_kind_from_proto(value: i32) -> api::JobKind {
        match JobKind::try_from(value).unwrap_or(JobKind::Unspecified) {
            JobKind::Scan => api::JobKind::Scan,
            JobKind::LibrarySync => api::JobKind::LibrarySync,
            JobKind::FavoritesSync => api::JobKind::FavoritesSync,
            JobKind::PlaylistSync => api::JobKind::PlaylistSync,
            JobKind::Download => api::JobKind::Download,
            JobKind::Ytdlp => api::JobKind::Ytdlp,
            JobKind::Unspecified => api::JobKind::Unknown,
        }
    }

    pub fn job_state_to_proto(value: api::JobState) -> JobState {
        match value {
            api::JobState::Running => JobState::Running,
            api::JobState::Finished => JobState::Finished,
            api::JobState::Failed => JobState::Failed,
            api::JobState::Cancelled => JobState::Cancelled,
            api::JobState::Unknown => JobState::Unspecified,
        }
    }

    pub fn job_state_from_proto(value: i32) -> api::JobState {
        match JobState::try_from(value).unwrap_or(JobState::Unspecified) {
            JobState::Running => api::JobState::Running,
            JobState::Finished => api::JobState::Finished,
            JobState::Cancelled => api::JobState::Cancelled,
            JobState::Failed => api::JobState::Failed,
            JobState::Unspecified => api::JobState::Unknown,
        }
    }

    pub fn source_state_to_proto(value: api::SourceState) -> SourceState {
        match value {
            api::SourceState::Online => SourceState::Online,
            api::SourceState::AuthExpired => SourceState::AuthExpired,
            api::SourceState::Offline => SourceState::Offline,
        }
    }

    pub fn source_state_from_proto(value: i32) -> api::SourceState {
        match SourceState::try_from(value).unwrap_or(SourceState::Unspecified) {
            SourceState::Online => api::SourceState::Online,
            SourceState::AuthExpired => api::SourceState::AuthExpired,
            SourceState::Offline | SourceState::Unspecified => api::SourceState::Offline,
        }
    }

    pub fn notice_level_to_proto(value: api::NoticeLevel) -> NoticeLevel {
        match value {
            api::NoticeLevel::Info => NoticeLevel::Info,
            api::NoticeLevel::Warning => NoticeLevel::Warning,
            api::NoticeLevel::Error => NoticeLevel::Error,
            api::NoticeLevel::Unknown => NoticeLevel::Unspecified,
        }
    }

    pub fn notice_level_from_proto(value: i32) -> api::NoticeLevel {
        match NoticeLevel::try_from(value).unwrap_or(NoticeLevel::Unspecified) {
            NoticeLevel::Info => api::NoticeLevel::Info,
            NoticeLevel::Warning => api::NoticeLevel::Warning,
            NoticeLevel::Error => api::NoticeLevel::Error,
            NoticeLevel::Unspecified => api::NoticeLevel::Unknown,
        }
    }

    pub fn error_code_to_proto(value: api::ErrorCode) -> ErrorCode {
        match value {
            api::ErrorCode::InvalidInput => ErrorCode::InvalidInput,
            api::ErrorCode::Unauthorized => ErrorCode::Unauthorized,
            api::ErrorCode::NotFound => ErrorCode::NotFound,
            api::ErrorCode::Conflict => ErrorCode::Conflict,
            api::ErrorCode::SourceAuthExpired => ErrorCode::SourceAuthExpired,
            api::ErrorCode::SourceUnreachable => ErrorCode::SourceUnreachable,
            api::ErrorCode::Unsupported => ErrorCode::Unsupported,
            api::ErrorCode::Internal => ErrorCode::Internal,
        }
    }

    pub fn error_code_from_proto(value: i32) -> api::ErrorCode {
        match ErrorCode::try_from(value).unwrap_or(ErrorCode::Unspecified) {
            ErrorCode::InvalidInput => api::ErrorCode::InvalidInput,
            ErrorCode::Unauthorized => api::ErrorCode::Unauthorized,
            ErrorCode::NotFound => api::ErrorCode::NotFound,
            ErrorCode::Conflict => api::ErrorCode::Conflict,
            ErrorCode::SourceAuthExpired => api::ErrorCode::SourceAuthExpired,
            ErrorCode::SourceUnreachable => api::ErrorCode::SourceUnreachable,
            ErrorCode::Unsupported => api::ErrorCode::Unsupported,
            ErrorCode::Internal | ErrorCode::Unspecified => api::ErrorCode::Internal,
        }
    }

    pub fn error_body_to_proto(value: &api::ErrorBody) -> ErrorBody {
        ErrorBody {
            code: error_code_to_proto(value.code) as i32,
            message: value.message.clone(),
            details_json: value.details.as_ref().map(|details| details.to_string()),
        }
    }

    pub fn error_body_from_proto(value: &ErrorBody) -> api::ErrorBody {
        api::ErrorBody {
            code: error_code_from_proto(value.code),
            message: value.message.clone(),
            details: value
                .details_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
        }
    }

    pub fn api_error_to_proto(value: &api::ApiError) -> ErrorBody {
        ErrorBody {
            code: error_code_to_proto(value.code) as i32,
            message: value.message.clone(),
            details_json: value.details.as_ref().map(|details| details.to_string()),
        }
    }

    pub fn api_error_from_proto(value: &ErrorBody) -> api::ApiError {
        api::ApiError {
            code: error_code_from_proto(value.code),
            message: value.message.clone(),
            details: value
                .details_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
        }
    }

    pub fn intent_to_proto(value: &api::Intent) -> Intent {
        let kind = match value {
            api::Intent::Stopped => intent::Kind::Stopped(Empty {}),
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
            title: value.title.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            duration_ms: value.duration_ms,
            khz: value.khz,
            bitrate: u32::from(value.bitrate),
            kind: track_kind_to_proto(value.kind) as i32,
            seekable: value.seekable,
            artwork: value.artwork.clone(),
        }
    }

    pub fn now_playing_from_proto(value: &NowPlaying) -> api::NowPlaying {
        api::NowPlaying {
            key: value.key.clone(),
            title: value.title.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            duration_ms: value.duration_ms,
            khz: value.khz,
            bitrate: value.bitrate.min(u32::from(u16::MAX)) as u16,
            kind: track_kind_from_proto(value.kind),
            seekable: value.seekable,
            artwork: value.artwork.clone(),
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
            output_latency_ms: value.output_latency_ms,
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
            output_latency_ms: value.output_latency_ms,
        }
    }

    pub fn command_to_proto(value: &api::PlayerCommand, request_id: u64) -> Command {
        let cmd = match value {
            api::PlayerCommand::Play => command::Cmd::Play(Empty {}),
            api::PlayerCommand::Pause => command::Cmd::Pause(Empty {}),
            api::PlayerCommand::Toggle => command::Cmd::Toggle(Empty {}),
            api::PlayerCommand::Next => command::Cmd::Next(Empty {}),
            api::PlayerCommand::Previous => command::Cmd::Previous(Empty {}),
            api::PlayerCommand::Stop => command::Cmd::Stop(Empty {}),
            api::PlayerCommand::Seek { position_ms } => command::Cmd::Seek(Seek {
                position_ms: *position_ms,
            }),
            api::PlayerCommand::SetVolume { volume } => {
                command::Cmd::Volume(SetVolume { volume: *volume })
            }
            api::PlayerCommand::SetMode { shuffle, loop_mode } => command::Cmd::Mode(SetMode {
                shuffle: *shuffle,
                r#loop: loop_mode.map(|mode| loop_to_proto(mode) as i32),
            }),
        };
        Command {
            request_id,
            cmd: Some(cmd),
        }
    }

    pub fn command_from_proto(value: &Command) -> Option<api::PlayerCommand> {
        Some(match value.cmd.as_ref()? {
            command::Cmd::Play(_) => api::PlayerCommand::Play,
            command::Cmd::Pause(_) => api::PlayerCommand::Pause,
            command::Cmd::Toggle(_) => api::PlayerCommand::Toggle,
            command::Cmd::Next(_) => api::PlayerCommand::Next,
            command::Cmd::Previous(_) => api::PlayerCommand::Previous,
            command::Cmd::Stop(_) => api::PlayerCommand::Stop,
            command::Cmd::Seek(seek) => api::PlayerCommand::Seek {
                position_ms: seek.position_ms,
            },
            command::Cmd::Volume(volume) => api::PlayerCommand::SetVolume {
                volume: volume.volume,
            },
            command::Cmd::Mode(mode) => api::PlayerCommand::SetMode {
                shuffle: mode.shuffle,
                loop_mode: mode.r#loop.map(loop_from_proto),
            },
        })
    }

    pub fn event_to_proto(value: &api::ApiEvent) -> Event {
        let kind = match value {
            api::ApiEvent::PlayerState(state) => {
                event::Kind::PlayerState(player_state_to_proto(state))
            }
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
            api::ApiEvent::PlayerBuffered { token, ranges } => {
                event::Kind::Buffered(BufferedEvent {
                    token: *token,
                    ranges: ranges.iter().map(buffered_to_proto).collect(),
                })
            }
            api::ApiEvent::PlayerExternalCommand(command) => {
                event::Kind::ExternalCommand(command_to_proto(command, 0))
            }
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
            api::ApiEvent::Resync => event::Kind::Resync(Empty {}),
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
            event::Kind::ExternalCommand(command) => {
                api::ApiEvent::PlayerExternalCommand(command_from_proto(command)?)
            }
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

    pub fn queue_context_to_proto(value: &api::QueueContext) -> QueueContext {
        let kind = match value {
            api::QueueContext::Tracks { keys } => {
                queue_context::Kind::Tracks(queue_context::TrackKeys { keys: keys.clone() })
            }
            api::QueueContext::Album { id } => {
                queue_context::Kind::Album(queue_context::Id { id: id.clone() })
            }
            api::QueueContext::Artist { name } => {
                queue_context::Kind::Artist(queue_context::Name { name: name.clone() })
            }
            api::QueueContext::Genre { name } => {
                queue_context::Kind::Genre(queue_context::Name { name: name.clone() })
            }
            api::QueueContext::Playlist { id } => {
                queue_context::Kind::Playlist(queue_context::Id { id: id.clone() })
            }
            api::QueueContext::Filter { filter } => {
                queue_context::Kind::Filter(track_filter_to_proto(filter))
            }
            api::QueueContext::Radio {
                station_id,
                stream_id,
            } => queue_context::Kind::Radio(queue_context::Radio {
                station_id: station_id.clone(),
                stream_id: stream_id.clone(),
            }),
            api::QueueContext::InlineTracks { tracks } => {
                queue_context::Kind::InlineTracks(queue_context::InlineTracks {
                    tracks: tracks.iter().map(track_info_to_proto).collect(),
                })
            }
        };
        QueueContext { kind: Some(kind) }
    }

    pub fn queue_context_from_proto(value: &QueueContext) -> Option<api::QueueContext> {
        Some(match value.kind.as_ref()? {
            queue_context::Kind::Tracks(tracks) => api::QueueContext::Tracks {
                keys: tracks.keys.clone(),
            },
            queue_context::Kind::Album(id) => api::QueueContext::Album { id: id.id.clone() },
            queue_context::Kind::Artist(name) => api::QueueContext::Artist {
                name: name.name.clone(),
            },
            queue_context::Kind::Genre(name) => api::QueueContext::Genre {
                name: name.name.clone(),
            },
            queue_context::Kind::Playlist(id) => api::QueueContext::Playlist { id: id.id.clone() },
            queue_context::Kind::Filter(filter) => api::QueueContext::Filter {
                filter: track_filter_from_proto(filter),
            },
            queue_context::Kind::Radio(radio) => api::QueueContext::Radio {
                station_id: radio.station_id.clone(),
                stream_id: radio.stream_id.clone(),
            },
            queue_context::Kind::InlineTracks(tracks) => api::QueueContext::InlineTracks {
                tracks: tracks.tracks.iter().map(track_info_from_proto).collect(),
            },
        })
    }

    pub fn set_queue_to_proto(value: &api::SetQueueRequest) -> SetQueueRequest {
        SetQueueRequest {
            mode: queue_mode_to_proto(value.mode) as i32,
            context: Some(queue_context_to_proto(&value.context)),
            start_index: value.start_index,
            shuffle: value.shuffle,
            insert_index: value.insert_index,
        }
    }

    pub fn set_queue_from_proto(value: &SetQueueRequest) -> Option<api::SetQueueRequest> {
        Some(api::SetQueueRequest {
            mode: queue_mode_from_proto(value.mode),
            context: queue_context_from_proto(value.context.as_ref()?)?,
            start_index: value.start_index,
            shuffle: value.shuffle,
            insert_index: value.insert_index,
        })
    }

    pub fn queue_edit_to_proto(value: &api::QueueEdit) -> QueueEditRequest {
        let op = match value {
            api::QueueEdit::Jump { index } => {
                queue_edit_request::Op::Jump(queue_edit_request::Jump { index: *index })
            }
            api::QueueEdit::Move { from, to } => {
                queue_edit_request::Op::Move(queue_edit_request::Move {
                    from: *from,
                    to: *to,
                })
            }
            api::QueueEdit::Remove { index } => {
                queue_edit_request::Op::Remove(queue_edit_request::Remove { index: *index })
            }
        };
        QueueEditRequest { op: Some(op) }
    }

    pub fn queue_edit_from_proto(value: &QueueEditRequest) -> Option<api::QueueEdit> {
        Some(match value.op.as_ref()? {
            queue_edit_request::Op::Jump(jump) => api::QueueEdit::Jump { index: jump.index },
            queue_edit_request::Op::Move(mv) => api::QueueEdit::Move {
                from: mv.from,
                to: mv.to,
            },
            queue_edit_request::Op::Remove(remove) => api::QueueEdit::Remove {
                index: remove.index,
            },
        })
    }

    pub fn track_filter_to_proto(value: &api::TrackFilter) -> TrackFilter {
        TrackFilter {
            search: value.search.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            genre: value.genre.clone(),
            favorite: value.favorite,
            sort: value.sort.clone(),
        }
    }

    pub fn track_filter_from_proto(value: &TrackFilter) -> api::TrackFilter {
        api::TrackFilter {
            search: value.search.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            genre: value.genre.clone(),
            favorite: value.favorite,
            sort: value.sort.clone(),
        }
    }

    pub fn page_to_proto(value: api::Page) -> Page {
        Page {
            offset: value.offset,
            limit: value.limit,
        }
    }

    pub fn page_from_proto(value: Option<&Page>) -> api::Page {
        let value = value.cloned().unwrap_or_default();
        api::Page {
            offset: value.offset,
            limit: if value.limit == 0 {
                api::DEFAULT_PAGE_LIMIT
            } else {
                value.limit
            },
        }
    }

    pub fn track_info_to_proto(value: &api::TrackInfo) -> TrackInfo {
        TrackInfo {
            key: value.key.clone(),
            uid: value.uid.clone(),
            title: value.title.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            album_id: value.album_id.clone(),
            duration_ms: value.duration_ms,
            khz: value.khz,
            bitrate: u32::from(value.bitrate),
            track_number: value.track_number,
            disc_number: value.disc_number,
            kind: track_kind_to_proto(value.kind) as i32,
            seekable: value.seekable,
            artwork: value.artwork.clone(),
            offline: value.offline,
            service: value
                .service
                .map(|service| music_service_to_proto(service) as i32),
            artists: value.artists.clone(),
            musicbrainz_release_id: value.musicbrainz_release_id.clone(),
            musicbrainz_recording_id: value.musicbrainz_recording_id.clone(),
            musicbrainz_track_id: value.musicbrainz_track_id.clone(),
            playlist_item_id: value.playlist_item_id.clone(),
            source: value.source.clone(),
        }
    }

    pub fn track_info_from_proto(value: &TrackInfo) -> api::TrackInfo {
        api::TrackInfo {
            key: value.key.clone(),
            uid: value.uid.clone(),
            title: value.title.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            album_id: value.album_id.clone(),
            duration_ms: value.duration_ms,
            khz: value.khz,
            bitrate: value.bitrate.min(u32::from(u16::MAX)) as u16,
            track_number: value.track_number,
            disc_number: value.disc_number,
            kind: track_kind_from_proto(value.kind),
            seekable: value.seekable,
            artwork: value.artwork.clone(),
            offline: value.offline,
            service: value.service.map(music_service_from_proto),
            artists: value.artists.clone(),
            musicbrainz_release_id: value.musicbrainz_release_id.clone(),
            musicbrainz_recording_id: value.musicbrainz_recording_id.clone(),
            musicbrainz_track_id: value.musicbrainz_track_id.clone(),
            playlist_item_id: value.playlist_item_id.clone(),
            source: value.source.clone(),
        }
    }

    pub fn external_playback_to_proto(value: &api::ExternalPlayback) -> ExternalPlayback {
        ExternalPlayback {
            kind: value.kind.clone(),
            device: value.device.clone(),
        }
    }

    pub fn external_playback_from_proto(value: &ExternalPlayback) -> api::ExternalPlayback {
        api::ExternalPlayback {
            kind: value.kind.clone(),
            device: value.device.clone(),
        }
    }

    pub fn external_lease_to_proto(value: &api::ExternalPlaybackLease) -> ExternalPlaybackLease {
        ExternalPlaybackLease {
            lease_id: value.lease_id.clone(),
            expires_in_ms: value.expires_in_ms,
        }
    }

    pub fn external_lease_from_proto(value: &ExternalPlaybackLease) -> api::ExternalPlaybackLease {
        api::ExternalPlaybackLease {
            lease_id: value.lease_id.clone(),
            expires_in_ms: value.expires_in_ms,
        }
    }

    pub fn external_report_to_proto(value: &api::ExternalPlaybackReport) -> ExternalPlaybackReport {
        ExternalPlaybackReport {
            lease_id: value.lease_id.clone(),
            track: value.track.as_ref().map(track_info_to_proto),
            position_ms: value.position_ms,
            playing: value.playing,
            completed: value.completed,
            device: value.device.clone(),
        }
    }

    pub fn external_report_from_proto(
        value: &ExternalPlaybackReport,
    ) -> api::ExternalPlaybackReport {
        api::ExternalPlaybackReport {
            lease_id: value.lease_id.clone(),
            track: value.track.as_ref().map(track_info_from_proto),
            position_ms: value.position_ms,
            playing: value.playing,
            completed: value.completed,
            device: value.device.clone(),
        }
    }

    pub fn track_page_to_proto(value: &api::TrackPage) -> TrackPage {
        TrackPage {
            total: value.total,
            offset: value.offset,
            items: value.items.iter().map(track_info_to_proto).collect(),
        }
    }

    pub fn track_page_from_proto(value: &TrackPage) -> api::TrackPage {
        api::TrackPage {
            total: value.total,
            offset: value.offset,
            items: value.items.iter().map(track_info_from_proto).collect(),
        }
    }

    pub fn queue_window_to_proto(value: &api::QueueWindow) -> QueueWindow {
        QueueWindow {
            rev: value.rev,
            total: value.total,
            offset: value.offset,
            items: value
                .items
                .iter()
                .map(|item| QueueItem {
                    index: item.index,
                    track: Some(track_info_to_proto(&item.track)),
                })
                .collect(),
        }
    }

    pub fn queue_window_from_proto(value: &QueueWindow) -> api::QueueWindow {
        api::QueueWindow {
            rev: value.rev,
            total: value.total,
            offset: value.offset,
            items: value
                .items
                .iter()
                .map(|item| api::QueueItem {
                    index: item.index,
                    track: item
                        .track
                        .as_ref()
                        .map(track_info_from_proto)
                        .unwrap_or_default(),
                })
                .collect(),
        }
    }

    pub fn queue_persistence_snapshot_to_proto(
        value: &api::QueuePersistenceSnapshot,
    ) -> QueuePersistenceSnapshot {
        QueuePersistenceSnapshot {
            tracks: value.tracks.iter().map(track_info_to_proto).collect(),
            current_index: value.current_index,
            progress_ms: value.progress_ms,
            shuffle_order: value.shuffle_order.clone(),
            shuffle_enabled: value.shuffle_enabled,
        }
    }

    pub fn queue_persistence_snapshot_from_proto(
        value: &QueuePersistenceSnapshot,
    ) -> api::QueuePersistenceSnapshot {
        api::QueuePersistenceSnapshot {
            tracks: value.tracks.iter().map(track_info_from_proto).collect(),
            current_index: value.current_index,
            progress_ms: value.progress_ms,
            shuffle_order: value.shuffle_order.clone(),
            shuffle_enabled: value.shuffle_enabled,
        }
    }

    pub fn lyrics_to_proto(value: &api::LyricsView) -> Lyrics {
        Lyrics {
            plain: value.plain.clone(),
            synced: value
                .synced
                .iter()
                .map(|line| LyricLine {
                    start_ms: line.start_ms,
                    end_ms: line.end_ms,
                    text: line.text.clone(),
                    chunks: line
                        .chunks
                        .iter()
                        .map(|chunk| LyricChunk {
                            start_ms: chunk.start_ms,
                            text: chunk.text.clone(),
                        })
                        .collect(),
                    parent_line_index: line.parent_line_index,
                    background: line.background,
                    opposite_turn: line.opposite_turn,
                })
                .collect(),
        }
    }

    pub fn lyrics_from_proto(value: &Lyrics) -> api::LyricsView {
        api::LyricsView {
            plain: value.plain.clone(),
            synced: value
                .synced
                .iter()
                .map(|line| api::LyricLineView {
                    start_ms: line.start_ms,
                    end_ms: line.end_ms,
                    text: line.text.clone(),
                    chunks: line
                        .chunks
                        .iter()
                        .map(|chunk| api::LyricChunkView {
                            start_ms: chunk.start_ms,
                            text: chunk.text.clone(),
                        })
                        .collect(),
                    parent_line_index: line.parent_line_index,
                    background: line.background,
                    opposite_turn: line.opposite_turn,
                })
                .collect(),
        }
    }

    pub fn stats_to_proto(value: &api::StatsView) -> Stats {
        Stats {
            listen_counts: value.listen_counts.clone().into_iter().collect(),
        }
    }

    pub fn stats_from_proto(value: &Stats) -> api::StatsView {
        api::StatsView {
            listen_counts: value.listen_counts.clone().into_iter().collect(),
        }
    }

    pub fn favorites_to_proto(value: &api::FavoritesView) -> Favorites {
        Favorites {
            refs: value.refs.clone(),
            generation: value.generation,
        }
    }

    pub fn favorites_from_proto(value: &Favorites) -> api::FavoritesView {
        api::FavoritesView {
            refs: value.refs.clone(),
            generation: value.generation,
        }
    }

    pub fn job_status_to_proto(value: &api::JobStatus) -> JobStatus {
        JobStatus {
            id: value.id.clone(),
            kind: job_kind_to_proto(value.kind) as i32,
            state: job_state_to_proto(value.state) as i32,
            phase: value.phase.clone(),
            current: value.current,
            total: value.total,
            message: value.message.clone(),
            error: value.error.as_ref().map(error_body_to_proto),
            request: value.request.clone(),
            title: value.title.clone(),
            format: value.format.clone(),
            speed: value.speed.clone(),
            eta: value.eta.clone(),
        }
    }

    pub fn job_status_from_proto(value: &JobStatus) -> api::JobStatus {
        api::JobStatus {
            id: value.id.clone(),
            kind: job_kind_from_proto(value.kind),
            state: job_state_from_proto(value.state),
            phase: value.phase.clone(),
            current: value.current,
            total: value.total,
            message: value.message.clone(),
            error: value.error.as_ref().map(error_body_from_proto),
            request: value.request.clone(),
            title: value.title.clone(),
            format: value.format.clone(),
            speed: value.speed.clone(),
            eta: value.eta.clone(),
        }
    }

    pub fn download_item_state_to_proto(value: api::DownloadItemState) -> DownloadItemState {
        match value {
            api::DownloadItemState::Queued => DownloadItemState::Queued,
            api::DownloadItemState::Downloading => DownloadItemState::Downloading,
            api::DownloadItemState::Finished => DownloadItemState::Finished,
            api::DownloadItemState::Failed => DownloadItemState::Failed,
            api::DownloadItemState::Cancelled => DownloadItemState::Cancelled,
            api::DownloadItemState::Unknown => DownloadItemState::Unspecified,
        }
    }

    pub fn download_item_state_from_proto(value: i32) -> api::DownloadItemState {
        match DownloadItemState::try_from(value).unwrap_or(DownloadItemState::Unspecified) {
            DownloadItemState::Queued => api::DownloadItemState::Queued,
            DownloadItemState::Downloading => api::DownloadItemState::Downloading,
            DownloadItemState::Finished => api::DownloadItemState::Finished,
            DownloadItemState::Failed => api::DownloadItemState::Failed,
            DownloadItemState::Cancelled => api::DownloadItemState::Cancelled,
            DownloadItemState::Unspecified => api::DownloadItemState::Unknown,
        }
    }

    pub fn download_status_to_proto(value: &api::DownloadItemStatus) -> DownloadItemStatus {
        DownloadItemStatus {
            key: value.key.clone(),
            state: download_item_state_to_proto(value.state) as i32,
            bytes_done: value.bytes_done,
            total_bytes: value.total_bytes,
            error: value.error.clone(),
        }
    }

    pub fn download_status_from_proto(value: &DownloadItemStatus) -> api::DownloadItemStatus {
        api::DownloadItemStatus {
            key: value.key.clone(),
            state: download_item_state_from_proto(value.state),
            bytes_done: value.bytes_done,
            total_bytes: value.total_bytes,
            error: value.error.clone(),
        }
    }

    pub fn music_service_to_proto(value: api::MusicService) -> MusicService {
        match value {
            api::MusicService::Jellyfin => MusicService::Jellyfin,
            api::MusicService::Subsonic => MusicService::Subsonic,
            api::MusicService::Custom => MusicService::Custom,
            api::MusicService::YtMusic => MusicService::YtMusic,
            api::MusicService::AppleMusic => MusicService::AppleMusic,
            api::MusicService::SoundCloud => MusicService::SoundCloud,
            api::MusicService::Spotify => MusicService::Spotify,
            api::MusicService::Nextcloud => MusicService::Nextcloud,
            api::MusicService::Unknown => MusicService::Unspecified,
        }
    }

    pub fn music_service_from_proto(value: i32) -> api::MusicService {
        match MusicService::try_from(value).unwrap_or(MusicService::Unspecified) {
            MusicService::Jellyfin => api::MusicService::Jellyfin,
            MusicService::Subsonic => api::MusicService::Subsonic,
            MusicService::Custom => api::MusicService::Custom,
            MusicService::YtMusic => api::MusicService::YtMusic,
            MusicService::AppleMusic => api::MusicService::AppleMusic,
            MusicService::SoundCloud => api::MusicService::SoundCloud,
            MusicService::Spotify => api::MusicService::Spotify,
            MusicService::Nextcloud => api::MusicService::Nextcloud,
            MusicService::Unspecified => api::MusicService::Unknown,
        }
    }

    pub fn album_filter_to_proto(value: &api::AlbumFilter) -> AlbumFilter {
        AlbumFilter {
            search: value.search.clone(),
            artist: value.artist.clone(),
            genre: value.genre.clone(),
            sort: value.sort.clone(),
        }
    }

    pub fn album_filter_from_proto(value: &AlbumFilter) -> api::AlbumFilter {
        api::AlbumFilter {
            search: value.search.clone(),
            artist: value.artist.clone(),
            genre: value.genre.clone(),
            sort: value.sort.clone(),
        }
    }

    pub fn album_info_to_proto(value: &api::AlbumInfo) -> AlbumInfo {
        AlbumInfo {
            id: value.id.clone(),
            title: value.title.clone(),
            artist: value.artist.clone(),
            genre: value.genre.clone(),
            year: value.year,
            artwork: value.artwork.clone(),
            manual_artwork: value.manual_artwork,
        }
    }

    pub fn album_info_from_proto(value: &AlbumInfo) -> api::AlbumInfo {
        api::AlbumInfo {
            id: value.id.clone(),
            title: value.title.clone(),
            artist: value.artist.clone(),
            genre: value.genre.clone(),
            year: value.year,
            artwork: value.artwork.clone(),
            manual_artwork: value.manual_artwork,
        }
    }

    pub fn album_page_to_proto(value: &api::AlbumPage) -> AlbumPage {
        AlbumPage {
            total: value.total,
            offset: value.offset,
            items: value.items.iter().map(album_info_to_proto).collect(),
        }
    }

    pub fn album_page_from_proto(value: &AlbumPage) -> api::AlbumPage {
        api::AlbumPage {
            total: value.total,
            offset: value.offset,
            items: value.items.iter().map(album_info_from_proto).collect(),
        }
    }

    pub fn artist_info_to_proto(value: &api::ArtistInfo) -> ArtistInfo {
        ArtistInfo {
            name: value.name.clone(),
            track_count: value.track_count,
            album_count: value.album_count,
            artwork: value.artwork.clone(),
            manual_artwork: value.manual_artwork,
        }
    }

    pub fn artist_info_from_proto(value: &ArtistInfo) -> api::ArtistInfo {
        api::ArtistInfo {
            name: value.name.clone(),
            track_count: value.track_count,
            album_count: value.album_count,
            artwork: value.artwork.clone(),
            manual_artwork: value.manual_artwork,
        }
    }

    pub fn artist_page_to_proto(value: &api::ArtistPage) -> ArtistPage {
        ArtistPage {
            total: value.total,
            offset: value.offset,
            items: value.items.iter().map(artist_info_to_proto).collect(),
        }
    }

    pub fn artist_page_from_proto(value: &ArtistPage) -> api::ArtistPage {
        api::ArtistPage {
            total: value.total,
            offset: value.offset,
            items: value.items.iter().map(artist_info_from_proto).collect(),
        }
    }

    pub fn search_results_to_proto(value: &api::SearchResults) -> SearchResults {
        SearchResults {
            tracks: value.tracks.iter().map(track_info_to_proto).collect(),
            albums: value.albums.iter().map(album_info_to_proto).collect(),
        }
    }

    pub fn search_results_from_proto(value: &SearchResults) -> api::SearchResults {
        api::SearchResults {
            tracks: value.tracks.iter().map(track_info_from_proto).collect(),
            albums: value.albums.iter().map(album_info_from_proto).collect(),
        }
    }

    pub fn playlist_info_to_proto(value: &api::PlaylistInfo) -> PlaylistInfo {
        PlaylistInfo {
            id: value.id.clone(),
            name: value.name.clone(),
            track_count: value.track_count,
            artwork: value.artwork.clone(),
            track_keys: value.track_keys.clone(),
            manual_artwork: value.manual_artwork,
        }
    }

    pub fn playlist_info_from_proto(value: &PlaylistInfo) -> api::PlaylistInfo {
        api::PlaylistInfo {
            id: value.id.clone(),
            name: value.name.clone(),
            track_count: value.track_count,
            artwork: value.artwork.clone(),
            track_keys: value.track_keys.clone(),
            manual_artwork: value.manual_artwork,
        }
    }

    pub fn playlist_folder_to_proto(value: &api::PlaylistFolderInfo) -> PlaylistFolderInfo {
        PlaylistFolderInfo {
            id: value.id.clone(),
            name: value.name.clone(),
            playlist_ids: value.playlist_ids.clone(),
        }
    }

    pub fn playlist_folder_from_proto(value: &PlaylistFolderInfo) -> api::PlaylistFolderInfo {
        api::PlaylistFolderInfo {
            id: value.id.clone(),
            name: value.name.clone(),
            playlist_ids: value.playlist_ids.clone(),
        }
    }

    pub fn playlist_catalog_to_proto(value: &api::PlaylistCatalog) -> PlaylistCatalog {
        PlaylistCatalog {
            playlists: value.playlists.iter().map(playlist_info_to_proto).collect(),
            folders: value.folders.iter().map(playlist_folder_to_proto).collect(),
        }
    }

    pub fn playlist_catalog_from_proto(value: &PlaylistCatalog) -> api::PlaylistCatalog {
        api::PlaylistCatalog {
            playlists: value
                .playlists
                .iter()
                .map(playlist_info_from_proto)
                .collect(),
            folders: value
                .folders
                .iter()
                .map(playlist_folder_from_proto)
                .collect(),
        }
    }

    pub fn source_kind_to_proto(value: api::SourceKind) -> SourceKind {
        match value {
            api::SourceKind::Local => SourceKind::Local,
            api::SourceKind::LocalLibrary => SourceKind::LocalLibrary,
            api::SourceKind::Server => SourceKind::Server,
            api::SourceKind::Unknown => SourceKind::Unspecified,
        }
    }

    pub fn source_kind_from_proto(value: i32) -> api::SourceKind {
        match SourceKind::try_from(value).unwrap_or(SourceKind::Unspecified) {
            SourceKind::Local => api::SourceKind::Local,
            SourceKind::LocalLibrary => api::SourceKind::LocalLibrary,
            SourceKind::Server => api::SourceKind::Server,
            SourceKind::Unspecified => api::SourceKind::Unknown,
        }
    }

    pub fn playlist_capability_to_proto(value: api::PlaylistCapability) -> PlaylistCapability {
        match value {
            api::PlaylistCapability::None => PlaylistCapability::None,
            api::PlaylistCapability::AddRemove => PlaylistCapability::AddRemove,
            api::PlaylistCapability::Reorder => PlaylistCapability::Reorder,
            api::PlaylistCapability::Unknown => PlaylistCapability::Unspecified,
        }
    }

    pub fn playlist_capability_from_proto(value: i32) -> api::PlaylistCapability {
        match PlaylistCapability::try_from(value).unwrap_or(PlaylistCapability::Unspecified) {
            PlaylistCapability::AddRemove => api::PlaylistCapability::AddRemove,
            PlaylistCapability::Reorder => api::PlaylistCapability::Reorder,
            PlaylistCapability::None | PlaylistCapability::Unspecified => {
                api::PlaylistCapability::None
            }
        }
    }

    pub fn artist_presentation_to_proto(value: api::ArtistPresentation) -> ArtistPresentation {
        match value {
            api::ArtistPresentation::Library => ArtistPresentation::Library,
            api::ArtistPresentation::Remote => ArtistPresentation::Remote,
            api::ArtistPresentation::Unknown => ArtistPresentation::Unspecified,
        }
    }

    pub fn artist_presentation_from_proto(value: i32) -> api::ArtistPresentation {
        match ArtistPresentation::try_from(value).unwrap_or(ArtistPresentation::Unspecified) {
            ArtistPresentation::Remote => api::ArtistPresentation::Remote,
            ArtistPresentation::Library | ArtistPresentation::Unspecified => {
                api::ArtistPresentation::Library
            }
        }
    }

    pub fn album_presentation_to_proto(value: api::AlbumPresentation) -> AlbumPresentation {
        match value {
            api::AlbumPresentation::Standard => AlbumPresentation::Standard,
            api::AlbumPresentation::Remote => AlbumPresentation::Remote,
            api::AlbumPresentation::Unknown => AlbumPresentation::Unspecified,
        }
    }

    pub fn album_presentation_from_proto(value: i32) -> api::AlbumPresentation {
        match AlbumPresentation::try_from(value).unwrap_or(AlbumPresentation::Unspecified) {
            AlbumPresentation::Remote => api::AlbumPresentation::Remote,
            AlbumPresentation::Standard | AlbumPresentation::Unspecified => {
                api::AlbumPresentation::Standard
            }
        }
    }

    pub fn favorites_sync_to_proto(value: api::FavoritesSyncMode) -> FavoritesSyncMode {
        match value {
            api::FavoritesSyncMode::Instant => FavoritesSyncMode::Instant,
            api::FavoritesSyncMode::Paginated => FavoritesSyncMode::Paginated,
            api::FavoritesSyncMode::Unknown => FavoritesSyncMode::Unspecified,
        }
    }

    pub fn favorites_sync_from_proto(value: i32) -> api::FavoritesSyncMode {
        match FavoritesSyncMode::try_from(value).unwrap_or(FavoritesSyncMode::Unspecified) {
            FavoritesSyncMode::Paginated => api::FavoritesSyncMode::Paginated,
            FavoritesSyncMode::Instant | FavoritesSyncMode::Unspecified => {
                api::FavoritesSyncMode::Instant
            }
        }
    }

    pub fn source_capabilities_to_proto(value: &api::SourceCapabilities) -> SourceCapabilities {
        SourceCapabilities {
            edit_tags: value.edit_tags,
            delete_from_disk: value.delete_from_disk,
            scan_folders: value.scan_folders,
            folders: value.folders,
            sync: value.sync,
            downloads: value.downloads,
            discover: value.discover,
            track_radio: value.track_radio,
            playlist_radio: value.playlist_radio,
            playlists: playlist_capability_to_proto(value.playlists) as i32,
            artists: artist_presentation_to_proto(value.artists) as i32,
            albums: album_presentation_to_proto(value.albums) as i32,
            favorites_sync: favorites_sync_to_proto(value.favorites_sync) as i32,
        }
    }

    pub fn source_capabilities_from_proto(
        value: Option<&SourceCapabilities>,
    ) -> api::SourceCapabilities {
        let value = value.cloned().unwrap_or_default();
        api::SourceCapabilities {
            edit_tags: value.edit_tags,
            delete_from_disk: value.delete_from_disk,
            scan_folders: value.scan_folders,
            folders: value.folders,
            sync: value.sync,
            downloads: value.downloads,
            discover: value.discover,
            track_radio: value.track_radio,
            playlist_radio: value.playlist_radio,
            playlists: playlist_capability_from_proto(value.playlists),
            artists: artist_presentation_from_proto(value.artists),
            albums: album_presentation_from_proto(value.albums),
            favorites_sync: favorites_sync_from_proto(value.favorites_sync),
        }
    }

    pub fn source_info_to_proto(value: &api::SourceInfo) -> SourceInfo {
        SourceInfo {
            id: value.id.clone(),
            name: value.name.clone(),
            kind: source_kind_to_proto(value.kind) as i32,
            service: value
                .service
                .map(|service| music_service_to_proto(service) as i32),
            active: value.active,
            authenticated: value.authenticated,
            capabilities: Some(source_capabilities_to_proto(&value.capabilities)),
            url: value.url.clone(),
            browser: value.browser.clone(),
            anonymous: value.anonymous,
            storefront: value.storefront.clone(),
            language: value.language.clone(),
            directories: value.directories.clone(),
        }
    }

    pub fn source_info_from_proto(value: &SourceInfo) -> api::SourceInfo {
        api::SourceInfo {
            id: value.id.clone(),
            name: value.name.clone(),
            kind: source_kind_from_proto(value.kind),
            service: value.service.map(music_service_from_proto),
            active: value.active,
            authenticated: value.authenticated,
            capabilities: source_capabilities_from_proto(value.capabilities.as_ref()),
            url: value.url.clone(),
            browser: value.browser.clone(),
            anonymous: value.anonymous,
            storefront: value.storefront.clone(),
            language: value.language.clone(),
            directories: value.directories.clone(),
        }
    }

    pub fn local_source_draft_to_proto(value: &api::LocalSourceDraft) -> LocalSourceDraft {
        LocalSourceDraft {
            id: value.id.clone(),
            name: value.name.clone(),
            directories: value.directories.clone(),
        }
    }

    pub fn local_source_draft_from_proto(value: &LocalSourceDraft) -> api::LocalSourceDraft {
        api::LocalSourceDraft {
            id: value.id.clone(),
            name: value.name.clone(),
            directories: value.directories.clone(),
        }
    }

    pub fn server_draft_to_proto(value: &api::ServerDraft) -> ServerDraft {
        ServerDraft {
            id: value.id.clone(),
            name: value.name.clone(),
            url: value.url.clone(),
            service: music_service_to_proto(value.service) as i32,
            browser: value.browser.clone(),
            anonymous: value.anonymous,
            storefront: value.storefront.clone(),
            language: value.language.clone(),
        }
    }

    pub fn server_draft_from_proto(value: &ServerDraft) -> api::ServerDraft {
        api::ServerDraft {
            id: value.id.clone(),
            name: value.name.clone(),
            url: value.url.clone(),
            service: music_service_from_proto(value.service),
            browser: value.browser.clone(),
            anonymous: value.anonymous,
            storefront: value.storefront.clone(),
            language: value.language.clone(),
        }
    }

    pub fn credential_to_proto(value: &api::CredentialProvision) -> CredentialProvision {
        CredentialProvision {
            server_id: value.server_id.clone(),
            secret: value.secret.clone(),
            user_id: value.user_id.clone(),
            browser: value.browser.clone(),
        }
    }

    pub fn credential_from_proto(value: &CredentialProvision) -> api::CredentialProvision {
        api::CredentialProvision {
            server_id: value.server_id.clone(),
            secret: value.secret.clone(),
            user_id: value.user_id.clone(),
            browser: value.browser.clone(),
        }
    }

    pub fn source_login_to_proto(value: &api::SourceLoginRequest) -> SourceLoginRequest {
        SourceLoginRequest {
            server_id: value.server_id.clone(),
            username: value.username.clone(),
            password: value.password.clone(),
        }
    }

    pub fn source_login_from_proto(value: &SourceLoginRequest) -> api::SourceLoginRequest {
        api::SourceLoginRequest {
            server_id: value.server_id.clone(),
            username: value.username.clone(),
            password: value.password.clone(),
        }
    }

    pub fn integration_kind_to_proto(value: api::IntegrationKind) -> IntegrationKind {
        match value {
            api::IntegrationKind::ListenBrainz => IntegrationKind::ListenBrainz,
            api::IntegrationKind::LastFm => IntegrationKind::LastFm,
            api::IntegrationKind::LibreFm => IntegrationKind::LibreFm,
            api::IntegrationKind::Unknown => IntegrationKind::Unspecified,
        }
    }

    pub fn integration_kind_from_proto(value: i32) -> api::IntegrationKind {
        match IntegrationKind::try_from(value).unwrap_or(IntegrationKind::Unspecified) {
            IntegrationKind::ListenBrainz => api::IntegrationKind::ListenBrainz,
            IntegrationKind::LastFm => api::IntegrationKind::LastFm,
            IntegrationKind::LibreFm => api::IntegrationKind::LibreFm,
            IntegrationKind::Unspecified => api::IntegrationKind::Unknown,
        }
    }

    pub fn integration_status_to_proto(
        value: &api::IntegrationCredentialStatus,
    ) -> IntegrationCredentialStatus {
        IntegrationCredentialStatus {
            kind: integration_kind_to_proto(value.kind) as i32,
            configured: value.configured,
        }
    }

    pub fn integration_status_from_proto(
        value: &IntegrationCredentialStatus,
    ) -> api::IntegrationCredentialStatus {
        api::IntegrationCredentialStatus {
            kind: integration_kind_from_proto(value.kind),
            configured: value.configured,
        }
    }

    pub fn integration_provision_to_proto(
        value: &api::IntegrationCredentialProvision,
    ) -> IntegrationCredentialProvision {
        IntegrationCredentialProvision {
            kind: integration_kind_to_proto(value.kind) as i32,
            token: value.token.clone(),
            api_key: value.api_key.clone(),
            api_secret: value.api_secret.clone(),
            session_key: value.session_key.clone(),
        }
    }

    pub fn integration_provision_from_proto(
        value: &IntegrationCredentialProvision,
    ) -> api::IntegrationCredentialProvision {
        api::IntegrationCredentialProvision {
            kind: integration_kind_from_proto(value.kind),
            token: value.token.clone(),
            api_key: value.api_key.clone(),
            api_secret: value.api_secret.clone(),
            session_key: value.session_key.clone(),
        }
    }

    pub fn source_folder_to_proto(value: &api::SourceFolderEntry) -> SourceFolderEntry {
        SourceFolderEntry {
            path: value.path.clone(),
            name: value.name.clone(),
        }
    }

    pub fn source_folder_from_proto(value: &SourceFolderEntry) -> api::SourceFolderEntry {
        api::SourceFolderEntry {
            path: value.path.clone(),
            name: value.name.clone(),
        }
    }

    pub fn ytdlp_format_to_proto(value: api::YtdlpAudioFormat) -> YtdlpAudioFormat {
        match value {
            api::YtdlpAudioFormat::Best => YtdlpAudioFormat::Best,
            api::YtdlpAudioFormat::Mp3 => YtdlpAudioFormat::Mp3,
            api::YtdlpAudioFormat::M4a => YtdlpAudioFormat::M4a,
            api::YtdlpAudioFormat::Opus => YtdlpAudioFormat::Opus,
            api::YtdlpAudioFormat::Flac => YtdlpAudioFormat::Flac,
            api::YtdlpAudioFormat::Wav => YtdlpAudioFormat::Wav,
            api::YtdlpAudioFormat::Video => YtdlpAudioFormat::Video,
            api::YtdlpAudioFormat::Unknown => YtdlpAudioFormat::Unspecified,
        }
    }

    pub fn ytdlp_format_from_proto(value: i32) -> api::YtdlpAudioFormat {
        match YtdlpAudioFormat::try_from(value).unwrap_or(YtdlpAudioFormat::Unspecified) {
            YtdlpAudioFormat::Mp3 => api::YtdlpAudioFormat::Mp3,
            YtdlpAudioFormat::M4a => api::YtdlpAudioFormat::M4a,
            YtdlpAudioFormat::Opus => api::YtdlpAudioFormat::Opus,
            YtdlpAudioFormat::Flac => api::YtdlpAudioFormat::Flac,
            YtdlpAudioFormat::Wav => api::YtdlpAudioFormat::Wav,
            YtdlpAudioFormat::Video => api::YtdlpAudioFormat::Video,
            YtdlpAudioFormat::Best | YtdlpAudioFormat::Unspecified => api::YtdlpAudioFormat::Best,
        }
    }

    pub fn ytdlp_request_to_proto(value: &api::YtdlpRequest) -> YtdlpRequest {
        YtdlpRequest {
            url: value.url.clone(),
            output_dir: value.output_dir.clone(),
            format: ytdlp_format_to_proto(value.format) as i32,
            options_json: value.options.to_string(),
        }
    }

    pub fn ytdlp_request_from_proto(
        value: &YtdlpRequest,
    ) -> Result<api::YtdlpRequest, api::ApiError> {
        Ok(api::YtdlpRequest {
            url: value.url.clone(),
            output_dir: value.output_dir.clone(),
            format: ytdlp_format_from_proto(value.format),
            options: serde_json::from_str(&value.options_json).map_err(|error| {
                api::ApiError::invalid_input(format!("invalid yt-dlp options JSON: {error}"))
            })?,
        })
    }

    pub fn external_access_to_proto(value: &api::ExternalAccess) -> ExternalAccess {
        ExternalAccess {
            kind: value.kind.clone(),
            access_token: value.access_token.clone(),
            client_id: value.client_id.clone(),
        }
    }

    pub fn external_access_from_proto(value: &ExternalAccess) -> api::ExternalAccess {
        api::ExternalAccess {
            kind: value.kind.clone(),
            access_token: value.access_token.clone(),
            client_id: value.client_id.clone(),
        }
    }

    pub fn catalog_item_kind_to_proto(value: api::CatalogItemKind) -> CatalogItemKind {
        match value {
            api::CatalogItemKind::Track => CatalogItemKind::Track,
            api::CatalogItemKind::Album => CatalogItemKind::Album,
            api::CatalogItemKind::Playlist => CatalogItemKind::Playlist,
            api::CatalogItemKind::Artist => CatalogItemKind::Artist,
            api::CatalogItemKind::Mood => CatalogItemKind::Mood,
            api::CatalogItemKind::Unknown => CatalogItemKind::Unspecified,
        }
    }

    pub fn catalog_item_kind_from_proto(value: i32) -> api::CatalogItemKind {
        match CatalogItemKind::try_from(value).unwrap_or(CatalogItemKind::Unspecified) {
            CatalogItemKind::Track => api::CatalogItemKind::Track,
            CatalogItemKind::Album => api::CatalogItemKind::Album,
            CatalogItemKind::Playlist => api::CatalogItemKind::Playlist,
            CatalogItemKind::Artist => api::CatalogItemKind::Artist,
            CatalogItemKind::Mood => api::CatalogItemKind::Mood,
            CatalogItemKind::Unspecified => api::CatalogItemKind::Unknown,
        }
    }

    pub fn catalog_item_to_proto(value: &api::CatalogItem) -> CatalogItem {
        CatalogItem {
            kind: catalog_item_kind_to_proto(value.kind) as i32,
            id: value.id.clone(),
            title: value.title.clone(),
            subtitle: value.subtitle.clone(),
            artwork: value.artwork.clone(),
            track: value.track.as_ref().map(track_info_to_proto),
        }
    }

    pub fn catalog_item_from_proto(value: &CatalogItem) -> api::CatalogItem {
        api::CatalogItem {
            kind: catalog_item_kind_from_proto(value.kind),
            id: value.id.clone(),
            title: value.title.clone(),
            subtitle: value.subtitle.clone(),
            artwork: value.artwork.clone(),
            track: value.track.as_ref().map(track_info_from_proto),
        }
    }

    pub fn catalog_page_to_proto(value: &api::CatalogPage) -> CatalogPage {
        CatalogPage {
            shelves: value
                .shelves
                .iter()
                .map(|shelf| CatalogShelf {
                    title: shelf.title.clone(),
                    strapline: shelf.strapline.clone(),
                    items: shelf.items.iter().map(catalog_item_to_proto).collect(),
                    more_ref: shelf.more_ref.clone(),
                    list: shelf.list,
                })
                .collect(),
            continuation: value.continuation.clone(),
        }
    }

    pub fn catalog_page_from_proto(value: &CatalogPage) -> api::CatalogPage {
        api::CatalogPage {
            shelves: value
                .shelves
                .iter()
                .map(|shelf| api::CatalogShelf {
                    title: shelf.title.clone(),
                    strapline: shelf.strapline.clone(),
                    items: shelf.items.iter().map(catalog_item_from_proto).collect(),
                    more_ref: shelf.more_ref.clone(),
                    list: shelf.list,
                })
                .collect(),
            continuation: value.continuation.clone(),
        }
    }

    pub fn catalog_detail_request_to_proto(
        value: &api::CatalogDetailRequest,
    ) -> CatalogDetailRequest {
        CatalogDetailRequest {
            kind: catalog_item_kind_to_proto(value.kind) as i32,
            id: value.id.clone(),
            continuation: value.continuation.clone(),
        }
    }

    pub fn catalog_detail_request_from_proto(
        value: &CatalogDetailRequest,
    ) -> api::CatalogDetailRequest {
        api::CatalogDetailRequest {
            kind: catalog_item_kind_from_proto(value.kind),
            id: value.id.clone(),
            continuation: value.continuation.clone(),
        }
    }

    pub fn catalog_detail_to_proto(value: &api::CatalogDetail) -> CatalogDetail {
        CatalogDetail {
            kind: catalog_item_kind_to_proto(value.kind) as i32,
            id: value.id.clone(),
            title: value.title.clone(),
            subtitle: value.subtitle.clone(),
            description: value.description.clone(),
            artwork: value.artwork.clone(),
            playback_id: value.playback_id.clone(),
            year: value.year.clone(),
            tracks: value.tracks.iter().map(track_info_to_proto).collect(),
            shelves: value
                .shelves
                .iter()
                .map(|shelf| CatalogShelf {
                    title: shelf.title.clone(),
                    strapline: shelf.strapline.clone(),
                    items: shelf.items.iter().map(catalog_item_to_proto).collect(),
                    more_ref: shelf.more_ref.clone(),
                    list: shelf.list,
                })
                .collect(),
            continuation: value.continuation.clone(),
        }
    }

    pub fn catalog_detail_from_proto(value: &CatalogDetail) -> api::CatalogDetail {
        api::CatalogDetail {
            kind: catalog_item_kind_from_proto(value.kind),
            id: value.id.clone(),
            title: value.title.clone(),
            subtitle: value.subtitle.clone(),
            description: value.description.clone(),
            artwork: value.artwork.clone(),
            playback_id: value.playback_id.clone(),
            year: value.year.clone(),
            tracks: value.tracks.iter().map(track_info_from_proto).collect(),
            shelves: value
                .shelves
                .iter()
                .map(|shelf| api::CatalogShelf {
                    title: shelf.title.clone(),
                    strapline: shelf.strapline.clone(),
                    items: shelf.items.iter().map(catalog_item_from_proto).collect(),
                    more_ref: shelf.more_ref.clone(),
                    list: shelf.list,
                })
                .collect(),
            continuation: value.continuation.clone(),
        }
    }

    pub fn radio_station_to_proto(value: &api::RadioStationInfo) -> RadioStationInfo {
        RadioStationInfo {
            id: value.id.clone(),
            name: value.name.clone(),
            description: value.description.clone(),
            tags: value.tags.clone(),
            streams: value
                .streams
                .iter()
                .map(|stream| RadioStreamInfo {
                    id: stream.id.clone(),
                    name: stream.name.clone(),
                    url: stream.url.clone(),
                    icon: stream.icon.clone(),
                })
                .collect(),
            pinned: value.pinned,
            icon: value.icon.clone(),
            artwork: value.artwork.clone(),
        }
    }

    pub fn radio_station_from_proto(value: &RadioStationInfo) -> api::RadioStationInfo {
        api::RadioStationInfo {
            id: value.id.clone(),
            name: value.name.clone(),
            description: value.description.clone(),
            icon: value.icon.clone(),
            artwork: value.artwork.clone(),
            tags: value.tags.clone(),
            streams: value
                .streams
                .iter()
                .map(|stream| api::RadioStreamInfo {
                    id: stream.id.clone(),
                    name: stream.name.clone(),
                    url: stream.url.clone(),
                    icon: stream.icon.clone(),
                })
                .collect(),
            pinned: value.pinned,
        }
    }

    pub fn radio_registry_to_proto(value: &api::RadioRegistryInfo) -> RadioRegistryInfo {
        RadioRegistryInfo {
            url: value.url.clone(),
            enabled: value.enabled,
            built_in: value.built_in,
        }
    }

    pub fn radio_registry_from_proto(value: &RadioRegistryInfo) -> api::RadioRegistryInfo {
        api::RadioRegistryInfo {
            url: value.url.clone(),
            enabled: value.enabled,
            built_in: value.built_in,
        }
    }

    pub fn metadata_patch_to_proto(value: &api::TrackMetadataPatch) -> TrackMetadataPatch {
        TrackMetadataPatch {
            key: value.key.clone(),
            title: value.title.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            track_number: value.track_number,
            clear_track_number: value.clear_track_number,
            disc_number: value.disc_number,
            clear_disc_number: value.clear_disc_number,
        }
    }

    pub fn metadata_patch_from_proto(value: &TrackMetadataPatch) -> api::TrackMetadataPatch {
        api::TrackMetadataPatch {
            key: value.key.clone(),
            title: value.title.clone(),
            artist: value.artist.clone(),
            album: value.album.clone(),
            track_number: value.track_number,
            clear_track_number: value.clear_track_number,
            disc_number: value.disc_number,
            clear_disc_number: value.clear_disc_number,
        }
    }

    pub fn artwork_upload_to_proto(value: &api::ArtworkUpload) -> ArtworkUpload {
        ArtworkUpload {
            target: value.target.as_ref().map(|target| match target {
                api::ArtworkTarget::Track { key } => artwork_upload::Target::TrackKey(key.clone()),
                api::ArtworkTarget::Album { id } => artwork_upload::Target::AlbumId(id.clone()),
                api::ArtworkTarget::Artist { name } => {
                    artwork_upload::Target::ArtistName(name.clone())
                }
                api::ArtworkTarget::Playlist { id } => {
                    artwork_upload::Target::PlaylistId(id.clone())
                }
            }),
            content_type: value.content_type.clone(),
            data: value.data.clone(),
        }
    }

    pub fn artwork_request_to_proto(value: &api::ArtworkRequest) -> ArtworkRequest {
        ArtworkRequest {
            entity: value.entity.as_ref().map(|entity| match entity {
                api::ArtworkEntity::Track { key } => artwork_request::Entity::Track(key.clone()),
                api::ArtworkEntity::Album { id } => artwork_request::Entity::Album(id.clone()),
                api::ArtworkEntity::Artist { name } => {
                    artwork_request::Entity::Artist(name.clone())
                }
                api::ArtworkEntity::Playlist { id } => {
                    artwork_request::Entity::Playlist(id.clone())
                }
            }),
            hq: value.hq,
        }
    }

    pub fn artwork_request_from_proto(value: &ArtworkRequest) -> api::ArtworkRequest {
        api::ArtworkRequest {
            entity: value.entity.as_ref().map(|entity| match entity {
                artwork_request::Entity::Track(key) => {
                    api::ArtworkEntity::Track { key: key.clone() }
                }
                artwork_request::Entity::Album(id) => api::ArtworkEntity::Album { id: id.clone() },
                artwork_request::Entity::Artist(name) => {
                    api::ArtworkEntity::Artist { name: name.clone() }
                }
                artwork_request::Entity::Playlist(id) => {
                    api::ArtworkEntity::Playlist { id: id.clone() }
                }
            }),
            hq: value.hq,
        }
    }

    pub fn artwork_upload_from_proto(value: &ArtworkUpload) -> api::ArtworkUpload {
        api::ArtworkUpload {
            target: value.target.as_ref().map(|target| match target {
                artwork_upload::Target::TrackKey(key) => {
                    api::ArtworkTarget::Track { key: key.clone() }
                }
                artwork_upload::Target::AlbumId(id) => api::ArtworkTarget::Album { id: id.clone() },
                artwork_upload::Target::ArtistName(name) => {
                    api::ArtworkTarget::Artist { name: name.clone() }
                }
                artwork_upload::Target::PlaylistId(id) => {
                    api::ArtworkTarget::Playlist { id: id.clone() }
                }
            }),
            content_type: value.content_type.clone(),
            data: value.data.clone(),
        }
    }

    pub fn remove_artwork_to_proto(value: &api::ArtworkTarget) -> RemoveArtworkRequest {
        let target = match value {
            api::ArtworkTarget::Track { key } => {
                remove_artwork_request::Target::TrackKey(key.clone())
            }
            api::ArtworkTarget::Album { id } => remove_artwork_request::Target::AlbumId(id.clone()),
            api::ArtworkTarget::Artist { name } => {
                remove_artwork_request::Target::ArtistName(name.clone())
            }
            api::ArtworkTarget::Playlist { id } => {
                remove_artwork_request::Target::PlaylistId(id.clone())
            }
        };
        RemoveArtworkRequest {
            target: Some(target),
        }
    }

    pub fn remove_artwork_from_proto(value: &RemoveArtworkRequest) -> Option<api::ArtworkTarget> {
        Some(match value.target.as_ref()? {
            remove_artwork_request::Target::TrackKey(key) => {
                api::ArtworkTarget::Track { key: key.clone() }
            }
            remove_artwork_request::Target::AlbumId(id) => {
                api::ArtworkTarget::Album { id: id.clone() }
            }
            remove_artwork_request::Target::ArtistName(name) => {
                api::ArtworkTarget::Artist { name: name.clone() }
            }
            remove_artwork_request::Target::PlaylistId(id) => {
                api::ArtworkTarget::Playlist { id: id.clone() }
            }
        })
    }

    pub fn config_view_to_proto(value: &api::ConfigView) -> ConfigView {
        ConfigView {
            config_json: value.config.to_string(),
            locked_keys: value.locked_keys.clone(),
        }
    }

    pub fn config_view_from_proto(value: &ConfigView) -> api::ConfigView {
        api::ConfigView {
            config: serde_json::from_str(&value.config_json).unwrap_or(serde_json::Value::Null),
            locked_keys: value.locked_keys.clone(),
        }
    }
}

/// ApiError <-> tonic Status, shared by the server and the Rust client so
/// the mapping cannot drift. The stable machine-readable code rides the
/// `kopuz-error-code` metadata entry; the gRPC code is the nearest
/// standard equivalent for clients that only know gRPC.
pub mod status {
    use tonic::{Code, Status};

    fn code_name(code: api::ErrorCode) -> &'static str {
        match code {
            api::ErrorCode::InvalidInput => "invalid_input",
            api::ErrorCode::Unauthorized => "unauthorized",
            api::ErrorCode::NotFound => "not_found",
            api::ErrorCode::Conflict => "conflict",
            api::ErrorCode::SourceAuthExpired => "source_auth_expired",
            api::ErrorCode::SourceUnreachable => "source_unreachable",
            api::ErrorCode::Unsupported => "unsupported",
            api::ErrorCode::Internal => "internal",
        }
    }

    fn code_from_name(name: &str) -> Option<api::ErrorCode> {
        Some(match name {
            "invalid_input" => api::ErrorCode::InvalidInput,
            "unauthorized" => api::ErrorCode::Unauthorized,
            "not_found" => api::ErrorCode::NotFound,
            "conflict" => api::ErrorCode::Conflict,
            "source_auth_expired" => api::ErrorCode::SourceAuthExpired,
            "source_unreachable" => api::ErrorCode::SourceUnreachable,
            "unsupported" => api::ErrorCode::Unsupported,
            "internal" => api::ErrorCode::Internal,
            _ => return None,
        })
    }

    pub fn to_status(error: api::ApiError) -> Status {
        let grpc_code = match error.code {
            api::ErrorCode::InvalidInput => Code::InvalidArgument,
            api::ErrorCode::Unauthorized | api::ErrorCode::SourceAuthExpired => {
                Code::Unauthenticated
            }
            api::ErrorCode::NotFound => Code::NotFound,
            api::ErrorCode::Conflict => Code::Aborted,
            api::ErrorCode::Unsupported => Code::Unimplemented,
            api::ErrorCode::SourceUnreachable => Code::Unavailable,
            api::ErrorCode::Internal => Code::Internal,
        };
        let mut status = Status::new(grpc_code, error.message);
        if let Ok(value) = code_name(error.code).parse() {
            status.metadata_mut().insert("kopuz-error-code", value);
        }
        if let Some(details) = error.details {
            let bytes = details.to_string().into_bytes();
            status.metadata_mut().insert_bin(
                "kopuz-error-details-bin",
                tonic::metadata::MetadataValue::from_bytes(&bytes),
            );
        }
        status
    }

    pub fn from_status(status: &Status) -> api::ApiError {
        let code = status
            .metadata()
            .get("kopuz-error-code")
            .and_then(|value| value.to_str().ok())
            .and_then(code_from_name)
            .unwrap_or(match status.code() {
                Code::InvalidArgument => api::ErrorCode::InvalidInput,
                Code::Unauthenticated => api::ErrorCode::Unauthorized,
                Code::NotFound => api::ErrorCode::NotFound,
                Code::Aborted | Code::AlreadyExists => api::ErrorCode::Conflict,
                Code::Unimplemented => api::ErrorCode::Unsupported,
                Code::Unavailable => api::ErrorCode::SourceUnreachable,
                _ => api::ErrorCode::Internal,
            });
        let details = status
            .metadata()
            .get_bin("kopuz-error-details-bin")
            .and_then(|value| value.to_bytes().ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        api::ApiError {
            code,
            message: status.message().to_string(),
            details,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_code_survives_the_status_round_trip() {
            let codes = [
                api::ErrorCode::InvalidInput,
                api::ErrorCode::Unauthorized,
                api::ErrorCode::NotFound,
                api::ErrorCode::Conflict,
                api::ErrorCode::SourceAuthExpired,
                api::ErrorCode::SourceUnreachable,
                api::ErrorCode::Unsupported,
                api::ErrorCode::Internal,
            ];
            for code in codes {
                let error = api::ApiError {
                    code,
                    message: "m".into(),
                    details: Some(serde_json::json!({"n": 1})),
                };
                let back = from_status(&to_status(error.clone()));
                assert_eq!(error, back);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::convert::*;
    use super::*;

    fn sample_state() -> api::PlayerState {
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
                title: "t".into(),
                artist: "a".into(),
                album: "al".into(),
                duration_ms: Some(223_000),
                khz: 44,
                bitrate: 320,
                artwork: Some("k".into()),
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
                details: Some(serde_json::json!({"k": 1})),
            }),
            output_latency_ms: Some(120),
        }
    }

    #[test]
    fn player_state_round_trips() {
        let state = sample_state();
        let back = player_state_from_proto(&player_state_to_proto(&state));
        assert_eq!(state, back);

        let intents = [
            api::Intent::Stopped,
            api::Intent::Loading {
                token: 2,
                from_token: None,
            },
            api::Intent::Loading {
                token: 3,
                from_token: Some(2),
            },
            api::Intent::Committed { token: 4 },
        ];
        for intent in intents {
            assert_eq!(intent, intent_from_proto(Some(&intent_to_proto(&intent))));
        }
    }

    #[test]
    fn unspecified_status_values_are_unknown() {
        assert_eq!(job_state_from_proto(0), api::JobState::Unknown);
        assert_eq!(notice_level_from_proto(0), api::NoticeLevel::Unknown);
    }

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
            api::ApiEvent::PlayerExternalCommand(api::PlayerCommand::Seek { position_ms: 9 }),
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
                    details: None,
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
    fn commands_and_requests_round_trip() {
        let commands = vec![
            api::PlayerCommand::Play,
            api::PlayerCommand::Pause,
            api::PlayerCommand::Toggle,
            api::PlayerCommand::Next,
            api::PlayerCommand::Previous,
            api::PlayerCommand::Stop,
            api::PlayerCommand::Seek { position_ms: 5 },
            api::PlayerCommand::SetVolume { volume: 0.5 },
            api::PlayerCommand::SetMode {
                shuffle: Some(true),
                loop_mode: Some(api::LoopMode::Track),
            },
            api::PlayerCommand::SetMode {
                shuffle: None,
                loop_mode: None,
            },
        ];
        for command in commands {
            let back =
                command_from_proto(&command_to_proto(&command, 1)).expect("command survives");
            assert_eq!(command, back);
        }

        let contexts = vec![
            api::QueueContext::Tracks {
                keys: vec!["a".into(), "b".into()],
            },
            api::QueueContext::Album { id: "al".into() },
            api::QueueContext::Artist { name: "ar".into() },
            api::QueueContext::Genre { name: "g".into() },
            api::QueueContext::Playlist { id: "p".into() },
            api::QueueContext::Filter {
                filter: api::TrackFilter {
                    search: Some("s".into()),
                    artist: Some("a".into()),
                    album: Some("al".into()),
                    genre: Some("g".into()),
                    favorite: Some(true),
                    sort: Some("title".into()),
                },
            },
            api::QueueContext::Radio {
                station_id: "s".into(),
                stream_id: "st".into(),
            },
            api::QueueContext::InlineTracks {
                tracks: vec![api::TrackInfo {
                    key: "remote".into(),
                    service: Some(api::MusicService::YtMusic),
                    ..Default::default()
                }],
            },
        ];
        for (index, context) in contexts.into_iter().enumerate() {
            let request = api::SetQueueRequest {
                mode: match index % 3 {
                    0 => api::QueueMode::Replace,
                    1 => api::QueueMode::Append,
                    _ => api::QueueMode::PlayNext,
                },
                context,
                start_index: (index == 0).then_some(2),
                shuffle: (index == 0).then_some(false),
                insert_index: None,
            };
            let back =
                set_queue_from_proto(&set_queue_to_proto(&request)).expect("request survives");
            assert_eq!(request, back);
        }
        let insert = api::SetQueueRequest {
            mode: api::QueueMode::Insert,
            context: api::QueueContext::Tracks {
                keys: vec!["inserted".into()],
            },
            start_index: None,
            shuffle: None,
            insert_index: Some(3),
        };
        assert_eq!(
            insert,
            set_queue_from_proto(&set_queue_to_proto(&insert)).expect("insert survives")
        );

        let edits = [
            api::QueueEdit::Jump { index: 2 },
            api::QueueEdit::Move { from: 1, to: 3 },
            api::QueueEdit::Remove { index: 4 },
        ];
        for edit in edits {
            let back = queue_edit_from_proto(&queue_edit_to_proto(&edit)).expect("edit survives");
            assert_eq!(edit, back);
        }
    }

    #[test]
    fn resource_dtos_round_trip_with_optional_fields() {
        let track = api::TrackInfo {
            key: "key".into(),
            uid: "uid".into(),
            title: "title".into(),
            artist: "artist".into(),
            album: "album".into(),
            album_id: "album-id".into(),
            duration_ms: None,
            khz: 48,
            bitrate: 320,
            track_number: Some(2),
            disc_number: None,
            kind: api::TrackKind::Radio,
            seekable: false,
            artwork: Some("key".into()),
            offline: true,
            service: Some(api::MusicService::YtMusic),
            artists: vec!["artist".into(), "guest".into()],
            musicbrainz_release_id: Some("release".into()),
            musicbrainz_recording_id: None,
            musicbrainz_track_id: Some("track-id".into()),
            playlist_item_id: Some("entry".into()),
            source: "server-1".into(),
        };
        assert_eq!(track, track_info_from_proto(&track_info_to_proto(&track)));

        let page = api::TrackPage {
            total: 1,
            offset: 7,
            items: vec![track.clone()],
        };
        assert_eq!(page, track_page_from_proto(&track_page_to_proto(&page)));

        let window = api::QueueWindow {
            rev: 9,
            total: 1,
            offset: 0,
            items: vec![api::QueueItem {
                index: 4,
                track: track.clone(),
            }],
        };
        assert_eq!(
            window,
            queue_window_from_proto(&queue_window_to_proto(&window))
        );

        let snapshot = api::QueuePersistenceSnapshot {
            tracks: vec![track],
            current_index: 0,
            progress_ms: 12_345,
            shuffle_order: vec![0],
            shuffle_enabled: true,
        };
        assert_eq!(
            snapshot,
            queue_persistence_snapshot_from_proto(&queue_persistence_snapshot_to_proto(&snapshot))
        );

        let lyrics = api::LyricsView {
            plain: None,
            synced: vec![api::LyricLineView {
                start_ms: 1,
                end_ms: None,
                text: "line".into(),
                chunks: vec![api::LyricChunkView {
                    start_ms: 2,
                    text: "word".into(),
                }],
                parent_line_index: Some(0),
                background: true,
                opposite_turn: true,
            }],
        };
        assert_eq!(lyrics, lyrics_from_proto(&lyrics_to_proto(&lyrics)));

        let stats = api::StatsView {
            listen_counts: std::collections::HashMap::from([("uid".into(), 3)]),
        };
        assert_eq!(stats, stats_from_proto(&stats_to_proto(&stats)));

        let favorites = api::FavoritesView {
            refs: vec!["key".into()],
            generation: 8,
        };
        assert_eq!(
            favorites,
            favorites_from_proto(&favorites_to_proto(&favorites))
        );

        let job = api::JobStatus {
            id: "job".into(),
            kind: api::JobKind::LibrarySync,
            state: api::JobState::Failed,
            phase: "done".into(),
            current: None,
            total: Some(5),
            message: None,
            error: Some(api::ErrorBody {
                code: api::ErrorCode::Internal,
                message: "failed".into(),
                details: Some(serde_json::json!({"retry": false})),
            }),
            request: Some("https://example.com/watch".into()),
            title: Some("Example".into()),
            format: Some("OPUS".into()),
            speed: Some("2MiB/s".into()),
            eta: Some("00:03".into()),
        };
        assert_eq!(job, job_status_from_proto(&job_status_to_proto(&job)));

        let download = api::DownloadItemStatus {
            key: "track".into(),
            state: api::DownloadItemState::Downloading,
            bytes_done: 512,
            total_bytes: Some(1024),
            error: None,
        };
        assert_eq!(
            download,
            download_status_from_proto(&download_status_to_proto(&download))
        );

        let config = api::ConfigView {
            config: serde_json::json!({"volume": 0.5}),
            locked_keys: vec!["theme".into()],
        };
        assert_eq!(
            config,
            config_view_from_proto(&config_view_to_proto(&config))
        );
    }

    #[test]
    fn frontend_dtos_round_trip_with_every_oneof_arm() {
        let album = api::AlbumInfo {
            id: "album".into(),
            title: "Title".into(),
            artist: "Artist".into(),
            genre: "Genre".into(),
            year: 2025,
            artwork: Some("album".into()),
            manual_artwork: true,
        };
        assert_eq!(album, album_info_from_proto(&album_info_to_proto(&album)));
        let album_page = api::AlbumPage {
            total: 4,
            offset: 2,
            items: vec![album.clone()],
        };
        assert_eq!(
            album_page,
            album_page_from_proto(&album_page_to_proto(&album_page))
        );

        let artist = api::ArtistInfo {
            name: "Artist".into(),
            track_count: 8,
            album_count: 2,
            artwork: Some("Artist".into()),
            manual_artwork: true,
        };
        let artist_page = api::ArtistPage {
            total: 1,
            offset: 0,
            items: vec![artist],
        };
        assert_eq!(
            artist_page,
            artist_page_from_proto(&artist_page_to_proto(&artist_page))
        );

        let catalog = api::PlaylistCatalog {
            playlists: vec![api::PlaylistInfo {
                id: "playlist".into(),
                name: "Playlist".into(),
                track_count: 2,
                track_keys: vec!["a".into(), "b".into()],
                artwork: Some("playlist".into()),
                manual_artwork: false,
            }],
            folders: vec![api::PlaylistFolderInfo {
                id: "folder".into(),
                name: "Folder".into(),
                playlist_ids: vec!["playlist".into()],
            }],
        };
        assert_eq!(
            catalog,
            playlist_catalog_from_proto(&playlist_catalog_to_proto(&catalog))
        );

        for service in [
            api::MusicService::Jellyfin,
            api::MusicService::Subsonic,
            api::MusicService::Custom,
            api::MusicService::YtMusic,
            api::MusicService::AppleMusic,
            api::MusicService::SoundCloud,
            api::MusicService::Spotify,
            api::MusicService::Nextcloud,
        ] {
            assert_eq!(
                service,
                music_service_from_proto(music_service_to_proto(service) as i32)
            );
        }
        for kind in [
            api::SourceKind::Local,
            api::SourceKind::LocalLibrary,
            api::SourceKind::Server,
        ] {
            assert_eq!(
                kind,
                source_kind_from_proto(source_kind_to_proto(kind) as i32)
            );
        }
        for capability in [
            api::PlaylistCapability::None,
            api::PlaylistCapability::AddRemove,
            api::PlaylistCapability::Reorder,
        ] {
            assert_eq!(
                capability,
                playlist_capability_from_proto(playlist_capability_to_proto(capability) as i32)
            );
        }
        for presentation in [
            api::ArtistPresentation::Library,
            api::ArtistPresentation::Remote,
        ] {
            assert_eq!(
                presentation,
                artist_presentation_from_proto(artist_presentation_to_proto(presentation) as i32)
            );
        }
        for presentation in [
            api::AlbumPresentation::Standard,
            api::AlbumPresentation::Remote,
        ] {
            assert_eq!(
                presentation,
                album_presentation_from_proto(album_presentation_to_proto(presentation) as i32)
            );
        }

        let source = api::SourceInfo {
            id: "server".into(),
            name: "Server".into(),
            kind: api::SourceKind::Server,
            service: Some(api::MusicService::Subsonic),
            active: true,
            authenticated: true,
            capabilities: api::SourceCapabilities {
                edit_tags: true,
                delete_from_disk: true,
                scan_folders: true,
                folders: true,
                sync: true,
                downloads: true,
                discover: true,
                track_radio: true,
                playlist_radio: true,
                playlists: api::PlaylistCapability::Reorder,
                artists: api::ArtistPresentation::Remote,
                albums: api::AlbumPresentation::Remote,
                favorites_sync: api::FavoritesSyncMode::Paginated,
            },
            url: Some("https://example.com".into()),
            browser: Some("chrome".into()),
            anonymous: true,
            storefront: Some("tr".into()),
            language: Some("tr".into()),
            directories: vec!["/music".into()],
        };
        assert_eq!(
            source,
            source_info_from_proto(&source_info_to_proto(&source))
        );

        let local = api::LocalSourceDraft {
            id: Some("local:test".into()),
            name: "Test".into(),
            directories: vec!["/music".into(), "/more".into()],
        };
        assert_eq!(
            local,
            local_source_draft_from_proto(&local_source_draft_to_proto(&local))
        );

        let server = api::ServerDraft {
            id: Some("server".into()),
            name: "Server".into(),
            url: "https://example.com".into(),
            service: api::MusicService::YtMusic,
            browser: Some("chrome".into()),
            anonymous: true,
            storefront: Some("tr".into()),
            language: Some("tr".into()),
        };
        assert_eq!(
            server,
            server_draft_from_proto(&server_draft_to_proto(&server))
        );
        let credential = api::CredentialProvision {
            server_id: "server".into(),
            secret: "secret".into(),
            user_id: Some("user".into()),
            browser: Some("chrome".into()),
        };
        assert_eq!(
            credential,
            credential_from_proto(&credential_to_proto(&credential))
        );
        let login = api::SourceLoginRequest {
            server_id: "server".into(),
            username: "user".into(),
            password: "password".into(),
        };
        assert_eq!(
            login,
            source_login_from_proto(&source_login_to_proto(&login))
        );
        let integration = api::IntegrationCredentialProvision {
            kind: api::IntegrationKind::LastFm,
            token: None,
            api_key: Some("key".into()),
            api_secret: Some("secret".into()),
            session_key: Some("session".into()),
        };
        assert_eq!(
            integration,
            integration_provision_from_proto(&integration_provision_to_proto(&integration))
        );
        let integration_status = api::IntegrationCredentialStatus {
            kind: api::IntegrationKind::LibreFm,
            configured: true,
        };
        assert_eq!(
            integration_status,
            integration_status_from_proto(&integration_status_to_proto(&integration_status))
        );
        let folder = api::SourceFolderEntry {
            path: "/Music".into(),
            name: "Music".into(),
        };
        assert_eq!(
            folder,
            source_folder_from_proto(&source_folder_to_proto(&folder))
        );
        let ytdlp = api::YtdlpRequest {
            url: "https://example.com/watch".into(),
            output_dir: "/tmp/music".into(),
            format: api::YtdlpAudioFormat::M4a,
            options: serde_json::json!({"embed_metadata": true}),
        };
        assert_eq!(
            ytdlp,
            ytdlp_request_from_proto(&ytdlp_request_to_proto(&ytdlp)).expect("yt-dlp request")
        );
        for format in [
            api::YtdlpAudioFormat::Best,
            api::YtdlpAudioFormat::Mp3,
            api::YtdlpAudioFormat::M4a,
            api::YtdlpAudioFormat::Opus,
            api::YtdlpAudioFormat::Flac,
            api::YtdlpAudioFormat::Wav,
            api::YtdlpAudioFormat::Video,
        ] {
            assert_eq!(
                format,
                ytdlp_format_from_proto(ytdlp_format_to_proto(format) as i32)
            );
        }
        let playback = api::ExternalPlayback {
            kind: "spotify".into(),
            device: Some("device".into()),
        };
        assert_eq!(
            playback,
            external_playback_from_proto(&external_playback_to_proto(&playback))
        );
        let lease = api::ExternalPlaybackLease {
            lease_id: "lease".into(),
            expires_in_ms: 15_000,
        };
        assert_eq!(
            lease,
            external_lease_from_proto(&external_lease_to_proto(&lease))
        );
        let report = api::ExternalPlaybackReport {
            lease_id: "lease".into(),
            track: Some(api::TrackInfo {
                key: "track".into(),
                service: Some(api::MusicService::Spotify),
                ..Default::default()
            }),
            position_ms: 42,
            playing: true,
            completed: false,
            device: Some("device".into()),
        };
        assert_eq!(
            report,
            external_report_from_proto(&external_report_to_proto(&report))
        );
        let external = api::ExternalAccess {
            kind: "spotify".into(),
            access_token: "token".into(),
            client_id: Some("client".into()),
        };
        assert_eq!(
            external,
            external_access_from_proto(&external_access_to_proto(&external))
        );

        let track = api::TrackInfo {
            key: "track".into(),
            title: "Track".into(),
            ..Default::default()
        };
        for kind in [
            api::CatalogItemKind::Track,
            api::CatalogItemKind::Album,
            api::CatalogItemKind::Playlist,
            api::CatalogItemKind::Artist,
            api::CatalogItemKind::Mood,
        ] {
            let page = api::CatalogPage {
                shelves: vec![api::CatalogShelf {
                    title: "Shelf".into(),
                    strapline: Some("Strapline".into()),
                    items: vec![api::CatalogItem {
                        kind,
                        id: "item".into(),
                        title: "Item".into(),
                        subtitle: Some("Subtitle".into()),
                        artwork: Some("item".into()),
                        track: Some(track.clone()),
                    }],
                    more_ref: Some("more".into()),
                    list: true,
                }],
                continuation: Some("next".into()),
            };
            assert_eq!(page, catalog_page_from_proto(&catalog_page_to_proto(&page)));
            let request = api::CatalogDetailRequest {
                kind,
                id: "detail".into(),
                continuation: Some("cursor".into()),
            };
            assert_eq!(
                request,
                catalog_detail_request_from_proto(&catalog_detail_request_to_proto(&request))
            );
            let detail = api::CatalogDetail {
                kind,
                id: "detail".into(),
                title: "Detail".into(),
                subtitle: Some("Subtitle".into()),
                description: Some("Description".into()),
                artwork: Some("artwork".into()),
                playback_id: Some("playback".into()),
                year: Some("2026".into()),
                tracks: vec![track.clone()],
                shelves: page.shelves,
                continuation: Some("next".into()),
            };
            assert_eq!(
                detail,
                catalog_detail_from_proto(&catalog_detail_to_proto(&detail))
            );
        }

        let station = api::RadioStationInfo {
            id: "station".into(),
            name: "Station".into(),
            description: "Description".into(),
            icon: "fa-solid fa-radio".into(),
            artwork: Some("https://example.com/cover.png".into()),
            tags: vec!["tag".into()],
            streams: vec![api::RadioStreamInfo {
                id: "main".into(),
                name: "Main".into(),
                url: "https://example.com/stream".into(),
                icon: Some("fa-solid fa-play".into()),
            }],
            pinned: true,
        };
        assert_eq!(
            station,
            radio_station_from_proto(&radio_station_to_proto(&station))
        );
        let registry = api::RadioRegistryInfo {
            url: "https://example.com/index.json".into(),
            enabled: true,
            built_in: false,
        };
        assert_eq!(
            registry,
            radio_registry_from_proto(&radio_registry_to_proto(&registry))
        );

        let metadata = api::TrackMetadataPatch {
            key: "track".into(),
            title: Some("Title".into()),
            artist: None,
            album: Some("Album".into()),
            track_number: Some(3),
            clear_track_number: false,
            disc_number: None,
            clear_disc_number: true,
        };
        assert_eq!(
            metadata,
            metadata_patch_from_proto(&metadata_patch_to_proto(&metadata))
        );

        let targets = [
            api::ArtworkTarget::Track {
                key: "track".into(),
            },
            api::ArtworkTarget::Album { id: "album".into() },
            api::ArtworkTarget::Artist {
                name: "artist".into(),
            },
            api::ArtworkTarget::Playlist {
                id: "playlist".into(),
            },
        ];
        for target in targets {
            let upload = api::ArtworkUpload {
                target: Some(target.clone()),
                content_type: "image/png".into(),
                data: vec![1, 2, 3],
            };
            assert_eq!(
                upload,
                artwork_upload_from_proto(&artwork_upload_to_proto(&upload))
            );
            assert_eq!(
                target,
                remove_artwork_from_proto(&remove_artwork_to_proto(&target))
                    .expect("artwork target survives")
            );
        }
        assert_eq!(
            api::ArtworkUpload::default(),
            artwork_upload_from_proto(&artwork_upload_to_proto(&api::ArtworkUpload::default()))
        );

        let entities = [
            api::ArtworkEntity::Track {
                key: "track".into(),
            },
            api::ArtworkEntity::Album { id: "album".into() },
            api::ArtworkEntity::Artist {
                name: "artist".into(),
            },
            api::ArtworkEntity::Playlist {
                id: "playlist".into(),
            },
        ];
        for entity in entities {
            let request = api::ArtworkRequest {
                entity: Some(entity),
                hq: true,
            };
            assert_eq!(
                request,
                artwork_request_from_proto(&artwork_request_to_proto(&request))
            );
        }
        assert_eq!(
            api::ArtworkRequest::default(),
            artwork_request_from_proto(&artwork_request_to_proto(&api::ArtworkRequest::default()))
        );
    }

    #[test]
    fn unknown_event_kind_is_ignorable() {
        assert!(event_from_proto(&Event { kind: None }).is_none());
        assert_eq!(phase_from_proto(i32::MAX), api::Phase::Idle);
        assert_eq!(loop_from_proto(i32::MAX), api::LoopMode::None);
        assert_eq!(track_kind_from_proto(i32::MAX), api::TrackKind::Normal);
        assert_eq!(queue_mode_from_proto(i32::MAX), api::QueueMode::Replace);
        assert_eq!(table_from_proto(i32::MAX), api::Table::Unknown);
        assert_eq!(job_kind_from_proto(i32::MAX), api::JobKind::Unknown);
        assert_eq!(
            download_item_state_from_proto(i32::MAX),
            api::DownloadItemState::Unknown
        );
        assert_eq!(
            integration_kind_from_proto(i32::MAX),
            api::IntegrationKind::Unknown
        );
        assert_eq!(
            ytdlp_format_from_proto(i32::MAX),
            api::YtdlpAudioFormat::Best
        );
        assert_eq!(
            music_service_from_proto(i32::MAX),
            api::MusicService::Unknown
        );
        assert_eq!(source_kind_from_proto(i32::MAX), api::SourceKind::Unknown);
        assert_eq!(
            playlist_capability_from_proto(i32::MAX),
            api::PlaylistCapability::None
        );
        assert_eq!(
            artist_presentation_from_proto(i32::MAX),
            api::ArtistPresentation::Library
        );
        assert_eq!(
            album_presentation_from_proto(i32::MAX),
            api::AlbumPresentation::Standard
        );
        assert_eq!(
            catalog_item_kind_from_proto(i32::MAX),
            api::CatalogItemKind::Unknown
        );
    }
}
