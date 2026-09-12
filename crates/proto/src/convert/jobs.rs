use crate::*;

pub fn download_item_state_to_proto(value: api::DownloadItemState) -> DownloadItemState {
    match value {
        api::DownloadItemState::Queued => DownloadItemState::DownloadItemQueued,
        api::DownloadItemState::Downloading => DownloadItemState::DownloadItemDownloading,
        api::DownloadItemState::Failed => DownloadItemState::DownloadItemFailed,
    }
}

pub fn download_item_state_from_proto(value: i32) -> api::DownloadItemState {
    match DownloadItemState::try_from(value) {
        Ok(DownloadItemState::DownloadItemDownloading) => api::DownloadItemState::Downloading,
        Ok(DownloadItemState::DownloadItemFailed) => api::DownloadItemState::Failed,
        _ => api::DownloadItemState::Queued,
    }
}

pub fn download_status_to_proto(value: &api::DownloadItemStatus) -> DownloadItemStatus {
    DownloadItemStatus {
        key: value.key.clone(),
        state: download_item_state_to_proto(value.state) as i32,
    }
}

pub fn download_status_from_proto(value: &DownloadItemStatus) -> api::DownloadItemStatus {
    api::DownloadItemStatus {
        key: value.key.clone(),
        state: download_item_state_from_proto(value.state),
    }
}

pub fn download_state_to_proto(value: api::DownloadState) -> DownloadState {
    match value {
        api::DownloadState::Finished => DownloadState::Finished,
        api::DownloadState::Failed => DownloadState::Failed,
    }
}

pub fn download_state_from_proto(value: i32) -> api::DownloadState {
    match DownloadState::try_from(value) {
        Ok(DownloadState::Failed) => api::DownloadState::Failed,
        Ok(DownloadState::Finished) | Ok(DownloadState::Unspecified) | Err(_) => {
            api::DownloadState::Finished
        }
    }
}

pub fn download_history_entry_to_proto(value: &api::DownloadHistoryEntry) -> DownloadHistoryEntry {
    DownloadHistoryEntry {
        url: value.url.clone(),
        title: value.title.clone(),
        format: value.format.clone(),
        state: download_state_to_proto(value.state) as i32,
        error: value.error.clone(),
    }
}

pub fn download_history_entry_from_proto(
    value: &DownloadHistoryEntry,
) -> api::DownloadHistoryEntry {
    api::DownloadHistoryEntry {
        url: value.url.clone(),
        title: value.title.clone(),
        format: value.format.clone(),
        state: download_state_from_proto(value.state),
        error: value.error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_status_round_trips_for_every_state() {
        for state in [
            api::DownloadItemState::Queued,
            api::DownloadItemState::Downloading,
            api::DownloadItemState::Failed,
        ] {
            let status = api::DownloadItemStatus {
                key: "track-1".into(),
                state,
            };
            assert_eq!(
                status,
                download_status_from_proto(&download_status_to_proto(&status))
            );
        }
    }

    /// The downloader page lists these, so an outcome and the reason for a
    /// failed one both have to survive the wire.
    #[test]
    fn a_history_entry_round_trips_with_and_without_an_error() {
        let entries = [
            api::DownloadHistoryEntry {
                url: "https://example.test/watch?v=1".into(),
                title: "A Song".into(),
                format: "flac".into(),
                state: api::DownloadState::Finished,
                error: None,
            },
            api::DownloadHistoryEntry {
                url: "https://example.test/watch?v=2".into(),
                title: "Another Song".into(),
                format: "best-audio".into(),
                state: api::DownloadState::Failed,
                error: Some("HTTP 403".into()),
            },
        ];
        for entry in entries {
            assert_eq!(
                entry,
                download_history_entry_from_proto(&download_history_entry_to_proto(&entry))
            );
        }
    }

    /// An outcome this build cannot name is not a failure to report.
    #[test]
    fn unknown_wire_values_fall_back_to_the_default() {
        assert_eq!(download_state_from_proto(0), api::DownloadState::Finished);
        assert_eq!(download_state_from_proto(404), api::DownloadState::Finished);
        assert_eq!(
            download_item_state_from_proto(404),
            api::DownloadItemState::Queued
        );
    }
}
