//! Starting a yt-dlp download and following it.
//!
//! The subprocess, the PATH search and the output-directory check belong to
//! the daemon; a page names a URL and a format, then watches the job.

use dioxus::prelude::*;

use crate::api::consume_api;

/// Ask the daemon to fetch a URL. Preconditions -- yt-dlp installed, ffmpeg
/// installed, the folder writable -- are checked there, so a refusal comes
/// back as an error rather than being re-derived here.
pub fn start(request: api::YtdlpRequest, mut failure: Signal<Option<String>>) {
    let api = consume_api();
    spawn(async move {
        match api.start_ytdlp(request).await {
            Ok(_) => failure.set(None),
            Err(error) => {
                tracing::warn!(%error, "yt-dlp download refused");
                failure.set(Some(error.to_string()));
            }
        }
    });
}

/// What the running download is doing, if one is.
pub fn use_progress() -> Signal<crate::jobs::JobProgress> {
    crate::jobs::use_job_progress(api::JobKind::Ytdlp)
}
