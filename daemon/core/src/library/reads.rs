//! Library reads beyond the track list: albums, artists, genres, recents and
//! search.
//!
//! Every one of these was a `ReadDb` call in the app's query hooks. They page
//! in the daemon so a frontend never holds the whole library, and they answer
//! with wire rows, so cover paths and local filesystem paths stay here.

use api::{
    AlbumInfo, AlbumPage, AlbumQuery, ApiError, ArtistInfo, ArtistPage, ArtistQuery, Page,
    SearchResults, TrackPage,
};
use reader::Album;

use super::{LibraryService, album_query, artist_query, db_error, db_page, offline_filter};

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

/// An album row as the wire carries it, keyed to the artist it bills.
pub(crate) fn album_info(album: &Album) -> AlbumInfo {
    AlbumInfo {
        id: album.id.clone(),
        title: album.title.clone(),
        artist_key: album.artist_key.clone(),
        artist: album.artist.clone(),
        genre: album.genre.clone(),
        year: album.year,
        artwork: crate::artwork::album_ref(album),
    }
}

struct ArtistArt {
    images: db::ArtistImages,
    covers: std::collections::HashMap<String, reader::CoverRef>,
    source: config::Source,
}

impl ArtistArt {
    fn info(&self, artist: db::ArtistRow) -> ArtistInfo {
        ArtistInfo {
            key: artist.key.clone(),
            artwork: crate::artwork::artist_ref(
                &artist,
                &self.source,
                &self.images,
                self.covers.get(&artist.key),
            ),
            name: artist.name,
            track_count: artist.tracks,
            album_count: artist.albums,
        }
    }
}

impl LibraryService {
    pub async fn tracks_by_keys(&self, keys: &[String]) -> Result<Vec<api::TrackInfo>, ApiError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let offline = self.offline();
        let mut rows = self
            .db
            .tracks_by_keys(&self.query_source(), keys)
            .await
            .map_err(db_error)?;
        self.keep_offline(&mut rows);
        Ok(rows
            .iter()
            .map(|track| crate::wire::track_info(track, &offline))
            .collect())
    }

    /// A public page for a track: the source's own where it has one, else the
    /// MusicBrainz page its metadata names. A row the library has never stored
    /// still answers, because a catalog hit is exactly the row someone wants
    /// to share.
    pub async fn track_web_url(&self, key: &str) -> Result<Option<String>, ApiError> {
        let sources = self.current_sources();
        let track = match self
            .db
            .tracks_by_keys(&self.query_source(), &[key.to_string()])
            .await
            .map_err(db_error)?
            .into_iter()
            .next()
        {
            Some(track) => track,
            None => match self.transient_track(key) {
                Some(track) => track,
                None => return Ok(None),
            },
        };
        if let Some(url) = server::source::active(self.db.clone(), &sources).web_url(&track) {
            return Ok(Some(url));
        }
        Ok(server::musicbrainz::track_page_url(
            track.musicbrainz_release_id.as_deref(),
            &track.artist,
            &track.title,
        )
        .await)
    }

    /// The source's page for an album, falling back to its first track's page
    /// for a source that has albums but no album pages.
    pub async fn album_web_url(&self, id: &str) -> Result<Option<String>, ApiError> {
        let sources = self.current_sources();
        let source = server::source::active(self.db.clone(), &sources);
        if let Some(url) = source.album_web_url(id) {
            return Ok(Some(url));
        }
        let track = self
            .db
            .album_tracks(&self.query_source(), id)
            .await
            .map_err(db_error)?
            .into_iter()
            .next();
        Ok(track.and_then(|track| source.web_url(&track)))
    }

    pub async fn albums(&self, query: AlbumQuery, page: Page) -> Result<AlbumPage, ApiError> {
        let Some(offline) = offline_filter(query.offline, self.offline_reads()) else {
            return Ok(AlbumPage::default());
        };
        let query = album_query(self.query_source(), query, offline);
        let listing = self
            .db
            .albums_page(&query, db_page(page))
            .await
            .map_err(db_error)?;
        Ok(AlbumPage {
            albums: listing.rows.iter().map(album_info).collect(),
            total: listing.total,
        })
    }

    pub async fn albums_recently_added(&self, page: Page) -> Result<AlbumPage, ApiError> {
        if self.offline_reads() {
            let newest = AlbumQuery {
                sort: vec![config::SortCriterion::new(
                    config::AlbumSortField::RecentlyAdded,
                    config::SortDirection::Desc,
                )],
                ..AlbumQuery::default()
            };
            return self.albums(newest, page).await;
        }
        // The store applies the recency order and the cut, so it has to see the
        // whole window the caller is paging within, not just the page length.
        let depth = page.offset.saturating_add(page.limit);
        let source = self.query_source();
        let rows = self
            .db
            .albums_recently_added(&source, depth)
            .await
            .map_err(db_error)?;
        let (total, items) = window(&rows, page);
        Ok(AlbumPage {
            albums: items.iter().map(album_info).collect(),
            total,
        })
    }

    pub async fn album(&self, id: &str) -> Result<Option<AlbumInfo>, ApiError> {
        let source = self.query_source();
        let album = self.db.album(&source, id).await.map_err(db_error)?;
        Ok(album.as_ref().map(album_info))
    }

    pub async fn album_tracks(&self, id: &str, page: Page) -> Result<TrackPage, ApiError> {
        let offline = self.offline();
        let mut rows = self
            .db
            .album_tracks(&self.query_source(), id)
            .await
            .map_err(db_error)?;
        self.keep_offline(&mut rows);
        let (total, items) = window(&rows, page);
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: items
                .iter()
                .map(|track| crate::wire::track_info(track, &offline))
                .collect(),
        })
    }

    /// The artist grid. Its artwork chain ends in "one of this artist's album
    /// covers", so the covers and photos are loaded once for the page rather
    /// than per row -- the same walk `ArtworkService` does, on bulk data.
    pub async fn artists(&self, query: ArtistQuery, page: Page) -> Result<ArtistPage, ApiError> {
        let Some(offline) = offline_filter(query.offline, self.offline_reads()) else {
            return Ok(ArtistPage::default());
        };
        let query = artist_query(self.query_source(), query, offline);
        let listing = self
            .db
            .artists_page(&query, db_page(page))
            .await
            .map_err(db_error)?;
        let art = self.artist_art(None).await?;
        Ok(ArtistPage {
            artists: listing.rows.into_iter().map(|row| art.info(row)).collect(),
            total: listing.total,
        })
    }

    /// The library row a key names in the source being read.
    pub(crate) async fn artist_row(&self, artist: &str) -> Result<db::ArtistRow, ApiError> {
        crate::artist_row::require(&self.db, &self.query_source(), artist).await
    }

    /// One artist's header; its albums list through `albums` by artist key.
    pub async fn artist(&self, artist: &str) -> Result<api::ArtistDetail, ApiError> {
        let row = self.artist_row(artist).await?;
        let art = self.artist_art(Some(&row.key)).await?;
        Ok(api::ArtistDetail {
            info: art.info(row),
        })
    }

    /// The photos and fallback covers an artist's artwork chain reads: for every artist, or just `one`.
    async fn artist_art(&self, one: Option<&str>) -> Result<ArtistArt, ApiError> {
        let source = self.query_source();
        let covers = match one {
            None => self
                .db
                .artist_fallback_covers(&source)
                .await
                .map_err(db_error)?,
            Some(key) => self
                .db
                .artist_fallback_cover(&source, key)
                .await
                .map_err(db_error)?
                .map(|cover| (key.to_string(), cover))
                .into_iter()
                .collect(),
        };
        Ok(ArtistArt {
            images: self.db.artist_images().await.map_err(db_error)?,
            covers,
            source,
        })
    }

    pub async fn artist_tracks(&self, artist: &str, page: Page) -> Result<TrackPage, ApiError> {
        let offline = self.offline();
        let row = self.artist_row(artist).await?;
        let mut rows = self
            .db
            .artist_tracks(&self.query_source(), &row.key, None)
            .await
            .map_err(db_error)?;
        self.keep_offline(&mut rows);
        let (total, items) = window(&rows, page);
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: items
                .iter()
                .map(|track| crate::wire::track_info(track, &offline))
                .collect(),
        })
    }

    pub async fn artist_sample_tracks(&self, page: Page) -> Result<TrackPage, ApiError> {
        let offline = self.offline();
        let mut rows = self
            .db
            .artist_sample_tracks(&self.query_source(), page.limit)
            .await
            .map_err(db_error)?;
        self.keep_offline(&mut rows);
        let total = rows.len() as u32;
        Ok(TrackPage {
            total,
            offset: 0,
            items: rows
                .iter()
                .map(|track| crate::wire::track_info(track, &offline))
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
        let offline = self.offline();
        let mut rows = self
            .db
            .genre_tracks(&self.query_source(), genre)
            .await
            .map_err(db_error)?;
        self.keep_offline(&mut rows);
        let (total, items) = window(&rows, page);
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: items
                .iter()
                .map(|track| crate::wire::track_info(track, &offline))
                .collect(),
        })
    }

    pub async fn recent_tracks(&self, page: Page) -> Result<TrackPage, ApiError> {
        let offline = self.offline();
        let source = self.query_source();
        // Recents are a key list; the rows come back unordered, so they are
        // put back into play order here.
        let keys = self
            .db
            .recently_played(&source, page.offset.saturating_add(page.limit))
            .await
            .map_err(db_error)?;
        let mut rows = self
            .db
            .tracks_by_keys(&source, &keys)
            .await
            .map_err(db_error)?;
        self.keep_offline(&mut rows);
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
                .map(|track| crate::wire::track_info(track, &offline))
                .collect(),
        })
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults, ApiError> {
        if self.offline_reads() {
            return self.search_offline(query).await;
        }
        let sources = self.current_sources();
        let offline = self.offline();
        // A remote source answers over the network, so search is the one read
        // here that goes through the source rather than straight to the DB.
        let source = server::source::active(self.db.clone(), &sources);
        let (mut tracks, albums) = source
            .search(query)
            .await
            .map_err(|error| ApiError::internal(format!("search failed: {error}")))?;
        crate::wire::listed_by(source.source(), &mut tracks);
        // A remote hit may name a track the library has never stored, so
        // remember it: the caller gets a key, and queueing or hearting that
        // key has to resolve to something.
        self.register_transient(&tracks);
        Ok(SearchResults {
            tracks: tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &offline))
                .collect(),
            albums: albums.iter().map(album_info).collect(),
            ..Default::default()
        })
    }

    /// Search while offline mode keeps to the copies here: the library's own rows with one, never the source.
    async fn search_offline(&self, query: &str) -> Result<SearchResults, ApiError> {
        let tracks = self
            .tracks(
                api::TrackFilter {
                    search: Some(query.to_string()),
                    ..api::TrackFilter::default()
                },
                Page::default(),
            )
            .await?;
        let albums = self
            .albums(
                AlbumQuery {
                    search: Some(query.to_string()),
                    ..AlbumQuery::default()
                },
                Page::default(),
            )
            .await?;
        Ok(SearchResults {
            tracks: tracks.items,
            albums: albums.albums,
            ..Default::default()
        })
    }
}
