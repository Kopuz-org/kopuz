use super::{track_sort_field_from_proto, track_sort_field_to_proto};
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

pub fn track_version_to_proto(value: api::TrackVersion) -> TrackVersion {
    match value {
        api::TrackVersion::Song => TrackVersion::Song,
        api::TrackVersion::Video => TrackVersion::Video,
    }
}

/// `None` for an unspecified version, which names no cut to switch to.
pub fn track_version_from_proto(value: i32) -> Option<api::TrackVersion> {
    match TrackVersion::try_from(value).unwrap_or(TrackVersion::Unspecified) {
        TrackVersion::Song => Some(api::TrackVersion::Song),
        TrackVersion::Video => Some(api::TrackVersion::Video),
        TrackVersion::Unspecified => None,
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
        api::Table::Stations => Table::Stations,
        api::Table::Artists => Table::Artists,
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
        Table::Stations => api::Table::Stations,
        Table::Artists => api::Table::Artists,
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
        api::JobKind::UrlDownload => JobKind::UrlDownload,
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
        JobKind::UrlDownload => api::JobKind::UrlDownload,
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
        api::SourceState::Checking => SourceState::Checking,
        api::SourceState::Online => SourceState::Online,
        api::SourceState::AuthExpired => SourceState::AuthExpired,
        api::SourceState::Offline => SourceState::Offline,
    }
}

pub fn source_state_from_proto(value: i32) -> api::SourceState {
    match SourceState::try_from(value).unwrap_or(SourceState::Unspecified) {
        SourceState::Checking => api::SourceState::Checking,
        SourceState::Online => api::SourceState::Online,
        SourceState::AuthExpired => api::SourceState::AuthExpired,
        SourceState::Offline | SourceState::Unspecified => api::SourceState::Offline,
    }
}

pub fn connection_check_to_proto(value: &api::ConnectionCheck) -> ConnectionCheck {
    ConnectionCheck {
        step: match value.step {
            api::CheckStep::Reach => CheckStep::Reach,
            api::CheckStep::Session => CheckStep::Session,
            api::CheckStep::SignedInPlayback => CheckStep::SignedInPlayback,
            api::CheckStep::AnonymousPlayback => CheckStep::AnonymousPlayback,
            api::CheckStep::PlaybackToken => CheckStep::PlaybackToken,
        } as i32,
        outcome: match value.outcome {
            api::CheckOutcome::Passed => CheckOutcome::Passed,
            api::CheckOutcome::Failed => CheckOutcome::Failed,
            api::CheckOutcome::Skipped => CheckOutcome::Skipped,
        } as i32,
        detail: value.detail.clone(),
    }
}

/// A step this build does not know is dropped rather than shown as another.
pub fn connection_check_from_proto(value: &ConnectionCheck) -> Option<api::ConnectionCheck> {
    let step = match CheckStep::try_from(value.step).ok()? {
        CheckStep::Reach => api::CheckStep::Reach,
        CheckStep::Session => api::CheckStep::Session,
        CheckStep::SignedInPlayback => api::CheckStep::SignedInPlayback,
        CheckStep::AnonymousPlayback => api::CheckStep::AnonymousPlayback,
        CheckStep::PlaybackToken => api::CheckStep::PlaybackToken,
        CheckStep::Unspecified => return None,
    };
    let outcome = match CheckOutcome::try_from(value.outcome).unwrap_or(CheckOutcome::Unspecified) {
        CheckOutcome::Passed => api::CheckOutcome::Passed,
        CheckOutcome::Failed => api::CheckOutcome::Failed,
        CheckOutcome::Skipped | CheckOutcome::Unspecified => api::CheckOutcome::Skipped,
    };
    Some(api::ConnectionCheck {
        step,
        outcome,
        detail: value.detail.clone(),
    })
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

pub fn album_order_field_to_proto(value: ::config::AlbumSortField) -> AlbumOrderField {
    match value {
        ::config::AlbumSortField::Title => AlbumOrderField::Title,
        ::config::AlbumSortField::Artist => AlbumOrderField::Artist,
        ::config::AlbumSortField::Year => AlbumOrderField::Year,
        ::config::AlbumSortField::Genre => AlbumOrderField::Genre,
        ::config::AlbumSortField::RecentlyAdded => AlbumOrderField::RecentlyAdded,
        ::config::AlbumSortField::TrackCount => AlbumOrderField::TrackCount,
    }
}

/// An order this build does not know falls back to the title, as an unspecified one does.
pub fn album_order_field_from_proto(value: i32) -> ::config::AlbumSortField {
    match AlbumOrderField::try_from(value).unwrap_or(AlbumOrderField::Unspecified) {
        AlbumOrderField::Artist => ::config::AlbumSortField::Artist,
        AlbumOrderField::Year => ::config::AlbumSortField::Year,
        AlbumOrderField::Genre => ::config::AlbumSortField::Genre,
        AlbumOrderField::RecentlyAdded => ::config::AlbumSortField::RecentlyAdded,
        AlbumOrderField::TrackCount => ::config::AlbumSortField::TrackCount,
        AlbumOrderField::Title | AlbumOrderField::Unspecified => ::config::AlbumSortField::Title,
    }
}

pub fn artist_order_field_to_proto(value: ::config::ArtistSortField) -> ArtistOrderField {
    match value {
        ::config::ArtistSortField::Name => ArtistOrderField::Name,
        ::config::ArtistSortField::Tracks => ArtistOrderField::TrackCount,
        ::config::ArtistSortField::Albums => ArtistOrderField::AlbumCount,
    }
}

/// An order this build does not know falls back to the name, as an unspecified one does.
pub fn artist_order_field_from_proto(value: i32) -> ::config::ArtistSortField {
    match ArtistOrderField::try_from(value).unwrap_or(ArtistOrderField::Unspecified) {
        ArtistOrderField::TrackCount => ::config::ArtistSortField::Tracks,
        ArtistOrderField::AlbumCount => ::config::ArtistSortField::Albums,
        ArtistOrderField::Name | ArtistOrderField::Unspecified => ::config::ArtistSortField::Name,
    }
}

pub fn track_sort_to_proto(value: &api::TrackSort) -> TrackSort {
    match value {
        api::TrackSort::Default => TrackSort::Unspecified,
        api::TrackSort::Title => TrackSort::Title,
        api::TrackSort::Artist => TrackSort::Artist,
        api::TrackSort::Album => TrackSort::Album,
        api::TrackSort::DateAdded => TrackSort::DateAdded,
        api::TrackSort::PlayCount => TrackSort::PlayCount,
        // The criteria themselves ride TrackFilter.sort_fields.
        api::TrackSort::Fields(_) => TrackSort::Fields,
    }
}

/// `fields` comes from `TrackFilter.sort_fields`; an empty list with the
/// Fields tag means the caller asked for no ordering in particular.
pub fn track_sort_from_proto(value: i32, fields: &[SortCriterion]) -> api::TrackSort {
    match TrackSort::try_from(value).unwrap_or(TrackSort::Unspecified) {
        TrackSort::Unspecified => api::TrackSort::Default,
        TrackSort::Title => api::TrackSort::Title,
        TrackSort::Artist => api::TrackSort::Artist,
        TrackSort::Album => api::TrackSort::Album,
        TrackSort::DateAdded => api::TrackSort::DateAdded,
        TrackSort::PlayCount => api::TrackSort::PlayCount,
        TrackSort::Fields => api::TrackSort::Fields(
            fields
                .iter()
                .map(|criterion| ::config::SortCriterion {
                    field: track_sort_field_from_proto(criterion.field),
                    direction: if criterion.descending {
                        ::config::SortDirection::Desc
                    } else {
                        ::config::SortDirection::Asc
                    },
                })
                .collect(),
        ),
    }
}

pub fn sort_criteria_to_proto(value: &api::TrackSort) -> Vec<SortCriterion> {
    let api::TrackSort::Fields(fields) = value else {
        return Vec::new();
    };
    fields
        .iter()
        .map(|criterion| SortCriterion {
            field: track_sort_field_to_proto(criterion.field) as i32,
            descending: criterion.direction == ::config::SortDirection::Desc,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unspecified_status_values_are_unknown() {
        assert_eq!(job_state_from_proto(0), api::JobState::Unknown);
        assert_eq!(notice_level_from_proto(0), api::NoticeLevel::Unknown);
    }

    #[test]
    fn every_job_kind_round_trips() {
        for kind in [
            api::JobKind::Scan,
            api::JobKind::LibrarySync,
            api::JobKind::FavoritesSync,
            api::JobKind::PlaylistSync,
            api::JobKind::Download,
            api::JobKind::UrlDownload,
            api::JobKind::Unknown,
        ] {
            assert_eq!(kind, job_kind_from_proto(job_kind_to_proto(kind) as i32));
        }
        assert_eq!(job_kind_from_proto(404), api::JobKind::Unknown);
    }

    #[test]
    fn every_listing_order_round_trips() {
        for field in [
            ::config::AlbumSortField::Title,
            ::config::AlbumSortField::Artist,
            ::config::AlbumSortField::Year,
            ::config::AlbumSortField::Genre,
            ::config::AlbumSortField::RecentlyAdded,
            ::config::AlbumSortField::TrackCount,
        ] {
            assert_eq!(
                field,
                album_order_field_from_proto(album_order_field_to_proto(field) as i32)
            );
        }
        for field in [
            ::config::ArtistSortField::Name,
            ::config::ArtistSortField::Tracks,
            ::config::ArtistSortField::Albums,
        ] {
            assert_eq!(
                field,
                artist_order_field_from_proto(artist_order_field_to_proto(field) as i32)
            );
        }
        assert_eq!(
            album_order_field_from_proto(404),
            ::config::AlbumSortField::Title
        );
        assert_eq!(
            artist_order_field_from_proto(404),
            ::config::ArtistSortField::Name
        );
    }

    #[test]
    fn every_table_round_trips() {
        for table in [
            api::Table::Tracks,
            api::Table::Albums,
            api::Table::Playlists,
            api::Table::Favorites,
            api::Table::Folders,
            api::Table::Servers,
            api::Table::Recents,
            api::Table::Stations,
            api::Table::Artists,
            api::Table::Unknown,
        ] {
            assert_eq!(table, table_from_proto(table_to_proto(table) as i32));
        }
        assert_eq!(table_from_proto(404), api::Table::Unknown);
    }

    #[test]
    fn every_source_state_round_trips() {
        for state in [
            api::SourceState::Checking,
            api::SourceState::Online,
            api::SourceState::AuthExpired,
            api::SourceState::Offline,
        ] {
            assert_eq!(
                state,
                source_state_from_proto(source_state_to_proto(state) as i32)
            );
        }
    }
}
