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
        }
    }

    pub fn queue_mode_from_proto(value: i32) -> api::QueueMode {
        match QueueMode::try_from(value).unwrap_or(QueueMode::Unspecified) {
            QueueMode::Append => api::QueueMode::Append,
            QueueMode::PlayNext => api::QueueMode::PlayNext,
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
            JobKind::Unspecified => api::JobKind::Unknown,
        }
    }

    pub fn job_state_to_proto(value: api::JobState) -> JobState {
        match value {
            api::JobState::Running => JobState::Running,
            api::JobState::Finished => JobState::Finished,
            api::JobState::Failed => JobState::Failed,
            api::JobState::Cancelled => JobState::Cancelled,
        }
    }

    pub fn job_state_from_proto(value: i32) -> api::JobState {
        match JobState::try_from(value).unwrap_or(JobState::Unspecified) {
            JobState::Running => api::JobState::Running,
            JobState::Finished => api::JobState::Finished,
            JobState::Cancelled => api::JobState::Cancelled,
            JobState::Failed | JobState::Unspecified => api::JobState::Failed,
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
        }
    }

    pub fn notice_level_from_proto(value: i32) -> api::NoticeLevel {
        match NoticeLevel::try_from(value).unwrap_or(NoticeLevel::Unspecified) {
            NoticeLevel::Info => api::NoticeLevel::Info,
            NoticeLevel::Warning => api::NoticeLevel::Warning,
            NoticeLevel::Error | NoticeLevel::Unspecified => api::NoticeLevel::Error,
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
        })
    }

    pub fn set_queue_to_proto(value: &api::SetQueueRequest) -> SetQueueRequest {
        SetQueueRequest {
            mode: queue_mode_to_proto(value.mode) as i32,
            context: Some(queue_context_to_proto(&value.context)),
            start_index: value.start_index,
            shuffle: value.shuffle,
        }
    }

    pub fn set_queue_from_proto(value: &SetQueueRequest) -> Option<api::SetQueueRequest> {
        Some(api::SetQueueRequest {
            mode: queue_mode_from_proto(value.mode),
            context: queue_context_from_proto(value.context.as_ref()?)?,
            start_index: value.start_index,
            shuffle: value.shuffle,
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
        }
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
        }
    }

    #[test]
    fn player_state_round_trips() {
        let state = sample_state();
        let back = player_state_from_proto(&player_state_to_proto(&state));
        assert_eq!(state, back);
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
            api::PlayerCommand::Seek { position_ms: 5 },
            api::PlayerCommand::SetVolume { volume: 0.5 },
            api::PlayerCommand::SetMode {
                shuffle: Some(true),
                loop_mode: Some(api::LoopMode::Track),
            },
        ];
        for command in commands {
            let back =
                command_from_proto(&command_to_proto(&command, 1)).expect("command survives");
            assert_eq!(command, back);
        }

        let request = api::SetQueueRequest {
            mode: api::QueueMode::PlayNext,
            context: api::QueueContext::Radio {
                station_id: "s".into(),
                stream_id: "st".into(),
            },
            start_index: Some(2),
            shuffle: Some(false),
        };
        let back = set_queue_from_proto(&set_queue_to_proto(&request)).expect("request survives");
        assert_eq!(request, back);

        let edit = api::QueueEdit::Move { from: 1, to: 3 };
        let back = queue_edit_from_proto(&queue_edit_to_proto(&edit)).expect("edit survives");
        assert_eq!(edit, back);
    }

    #[test]
    fn unknown_event_kind_is_ignorable() {
        assert!(event_from_proto(&Event { kind: None }).is_none());
    }
}
