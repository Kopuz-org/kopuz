//! Refresh reactive local metadata when another machine commits to the
//! database carried by the active music folder.

use std::path::PathBuf;
use std::time::SystemTime;

use dioxus::prelude::*;
use server::source::ActiveSource;

use crate::db_reactivity::{Table, use_generations};

type FileSignature = Option<(SystemTime, u64)>;

pub fn use_portable_metadata_watch() {
    let active_source = use_context::<Signal<ActiveSource>>();
    let gens = use_generations();

    use_future(move || async move {
        let mut observed: Option<(Option<PathBuf>, FileSignature)> = None;
        loop {
            let path = {
                let source = active_source.read().clone();
                source.portable_metadata_path()
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
            let current = (path, signature);
            if let Some(previous) = observed.as_ref()
                && previous.0 == current.0
                && previous.1 != current.1
            {
                gens.bump(Table::Favorites);
                gens.bump(Table::Playlists);
                gens.bump(Table::Folders);
            }
            observed = Some(current);
            utils::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}
