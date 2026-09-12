use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use reader::models::Album;
use tracing::{error, info};

/// Whether a sync is in flight, for the spinner and the guard below. Global
/// because the task outlives the page that started it.
pub static SYNC_RUNNING: GlobalSignal<bool> = Signal::global(|| false);

/// Start a library sync detached from the calling page, one at a time.
///
/// `spawn_forever` because a scope-tied task dies with its component, and a
/// source slow enough to outlast a page change (Nextcloud probes every file
/// header) never survived to persist anything: the same trap as the downloads
/// in #327. The guard collapses the stampede an empty library provokes, since
/// Home, Library and Album each pull on mount.
pub fn spawn_library_sync(clear_first: bool) {
    if *SYNC_RUNNING.peek() {
        return;
    }
    // A detached task has no scope, so the contexts are read here, not inside it.
    let read_db = consume_context::<hooks::ReadDb>();
    let gens = consume_context::<hooks::db_reactivity::Generations>();
    let source = consume_context::<Signal<::server::source::ActiveSource>>()
        .peek()
        .clone();

    *SYNC_RUNNING.write() = true;
    spawn_forever(async move {
        if let Err(e) = sync_server_library(clear_first, read_db, gens, source).await {
            error!(error = %e, "server library sync failed");
        }
        *SYNC_RUNNING.write() = false;
    });
}

fn normalize_album_id(id: &str) -> String {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() >= 2
        && (parts[0] == "subsonic" || parts[0] == "custom" || parts[0] == "jellyfin")
    {
        format!("{}:{}", parts[0], parts[1])
    } else {
        id.to_string()
    }
}

/// Full library sync for the active server. The per-service fetch + transform
/// (Jellyfin libraries/albums/tracks, Subsonic album-list/songs, artist images)
/// lives in the source layer ([`MediaSource::fetch_library`]); this just persists
/// the snapshot — chunked upsert + coalesced bumps keep the library view
/// streaming in — merges manual covers, and prunes rows the server dropped.
#[tracing::instrument(name = "library.sync", skip_all, fields(clear_first = clear_first))]
pub async fn sync_server_library(
    clear_first: bool,
    read_db: hooks::ReadDb,
    gens: hooks::db_reactivity::Generations,
    source: ::server::source::ActiveSource,
) -> Result<(), String> {
    if !source.capabilities().sync {
        return Ok(());
    }
    let src = source.source().clone();

    // Preserve manual / already-cached covers across a re-sync.
    let existing_albums = read_db.albums(&src).await.unwrap_or_default();
    let merge_cover = |mut album: Album| -> Album {
        if let Some(old) = existing_albums
            .iter()
            .find(|a| normalize_album_id(&a.id) == normalize_album_id(&album.id))
        {
            if album.cover_path.is_none() || old.manual_cover {
                album.cover_path = old.cover_path.clone();
            }
            if old.manual_cover {
                album.manual_cover = true;
            }
        }
        album
    };

    info!("Starting server library sync…");
    let snapshot = source.fetch_library().await.map_err(|e| e.to_string())?;

    let merged_albums: Vec<Album> = snapshot.albums.into_iter().map(merge_cover).collect();
    for chunk in merged_albums.chunks(100) {
        source
            .upsert_albums(chunk)
            .await
            .map_err(|e| e.to_string())?;
        gens.bump_coalesced(Table::Albums);
    }
    for chunk in snapshot.tracks.chunks(100) {
        source
            .upsert_tracks(chunk)
            .await
            .map_err(|e| e.to_string())?;
        gens.bump_coalesced(Table::Tracks);
    }
    for (name, url) in &snapshot.artist_images {
        source
            .set_artist_image(name, "server", Some(url))
            .await
            .map_err(|e| e.to_string())?;
    }

    // Full sync — drop rows the server no longer has.
    let keep_keys: Vec<String> = snapshot
        .tracks
        .iter()
        .map(|t| t.id.key().into_owned())
        .collect();
    let keep_albums: Vec<String> = merged_albums.iter().map(|a| a.id.clone()).collect();
    let _ = source.prune(&keep_keys, &keep_albums).await;
    gens.bump(Table::Tracks);
    gens.bump(Table::Albums);
    info!("Server library sync completed.");
    let _ = clear_first;
    Ok(())
}
