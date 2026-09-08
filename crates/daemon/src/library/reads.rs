//! Library reads beyond the track list: albums, artists, genres, recents and
//! search.
//!
//! Every one of these was a `ReadDb` call in the app's query hooks. They page
//! in the daemon so a frontend never holds the whole library, and they answer
//! with wire rows, so cover paths and local filesystem paths stay here.

use api::{
    AlbumInfo, AlbumPage, ApiError, ArtistInfo, ArtistPage, ArtworkTarget, Page, SearchResults,
    TrackPage,
};
use reader::Album;

use super::{LibraryService, db_error};

/// Paging a fully materialized list. The database returns whole collections
/// for these (they are small next to the track table), so the window is
/// applied here rather than pushed into SQL.
fn window<T: Clone>(rows: &[T], page: Page) -> (u32, Vec<T>) {
    let total = rows.len() as u32;
    let items = rows
        .iter()
        .skip(page.offset as usize)
        .take(page.limit as usize)
        .cloned()
        .collect();
    (total, items)
}

fn album_info(album: &Album) -> AlbumInfo {
    AlbumInfo {
        id: album.id.clone(),
        title: album.title.clone(),
        artist: album.artist.clone(),
        genre: album.genre.clone(),
        year: album.year,
        artwork: Some(ArtworkTarget::Album(album.id.clone())),
    }
}

impl LibraryService {
    pub async fn tracks_by_keys(&self, keys: &[String]) -> Result<Vec<api::TrackInfo>, ApiError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let config = self.current_config();
        let rows = self
            .db
            .tracks_by_keys(&self.query_source(), keys)
            .await
            .map_err(db_error)?;
        Ok(rows
            .iter()
            .map(|track| crate::wire::track_info(track, &config))
            .collect())
    }

    pub async fn albums(&self, page: Page) -> Result<AlbumPage, ApiError> {
        let rows = self
            .db
            .albums(&self.query_source())
            .await
            .map_err(db_error)?;
        let (total, items) = window(&rows, page);
        Ok(AlbumPage {
            albums: items.iter().map(album_info).collect(),
            total,
        })
    }

    pub async fn album(&self, id: &str) -> Result<Option<AlbumInfo>, ApiError> {
        let album = self
            .db
            .album(&self.query_source(), id)
            .await
            .map_err(db_error)?;
        Ok(album.as_ref().map(album_info))
    }

    pub async fn album_tracks(&self, id: &str, page: Page) -> Result<TrackPage, ApiError> {
        let config = self.current_config();
        let rows = self
            .db
            .album_tracks(&self.query_source(), id)
            .await
            .map_err(db_error)?;
        let (total, items) = window(&rows, page);
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: items
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn artists(&self, page: Page) -> Result<ArtistPage, ApiError> {
        let rows = self
            .db
            .artists(&self.query_source())
            .await
            .map_err(db_error)?;
        let (total, items) = window(&rows, page);
        Ok(ArtistPage {
            artists: items
                .into_iter()
                .map(|(name, track_count)| ArtistInfo {
                    artwork: Some(ArtworkTarget::Artist(name.clone())),
                    name,
                    track_count,
                })
                .collect(),
            total,
        })
    }

    pub async fn artist_tracks(&self, artist: &str, page: Page) -> Result<TrackPage, ApiError> {
        let config = self.current_config();
        let rows = self
            .db
            .artist_tracks(&self.query_source(), artist, None)
            .await
            .map_err(db_error)?;
        let (total, items) = window(&rows, page);
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: items
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn artist_sample_tracks(&self, page: Page) -> Result<TrackPage, ApiError> {
        let config = self.current_config();
        let rows = self
            .db
            .artist_sample_tracks(&self.query_source(), page.limit)
            .await
            .map_err(db_error)?;
        let total = rows.len() as u32;
        Ok(TrackPage {
            total,
            offset: 0,
            items: rows
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn genres(&self) -> Result<Vec<String>, ApiError> {
        self.db.genres(&self.query_source()).await.map_err(db_error)
    }

    pub async fn top_genre(&self) -> Result<Option<String>, ApiError> {
        self.db
            .top_genre(&self.query_source())
            .await
            .map_err(db_error)
    }

    pub async fn genre_tracks(&self, genre: &str, page: Page) -> Result<TrackPage, ApiError> {
        let config = self.current_config();
        let rows = self
            .db
            .genre_tracks(&self.query_source(), genre)
            .await
            .map_err(db_error)?;
        let (total, items) = window(&rows, page);
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: items
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn recent_tracks(&self, page: Page) -> Result<TrackPage, ApiError> {
        let config = self.current_config();
        let source = self.query_source();
        // Recents are a key list; the rows come back unordered, so they are
        // put back into play order here.
        let keys = self
            .db
            .recently_played(&source, page.offset.saturating_add(page.limit))
            .await
            .map_err(db_error)?;
        let rows = self
            .db
            .tracks_by_keys(&source, &keys)
            .await
            .map_err(db_error)?;
        let ordered: Vec<_> = keys
            .iter()
            .filter_map(|key| rows.iter().find(|track| track.id.key() == key.as_str()))
            .collect();
        let total = ordered.len() as u32;
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: ordered
                .into_iter()
                .skip(page.offset as usize)
                .take(page.limit as usize)
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults, ApiError> {
        let config = self.current_config();
        // A remote source answers over the network, so search is the one read
        // here that goes through the source rather than straight to the DB.
        let source = server::source::active(self.db.clone(), &config);
        let (tracks, albums) = source
            .search(query)
            .await
            .map_err(|error| ApiError::internal(format!("search failed: {error}")))?;
        Ok(SearchResults {
            tracks: tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
            albums: albums.iter().map(album_info).collect(),
        })
    }
}
