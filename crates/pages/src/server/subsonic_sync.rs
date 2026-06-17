use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use reader::models::Album;
use tracing::info;

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
pub async fn sync_server_library(clear_first: bool) -> Result<(), String> {
    let read_db = consume_context::<hooks::ReadDb>();
    let gens = hooks::db_reactivity::use_generations();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let source = active_source.peek().clone();
    if !source.source().is_server() {
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
