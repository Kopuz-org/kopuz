use crate::*;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unspecified_status_values_are_unknown() {
        assert_eq!(job_state_from_proto(0), api::JobState::Unknown);
        assert_eq!(notice_level_from_proto(0), api::NoticeLevel::Unknown);
    }
}
