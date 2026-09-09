//! Offline copies: what is stored, what is being fetched, and asking for more.
//!
//! The downloader ran in the UI process. It built credentialed stream URLs
//! from the config, wrote files, decrypted Apple Music in the render thread,
//! and kept its own queue in a signal -- so navigating away could strand a
//! download and no second frontend could see one.
//!
//! It is a job now. What is stored comes from the daemon, and what is in
//! flight comes from that job's progress, so this is a view rather than a
//! worker.

use dioxus::prelude::*;

use crate::api::{consume_api, use_api};
use crate::db_reactivity::{Table, use_generations};
use crate::toast::toast_error;

/// The offline state of the library, as a row renders it.
#[derive(Clone, Default, PartialEq)]
pub struct Downloads {
    /// Keys with a file on disk.
    stored: Vec<String>,
    /// The key the daemon is fetching right now, if any.
    active: Option<String>,
    pub running: bool,
    pub done: u64,
    pub total: u64,
}

impl Downloads {
    pub fn is_stored(&self, key: &str) -> bool {
        self.stored.iter().any(|stored| stored == key)
    }

    /// Whether this key is the one being fetched. The daemon downloads one at
    /// a time, so "queued" is simply "asked for and not stored yet" -- which
    /// the caller already knows from what it just requested.
    pub fn is_active(&self, key: &str) -> bool {
        self.active.as_deref() == Some(key)
    }

    pub fn stored_keys(&self) -> &[String] {
        &self.stored
    }
}

/// The offline view, re-read when a download lands and while one runs.
pub fn use_downloads() -> Memo<Downloads> {
    let api = use_api();
    let gens = use_generations();
    let stored = use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let api = api.clone();
        async move { api.downloads().await.unwrap_or_default() }
    });
    let job = crate::jobs::use_job_progress(api::JobKind::Download);
    use_memo(move || {
        let progress = job.read();
        Downloads {
            stored: stored.read().clone().unwrap_or_default(),
            active: progress.running.then(|| progress.message.clone()).flatten(),
            running: progress.running,
            done: progress.current.unwrap_or(0),
            total: progress.total.unwrap_or(0),
        }
    })
}

/// Ask for offline copies of these tracks.
pub fn start(keys: Vec<String>) {
    if keys.is_empty() {
        return;
    }
    let api = consume_api();
    spawn(async move {
        if let Err(error) = api.download(keys).await {
            tracing::warn!(%error, "starting a download failed");
            toast_error(&error.to_string());
        }
    });
}

/// Forget the offline copies of these tracks, deleting the files.
pub fn remove(keys: Vec<String>) {
    if keys.is_empty() {
        return;
    }
    let api = consume_api();
    spawn(async move {
        for key in keys {
            if let Err(error) = api.remove_download(key.clone()).await {
                tracing::warn!(%error, %key, "removing a download failed");
            }
        }
    });
}

/// Stop the running download job. What has already been fetched stays.
pub fn cancel() {
    let api = consume_api();
    spawn(async move {
        let running: Vec<String> = api
            .jobs()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|job| job.kind == api::JobKind::Download && job.state == api::JobState::Running)
            .map(|job| job.id)
            .collect();
        for id in running {
            let _ = api.cancel_job(id).await;
        }
    });
}
