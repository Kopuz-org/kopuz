//! LibraryService: database-backed track reads and queue materialization.
//!
//! First slice of the daemon's library ownership: read-only queries plus the
//! [`QueueMaterializer`] impl, so "play this album" resolves inside the daemon
//! and the track list never round-trips through a client. Scan, sync, and
//! write paths move in with the job runner.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use api::{ApiError, Page, QueueContext, TrackFilter, TrackPage};
use reader::Track;

use crate::session::QueueMaterializer;

pub struct LibraryService {
    db: db::ReadDb,
    source: config::Source,
    station_registry: Arc<radio::registry::StationRegistry>,
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

fn map_sort(sort: Option<&str>) -> db::TrackSort {
    match sort {
        Some("title") => db::TrackSort::Title,
        Some("artist") => db::TrackSort::Artist,
        Some("album") => db::TrackSort::Album,
        Some("date_added") => db::TrackSort::DateAdded,
        Some("play_count") => db::TrackSort::PlayCount,
        _ => db::TrackSort::ArtistAlbum,
    }
}

fn matches_search(track: &Track, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    [&track.title, &track.artist, &track.album]
        .into_iter()
        .any(|field| field.to_lowercase().contains(&needle))
}

impl LibraryService {
    pub fn new(
        db: db::ReadDb,
        source: config::Source,
        station_registry: Arc<radio::registry::StationRegistry>,
    ) -> Self {
        Self {
            db,
            source,
            station_registry,
        }
    }

    pub async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError> {
        if filter.favorite.is_some() {
            return Err(ApiError::unsupported(
                "favorite filtering lands with the favorites service",
            ));
        }

        let narrowed = if let Some(album) = filter.album.as_deref() {
            Some(
                self.db
                    .album_tracks(&self.source, album)
                    .await
                    .map_err(db_error)?,
            )
        } else if let Some(artist) = filter.artist.as_deref() {
            Some(
                self.db
                    .artist_tracks(&self.source, artist, None)
                    .await
                    .map_err(db_error)?,
            )
        } else if let Some(genre) = filter.genre.as_deref() {
            Some(
                self.db
                    .genre_tracks(&self.source, genre)
                    .await
                    .map_err(db_error)?,
            )
        } else {
            None
        };

        if let Some(mut rows) = narrowed {
            if let Some(search) = filter.search.as_deref().filter(|s| !s.is_empty()) {
                rows.retain(|track| matches_search(track, search));
            }
            let total = rows.len() as u32;
            let items = rows
                .into_iter()
                .skip(page.offset as usize)
                .take(page.limit as usize)
                .collect();
            return Ok(TrackPage {
                total,
                offset: page.offset,
                items,
            });
        }

        let db_filter = db::TrackFilter {
            source: self.source.clone(),
            sort: map_sort(filter.sort.as_deref()),
            search: filter.search.unwrap_or_default(),
        };
        let items = self
            .db
            .tracks_page(
                &db_filter,
                db::Page {
                    offset: page.offset,
                    limit: page.limit,
                },
            )
            .await
            .map_err(db_error)?;
        let total = self.db.tracks_count(&db_filter).await.map_err(db_error)?;
        Ok(TrackPage {
            total,
            offset: page.offset,
            items,
        })
    }

    /// Synthetic radio track, seeded from the manifest so no client ever sees
    /// raw ids while the first metadata update is in flight. The `u64::MAX`
    /// duration sentinel is translated to `TrackKind::Radio` at the wire.
    fn radio_track(&self, station_id: &str, stream_id: &str) -> Track {
        let station = self.station_registry.get(station_id);
        let title = station
            .map(|station| station.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| stream_id.to_string());
        let artist = station
            .and_then(|station| match &station.metadata {
                Some(radio::manifest::MetadataSourceDef::Static(meta)) => {
                    Some(meta.resolve(stream_id).1.to_string())
                }
                _ => None,
            })
            .or_else(|| {
                station
                    .and_then(|station| {
                        station.streams.iter().find(|stream| stream.id == stream_id)
                    })
                    .map(|stream| stream.name.clone())
            })
            .filter(|artist| !artist.trim().is_empty())
            .unwrap_or_else(|| "Live Radio".to_string());

        Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(format!(
                "radio:{station_id}:{stream_id}"
            ))),
            cover: None,
            album_id: String::new(),
            title,
            artist,
            album: "Live Radio".to_string(),
            duration: u64::MAX,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
        }
    }

    /// Keys the database does not know but that exist as local audio files are
    /// probed directly, so ad-hoc file playback keeps working alongside the
    /// library.
    async fn probe_local_files(keys: Vec<String>) -> Vec<Track> {
        if keys.is_empty() {
            return Vec::new();
        }
        tokio::task::spawn_blocking(move || {
            let cover_cache = std::env::temp_dir();
            let mut library = reader::Library::default();
            keys.iter()
                .filter_map(|key| {
                    let path = Path::new(key);
                    path.is_file()
                        .then(|| reader::read(path, &cover_cache, &mut library))
                        .flatten()
                })
                .collect()
        })
        .await
        .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl QueueMaterializer for LibraryService {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        match context {
            QueueContext::Tracks { keys } => {
                let known = self
                    .db
                    .tracks_by_keys(&self.source, keys)
                    .await
                    .map_err(db_error)?;
                let mut by_key: HashMap<String, Track> = known
                    .into_iter()
                    .map(|track| (track.id.key().to_string(), track))
                    .collect();
                let missing: Vec<String> = keys
                    .iter()
                    .filter(|key| !by_key.contains_key(*key))
                    .cloned()
                    .collect();
                for track in Self::probe_local_files(missing).await {
                    by_key.insert(track.id.key().to_string(), track);
                }
                Ok(keys.iter().filter_map(|key| by_key.remove(key)).collect())
            }
            QueueContext::Album { id } => self
                .db
                .album_tracks(&self.source, id)
                .await
                .map_err(db_error),
            QueueContext::Artist { name } => self
                .db
                .artist_tracks(&self.source, name, None)
                .await
                .map_err(db_error),
            QueueContext::Genre { name } => self
                .db
                .genre_tracks(&self.source, name)
                .await
                .map_err(db_error),
            QueueContext::Playlist { id } => {
                let store = self
                    .db
                    .load_playlists(&self.source)
                    .await
                    .map_err(db_error)?;
                let playlist = store
                    .playlists
                    .iter()
                    .find(|playlist| playlist.id == *id)
                    .ok_or_else(|| ApiError::not_found("playlist not found"))?;
                self.db
                    .tracks_by_keys(&self.source, &playlist.tracks)
                    .await
                    .map_err(db_error)
            }
            QueueContext::Filter { filter } => Ok(self
                .tracks(
                    filter.clone(),
                    Page {
                        offset: 0,
                        limit: u32::MAX,
                    },
                )
                .await?
                .items),
            QueueContext::Radio {
                station_id,
                stream_id,
            } => Ok(vec![self.radio_track(station_id, stream_id)]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(n: usize, artist: &str) -> Track {
        Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(format!("/lib/{n}.flac"))),
            cover: None,
            album_id: format!("album-{}", n % 2),
            title: format!("song {n}"),
            artist: artist.to_string(),
            album: format!("album {}", n % 2),
            duration: 60,
            khz: 44,
            bitrate: 320,
            track_number: Some(n as u32),
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
        }
    }

    async fn seeded_library() -> (tempfile::TempDir, LibraryService) {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("test.db"))
            .await
            .expect("db init");
        let source = config::Source::default();
        let tracks: Vec<Track> = (0..5)
            .map(|n| track(n, if n < 3 { "Ada" } else { "Boris" }))
            .collect();
        database
            .upsert_tracks(&source, &tracks)
            .await
            .expect("seed tracks");
        let service = LibraryService::new(
            database.reads(),
            source,
            Arc::new(radio::registry::StationRegistry::default()),
        );
        (dir, service)
    }

    #[tokio::test]
    async fn tracks_pages_and_searches_the_database() {
        let (_dir, library) = seeded_library().await;

        let page = library
            .tracks(
                TrackFilter::default(),
                Page {
                    offset: 0,
                    limit: 2,
                },
            )
            .await
            .expect("page");
        assert_eq!(page.total, 5);
        assert_eq!(page.items.len(), 2);

        let page = library
            .tracks(
                TrackFilter {
                    search: Some("song 4".into()),
                    ..Default::default()
                },
                Page::default(),
            )
            .await
            .expect("search");
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].title, "song 4");

        let page = library
            .tracks(
                TrackFilter {
                    artist: Some("Ada".into()),
                    ..Default::default()
                },
                Page::default(),
            )
            .await
            .expect("artist listing");
        assert_eq!(page.total, 3);
    }

    #[tokio::test]
    async fn materialize_resolves_database_contexts() {
        let (_dir, library) = seeded_library().await;

        let tracks = library
            .materialize(&QueueContext::Album {
                id: "album-1".into(),
            })
            .await
            .expect("album context");
        assert_eq!(tracks.len(), 2);

        let tracks = library
            .materialize(&QueueContext::Tracks {
                keys: vec!["/lib/2.flac".into(), "/lib/0.flac".into(), "/nope".into()],
            })
            .await
            .expect("keys context");
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "song 2");
        assert_eq!(tracks[1].title, "song 0");

        let missing = library
            .materialize(&QueueContext::Playlist { id: "ghost".into() })
            .await
            .expect_err("unknown playlist");
        assert_eq!(missing.code, api::ErrorCode::NotFound);

        let radio = library
            .materialize(&QueueContext::Radio {
                station_id: "st".into(),
                stream_id: "hi".into(),
            })
            .await
            .expect("radio context");
        assert_eq!(radio[0].duration, u64::MAX);
        assert_eq!(radio[0].title, "hi");
    }
}
