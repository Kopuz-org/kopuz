use std::sync::Arc;
use std::time::Duration;

use api::{JobKind, JobState, KopuzApi};
use dioxus::prelude::*;

#[tracing::instrument(name = "library.sync", skip_all)]
pub async fn sync_server_library() -> Result<(), String> {
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let job = api
        .start_job(JobKind::LibrarySync)
        .await
        .map_err(|error| error.to_string())?;
    loop {
        let jobs = api.jobs().await.map_err(|error| error.to_string())?;
        let status = jobs
            .iter()
            .find(|status| status.id == job.job_id)
            .ok_or_else(|| "library sync job disappeared".to_string())?;
        match status.state {
            JobState::Running => tokio::time::sleep(Duration::from_millis(100)).await,
            JobState::Finished => return Ok(()),
            JobState::Cancelled => return Err("library sync was cancelled".to_string()),
            JobState::Failed => {
                return Err(status
                    .error
                    .as_ref()
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| "library sync failed".to_string()));
            }
            JobState::Unknown => return Err("library sync returned an unknown state".to_string()),
        }
    }
}
