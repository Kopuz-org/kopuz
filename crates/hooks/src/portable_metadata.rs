//! Refresh reactive local metadata when another machine commits to the
//! database carried by the active music folder.

use std::path::PathBuf;
use std::time::SystemTime;

use config::{AppConfig, Source};
use dioxus::prelude::*;
use server::source::ActiveSource;

use crate::db_reactivity::{Table, use_generations};

type FileSignature = Option<(SystemTime, u64)>;

pub fn use_portable_metadata_watch(mut config: Signal<AppConfig>) {
    let active_source = use_context::<Signal<ActiveSource>>();
    let gens = use_generations();

    use_future(move || async move {
        let mut observed: Option<(Source, Option<PathBuf>, FileSignature)> = None;
        loop {
            let (source, source_id, path) = {
                let source = active_source.read().clone();
                let source_id = source.source().clone();
                let path = source.portable_metadata_path();
                (source, source_id, path)
            };
            let signature = match path.as_ref() {
                Some(path) => tokio::fs::metadata(path).await.ok().map(|metadata| {
                    (
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        metadata.len(),
                    )
                }),
                None => None,
            };
            let current = (source_id, path, signature);
            let needs_refresh = current.1.is_some()
                && observed.as_ref().is_none_or(|previous| {
                    previous.0 != current.0 || previous.1 != current.1 || previous.2 != current.2
                });
            if needs_refresh {
                match source.sync_portable_activity().await {
                    Ok(counts) => {
                        let mut config = config.write();
                        for (key, count) in counts {
                            let current = config.listen_counts.entry(key).or_insert(0);
                            *current = (*current).max(count);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to refresh shared library activity");
                    }
                }
                gens.bump(Table::Favorites);
                gens.bump(Table::Playlists);
                gens.bump(Table::Folders);
                gens.bump(Table::Recents);
                gens.bump(Table::Tracks);
            }
            observed = Some(current);
            utils::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}
