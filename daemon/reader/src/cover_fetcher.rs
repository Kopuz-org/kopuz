use crate::cover_indexer::missing_cover_ids;
use crate::models::Library;
use crate::utils::save_cover;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing;

#[derive(Debug, Default)]
pub struct FetchReport {
    pub found: usize,
    pub missing: usize,
    pub errors: usize,
}

pub struct CoverFetcher {
    client: reqwest::Client,
    cache_dir: PathBuf,
    on_progress: Arc<dyn Fn(String) + Send + Sync>,
}

impl CoverFetcher {
    pub fn new(cache_dir: PathBuf, on_progress: Arc<dyn Fn(String) + Send + Sync>) -> Self {
        let client = ::utils::http::builder()
            .user_agent(concat!(
                "Kopuz/",
                env!("CARGO_PKG_VERSION"),
                " (music-player)"
            ))
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self {
            client,
            cache_dir,
            on_progress,
        }
    }

    pub async fn fetch_missing_covers(&self, library: &mut Library) -> FetchReport {
        let mut report = FetchReport::default();
        let missing_ids = missing_cover_ids(library);
        let missing: Vec<usize> = library
            .albums
            .iter()
            .enumerate()
            .filter(|(_, album)| {
                missing_ids.contains(&album.id)
                    && album.title != "Unknown Album"
                    && !crate::metadata::is_untitled_album(&album.id)
            })
            .map(|(i, _)| i)
            .collect();

        if missing.is_empty() {
            tracing::info!("Cover fetcher: no missing covers to fetch");
            return report;
        }

        tracing::info!("Cover fetcher: fetching {} missing covers", missing.len());

        for &idx in &missing {
            let artist = library.albums[idx].artist.clone();
            let title = library.albums[idx].title.clone();
            let album_id = library.albums[idx].id.clone();
            tracing::info!("Cover fetcher: fetching {} — {}", artist, title);
            (self.on_progress)(format!("Fetching cover: {} — {}", artist, title));

            let release_id = library
                .tracks
                .iter()
                .filter(|t| t.album_id == album_id)
                .find_map(|t| t.musicbrainz_release_id.as_deref());

            let result = self.fetch_cover(release_id, &title, &artist).await;

            match result {
                Some(img_data) => match save_cover(&album_id, &img_data, None, &self.cache_dir) {
                    Ok(saved_path) => {
                        library.albums[idx].cover_path = Some(saved_path);
                        report.found += 1;
                        tracing::info!(
                            "Cover fetcher: successfully found and saved cover for {} — {}",
                            artist,
                            title,
                        );
                    }
                    Err(e) => {
                        report.errors += 1;
                        tracing::warn!(
                            "Cover fetcher: failed to save cover for {} — {}: {}",
                            artist,
                            title,
                            e,
                        );
                    }
                },
                None => {
                    report.missing += 1;
                    tracing::info!("Cover fetcher: no cover found for {} — {}", artist, title);
                }
            }
        }

        tracing::info!(
            "Cover fetcher: done — {} found, {} missing, {} errors",
            report.found,
            report.missing,
            report.errors,
        );
        report
    }

    async fn fetch_cover(
        &self,
        release_id: Option<&str>,
        album: &str,
        artist: &str,
    ) -> Option<Vec<u8>> {
        self.try_musicbrainz(release_id, album, artist).await
    }

    async fn try_musicbrainz(
        &self,
        release_id: Option<&str>,
        album: &str,
        artist: &str,
    ) -> Option<Vec<u8>> {
        let mbid = match release_id {
            Some(id) => {
                tracing::info!(
                    "Cover fetcher: using provided MusicBrainz Release ID: {}",
                    id
                );
                id.to_string()
            }
            None => {
                tracing::info!(
                    "Cover fetcher: no release ID, searching MusicBrainz for album \"{}\" by artist \"{}\"",
                    album,
                    artist
                );
                let found = self.search_musicbrainz_release(album, artist).await?;
                tracing::info!(
                    "Cover fetcher: MusicBrainz search returned Release ID: {}",
                    found
                );
                found
            }
        };

        self.sleep_rate_limit().await;
        let resp = self
            .client
            .get(format!("https://coverartarchive.org/release/{mbid}"))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        #[derive(Deserialize)]
        struct CoverArchiveResponse {
            images: Vec<CoverArchiveImage>,
        }
        #[derive(Deserialize)]
        struct CoverArchiveImage {
            image: String,
            #[serde(default)]
            front: bool,
            #[serde(default)]
            types: Vec<String>,
        }

        let body: CoverArchiveResponse = resp.json().await.ok()?;
        let url = body
            .images
            .iter()
            .find(|i| i.front || i.types.iter().any(|t| t == "Front"))
            .or_else(|| body.images.first())?;

        self.client
            .get(&url.image)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .bytes()
            .await
            .ok()
            .map(|b| b.to_vec())
    }

    async fn search_musicbrainz_release(&self, album: &str, artist: &str) -> Option<String> {
        let (esc_album, esc_artist) = (
            album.replace('\\', "\\\\").replace('"', "\\\""),
            artist.replace('\\', "\\\\").replace('"', "\\\""),
        );
        let query = if artist.is_empty() || artist == "Unknown Artist" {
            format!("release:\"{}\"", esc_album)
        } else {
            format!("release:\"{}\" AND artist:\"{}\"", esc_album, esc_artist)
        };

        self.sleep_rate_limit().await;
        let resp = self
            .client
            .get("https://musicbrainz.org/ws/2/release/")
            .query(&[("query", query.as_str()), ("fmt", "json")])
            .send()
            .await
            .ok()?;

        #[derive(Deserialize)]
        struct SearchResponse {
            releases: Vec<Release>,
        }
        #[derive(Deserialize)]
        struct Release {
            id: String,
            score: u32,
        }

        let body: SearchResponse = resp.json().await.ok()?;
        body.releases
            .into_iter()
            .find(|r| r.score >= 80)
            .map(|r| r.id)
    }

    async fn sleep_rate_limit(&self) {
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
}
