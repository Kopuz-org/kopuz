//! Fetching a URL to a file, and following it.
//!
//! Which tool does it, where it writes and every option it is given belong to
//! the daemon; a page names a URL and a format, then watches the job.

use dioxus::prelude::*;

use crate::api::{consume_api, use_api};

/// Ask the daemon to fetch a URL. Preconditions -- the tool installed, the
/// folder writable -- are checked there, so a refusal comes back as an error
/// rather than being re-derived here.
pub fn start(url: String, format: String, mut failure: Signal<Option<String>>) {
    let api = consume_api();
    spawn(async move {
        match api.download_url(url, format).await {
            Ok(_) => failure.set(None),
            Err(error) => {
                tracing::warn!(%error, "the download was refused");
                failure.set(Some(error.to_string()));
            }
        }
    });
}

/// The formats a download can be asked for.
pub fn use_formats() -> Resource<Vec<api::ChoiceOption>> {
    let api = use_api();
    use_resource(move || {
        let api = api.clone();
        async move { api.download_formats().await.unwrap_or_default() }
    })
}

/// The downloader's own options, with the values it currently has.
pub fn use_settings(reload: Signal<u64>) -> Resource<Vec<api::FieldSpec>> {
    let api = use_api();
    use_resource(move || {
        let _ = reload();
        let api = api.clone();
        async move { api.downloader_settings().await.unwrap_or_default() }
    })
}

/// Answer one of those options.
pub fn set_setting(value: api::FieldValue, mut done: Signal<u64>) {
    let api = consume_api();
    spawn(async move {
        match api.set_downloader_settings(vec![value]).await {
            Ok(_) => done += 1,
            Err(error) => {
                tracing::warn!(%error, "saving a downloader setting failed");
                crate::toast::toast_error(&error.to_string());
            }
        }
    });
}

/// What has been fetched, newest first.
pub fn use_history(reload: Signal<u64>) -> Resource<Vec<api::DownloadHistoryEntry>> {
    let api = use_api();
    use_resource(move || {
        let _ = reload();
        let api = api.clone();
        async move { api.downloader_history().await.unwrap_or_default() }
    })
}

pub fn clear_history(mut done: Signal<u64>) {
    let api = consume_api();
    spawn(async move {
        match api.clear_downloader_history().await {
            Ok(()) => done += 1,
            Err(error) => {
                tracing::warn!(%error, "clearing the download history failed");
                crate::toast::toast_error(&error.to_string());
            }
        }
    });
}

/// What the running download is doing, if one is.
pub fn use_progress() -> Signal<crate::jobs::JobProgress> {
    crate::jobs::use_job_progress(api::JobKind::UrlDownload)
}
