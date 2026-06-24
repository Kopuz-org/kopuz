use reqwest::Client;

use super::auth;
use super::types::*;

const BASE: &str = "https://amp-api.music.apple.com";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct AppleMusicApi {
    http: Client,
    media_user_token: Option<String>,
    storefront: String,
    language: String,
}

impl AppleMusicApi {
    pub fn new(
        media_user_token: Option<String>,
        storefront: impl Into<String>,
        language: impl Into<String>,
    ) -> Self {
        let sf = storefront.into();
        let lang = language.into();
        tracing::debug!(
            "am.new: storefront={sf}, lang={lang}, has_token={}",
            media_user_token.is_some()
        );
        Self {
            http: Client::new(),
            media_user_token,
            storefront: sf,
            language: lang,
        }
    }

    pub fn storefront(&self) -> &str {
        &self.storefront
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn media_user_token(&self) -> Option<&str> {
        self.media_user_token.as_deref()
    }

    async fn get(&self, path: &str) -> Result<reqwest::Response, String> {
        let bearer = auth::get_bearer_token().await?;
        let url = format!("{BASE}{path}");
        let mut req = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("User-Agent", USER_AGENT)
            .header("Origin", "https://music.apple.com")
            .header("Referer", "https://music.apple.com/");

        if let Some(token) = &self.media_user_token {
            req = req.header("Cookie", format!("media-user-token={token}"));
        }

        tracing::debug!("am.get: {url}");
        let resp = req.send().await.map_err(|e| format!("GET {path}: {e}"))?;
        let status = resp.status();
        tracing::debug!("am.get: {path} → {status}");
        if !status.is_success() {
            tracing::warn!("am.get: {path} failed ({status})");
        }
        Ok(resp)
    }

    async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, String> {
        let bearer = auth::get_bearer_token().await?;
        let url = format!("{BASE}{path}");
        let mut req = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("User-Agent", USER_AGENT)
            .header("Origin", "https://music.apple.com")
            .header("Referer", "https://music.apple.com/")
            .header("Content-Type", "application/json")
            .json(body);

        if let Some(token) = &self.media_user_token {
            req = req.header("Cookie", format!("media-user-token={token}"));
        }

        tracing::debug!("am.post: {url}");
        let resp = req.send().await.map_err(|e| format!("POST {path}: {e}"))?;
        let status = resp.status();
        tracing::debug!("am.post: {path} → {status}");
        if !status.is_success() {
            tracing::warn!("am.post: {path} failed ({status})");
        }
        Ok(resp)
    }

    async fn delete(&self, path: &str) -> Result<reqwest::Response, String> {
        let bearer = auth::get_bearer_token().await?;
        let url = format!("{BASE}{path}");
        let mut req = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("User-Agent", USER_AGENT)
            .header("Origin", "https://music.apple.com")
            .header("Referer", "https://music.apple.com/");

        if let Some(token) = &self.media_user_token {
            req = req.header("Cookie", format!("media-user-token={token}"));
        }

        tracing::debug!("am.delete: {url}");
        let resp = req
            .send()
            .await
            .map_err(|e| format!("DELETE {path}: {e}"))?;
        let status = resp.status();
        tracing::debug!("am.delete: {path} → {status}");
        if !status.is_success() {
            tracing::warn!("am.delete: {path} failed ({status})");
        }
        Ok(resp)
    }

    // ── Catalog API (no media-user-token needed) ────────────────────

    pub async fn get_song(&self, id: &str) -> Result<TrackData, String> {
        let path = format!(
            "/v1/catalog/{}/songs/{}?include=albums,artists&extend=extendedAssetUrls&l={}",
            self.storefront, id, self.language
        );
        tracing::debug!("am.get_song: id={id}");
        let resp = self.get(&path).await?;
        if !resp.status().is_success() {
            let err = format!("get_song {id}: HTTP {}", resp.status());
            tracing::warn!("am.get_song: {err}");
            return Err(err);
        }
        let song: SongResp = resp.json().await.map_err(|e| {
            tracing::warn!("am.get_song: parse failed: {e}");
            format!("parse song: {e}")
        })?;
        song.data.into_iter().next().ok_or_else(|| {
            let msg = format!("song {id} not found in response");
            tracing::warn!("am.get_song: {msg}");
            msg
        })
    }

    pub async fn get_album(&self, id: &str) -> Result<AlbumData, String> {
        let path = format!(
            "/v1/catalog/{}/albums/{}?include=tracks,artists&extend=extendedAssetUrls&l={}",
            self.storefront, id, self.language
        );
        tracing::debug!("am.get_album: id={id}");
        let resp = self.get(&path).await?;
        if !resp.status().is_success() {
            let err = format!("get_album {id}: HTTP {}", resp.status());
            tracing::warn!("am.get_album: {err}");
            return Err(err);
        }
        let album: AlbumResp = resp.json().await.map_err(|e| {
            tracing::warn!("am.get_album: parse failed: {e}");
            format!("parse album: {e}")
        })?;
        album.data.into_iter().next().ok_or_else(|| {
            let msg = format!("album {id} not found in response");
            tracing::warn!("am.get_album: {msg}");
            msg
        })
    }

    pub async fn get_playlist(&self, id: &str) -> Result<PlaylistData, String> {
        let path = format!(
            "/v1/catalog/{}/playlists/{}?include=tracks,artists&extend=extendedAssetUrls&l={}",
            self.storefront, id, self.language
        );
        tracing::debug!("am.get_playlist: id={id}");
        let resp = self.get(&path).await?;
        if !resp.status().is_success() {
            let err = format!("get_playlist {id}: HTTP {}", resp.status());
            tracing::warn!("am.get_playlist: {err}");
            return Err(err);
        }
        let pl: PlaylistResp = resp.json().await.map_err(|e| {
            tracing::warn!("am.get_playlist: parse failed: {e}");
            format!("parse playlist: {e}")
        })?;
        pl.data.into_iter().next().ok_or_else(|| {
            let msg = format!("playlist {id} not found in response");
            tracing::warn!("am.get_playlist: {msg}");
            msg
        })
    }

    pub async fn search(
        &self,
        term: &str,
        types: &str,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResp, String> {
        tracing::debug!("am.search: term={term}, types={types}, limit={limit}");
        let path = format!(
            "/v1/catalog/{}/search?term={}&types={}&limit={}&offset={}&l={}",
            self.storefront,
            urlencoding::encode(term),
            urlencoding::encode(types),
            limit,
            offset,
            self.language,
        );
        let resp = self.get(&path).await?;
        if !resp.status().is_success() {
            let err = format!("search: HTTP {}", resp.status());
            tracing::warn!("am.search: {err}");
            return Err(err);
        }
        resp.json().await.map_err(|e| {
            tracing::warn!("am.search: parse failed: {e}");
            format!("parse search: {e}")
        })
    }

    // ── Library API (requires media-user-token) ─────────────────────
    // These use the standard format (no format[resources]=map) where
    // data[] contains full objects with inline attributes/relationships,
    // matching how the Go downloader parses them.

    /// Generic paginated library fetch — returns the full `data` array from
    /// each page, following `next` until exhausted.
    async fn library_page<T: serde::de::DeserializeOwned>(
        &self,
        initial_path: &str,
    ) -> Result<Vec<T>, String> {
        let mut all = Vec::new();
        let mut next = Some(initial_path.to_string());
        let mut page_num = 0u32;
        while let Some(path) = next.take() {
            page_num += 1;
            tracing::info!("am.library_page: page {page_num}, path={path}");
            let resp = self.get(&path).await?;
            if !resp.status().is_success() {
                let err = format!("library page {page_num}: HTTP {}", resp.status());
                tracing::warn!("am.library_page: {err}");
                return Err(err);
            }
            let body = resp.text().await.map_err(|e| {
                tracing::warn!("am.library_page: read body failed page {page_num}: {e}");
                format!("read library page: {e}")
            })?;
            tracing::debug!("am.library_page: page {page_num} body_len={}", body.len());
            let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                tracing::warn!(
                    "am.library_page: parse failed page {page_num}: {e}\nbody (first 2000): {}",
                    &body[..body.len().min(2000)]
                );
                format!("parse library page: {e}")
            })?;
            let data = parsed
                .get("data")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default();
            let count = data.len();
            tracing::info!("am.library_page: page {page_num} — {count} items");
            for item in data {
                match serde_json::from_value::<T>(item) {
                    Ok(v) => all.push(v),
                    Err(e) => {
                        tracing::warn!("am.library_page: deserialize item on page {page_num}: {e}")
                    }
                }
            }
            next = parsed
                .get("next")
                .and_then(|n| n.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            if next.is_none() {
                break;
            }
        }
        tracing::info!("am.library_page: done — {} items total", all.len());
        Ok(all)
    }

    pub async fn get_library_songs(&self) -> Result<Vec<LibrarySongResource>, String> {
        tracing::debug!("am.get_library_songs: starting");
        self.library_page(&format!(
            "/v1/me/library/songs?l={}&limit=100&sort=dateAdded&include=catalog",
            self.language
        ))
        .await
    }

    pub async fn get_library_albums(&self) -> Result<Vec<LibraryAlbumResource>, String> {
        tracing::debug!("am.get_library_albums: starting");
        self.library_page(&format!(
            "/v1/me/library/albums?l={}&limit=100&sort=name",
            self.language
        ))
        .await
    }

    pub async fn get_library_playlists(&self) -> Result<Vec<LibraryPlaylistResource>, String> {
        tracing::debug!("am.get_library_playlists: starting");
        self.library_page(&format!(
            "/v1/me/library/playlists?l={}&limit=100",
            self.language
        ))
        .await
    }

    pub async fn get_library_artists(&self) -> Result<Vec<LibraryArtistResource>, String> {
        tracing::debug!("am.get_library_artists: starting");
        self.library_page(&format!(
            "/v1/me/library/artists?l={}&limit=100&sort=name",
            self.language
        ))
        .await
    }

    /// Fetch tracks of a library playlist using the standard format.
    /// Like the Go code, we fetch the playlist with `include=tracks` and
    /// paginate `relationships.tracks.next`.
    pub async fn get_library_playlist_tracks(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<TrackData>, String> {
        tracing::info!("am.get_library_playlist_tracks: playlist_id={playlist_id}");
        let path = format!(
            "/v1/me/library/playlists/{}?l={}&include=tracks,artists&omit[resource]=autos",
            playlist_id, self.language
        );
        let resp = self.get(&path).await?;
        if !resp.status().is_success() {
            let err = format!("get_library_playlist_tracks: HTTP {}", resp.status());
            tracing::warn!("am.get_library_playlist_tracks: {err}");
            return Err(err);
        }
        let body = resp.text().await.map_err(|e| {
            tracing::warn!("am.get_library_playlist_tracks: read body failed: {e}");
            format!("read playlist: {e}")
        })?;
        tracing::debug!("am.get_library_playlist_tracks: body_len={}", body.len());
        let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            tracing::warn!(
                "am.get_library_playlist_tracks: parse failed: {e}\nbody (first 2000): {}",
                &body[..body.len().min(2000)]
            );
            format!("parse playlist: {e}")
        })?;

        // Extract tracks from relationships.tracks.data
        let mut all = Vec::new();
        let tracks_data = parsed
            .pointer("/data/0/relationships/tracks/data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        tracing::info!(
            "am.get_library_playlist_tracks: {} tracks in first page",
            tracks_data.len()
        );
        for item in tracks_data {
            match serde_json::from_value::<TrackData>(item) {
                Ok(v) => all.push(v),
                Err(e) => tracing::warn!("am.get_library_playlist_tracks: deserialize track: {e}"),
            }
        }

        // Follow pagination via relationships.tracks.next
        let mut next = parsed
            .pointer("/data/0/relationships/tracks/next")
            .and_then(|n| n.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let mut page_num = 1u32;
        while let Some(next_path) = next.take() {
            page_num += 1;
            // Strip absolute prefix so self.get() adds auth headers
            let path = next_path
                .strip_prefix(BASE)
                .unwrap_or(&next_path)
                .to_string();
            tracing::info!("am.get_library_playlist_tracks: page {page_num}, path={path}");
            let resp = self.get(&path).await?;
            if !resp.status().is_success() {
                tracing::warn!(
                    "am.get_library_playlist_tracks: page {page_num} HTTP {}",
                    resp.status()
                );
                break;
            }
            let next_body = resp
                .text()
                .await
                .map_err(|e| format!("read tracks next: {e}"))?;
            let next_parsed: serde_json::Value = serde_json::from_str(&next_body).map_err(|e| {
                tracing::warn!("am.get_library_playlist_tracks: parse page {page_num}: {e}");
                format!("parse tracks next: {e}")
            })?;
            let data = next_parsed
                .get("data")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default();
            tracing::info!(
                "am.get_library_playlist_tracks: page {page_num} — {} tracks",
                data.len()
            );
            for item in data {
                match serde_json::from_value::<TrackData>(item) {
                    Ok(v) => all.push(v),
                    Err(e) => tracing::warn!(
                        "am.get_library_playlist_tracks: deserialize track page {page_num}: {e}"
                    ),
                }
            }
            next = next_parsed
                .get("next")
                .and_then(|n| n.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
        }
        tracing::info!(
            "am.get_library_playlist_tracks: done — {} tracks total",
            all.len()
        );
        Ok(all)
    }

    /// Find the "Favorite Songs" playlist in the user's library.
    /// Apple Music doesn't have a separate favorites endpoint — the
    /// Favorite Songs playlist IS the favorites.
    pub async fn find_favorite_songs_playlist(&self) -> Result<Option<String>, String> {
        tracing::debug!("am.find_favorite_songs: scanning playlists");
        let playlists = self.get_library_playlists().await?;
        tracing::debug!(
            "am.find_favorite_songs: {} playlists to scan",
            playlists.len()
        );
        for pl in &playlists {
            if pl.attributes.name == "Favorite Songs" {
                tracing::debug!("am.find_favorite_songs: found by name — id={}", pl.id);
                return Ok(Some(pl.id.clone()));
            }
            if let Some(tags) = &pl.attributes.tags {
                if tags.iter().any(|t| t == "favorited") {
                    tracing::debug!("am.find_favorite_songs: found by tag — id={}", pl.id);
                    return Ok(Some(pl.id.clone()));
                }
            }
        }
        tracing::warn!(
            "am.find_favorite_songs: no Favorite Songs playlist found among {} playlists",
            playlists.len()
        );
        Ok(None)
    }

    /// Fetch favorited track IDs from the "Favorite Songs" playlist.
    pub async fn get_favorites(&self) -> Result<Vec<String>, String> {
        tracing::debug!("am.get_favorites: starting");
        let Some(playlist_id) = self.find_favorite_songs_playlist().await? else {
            tracing::warn!("am.get_favorites: no Favorite Songs playlist — returning empty");
            return Ok(Vec::new());
        };
        let tracks = self.get_library_playlist_tracks(&playlist_id).await?;
        tracing::debug!("am.get_favorites: {} favorited tracks", tracks.len());
        Ok(tracks.into_iter().map(|t| t.id).collect())
    }

    // ── Library mutations ───────────────────────────────────────────

    pub async fn add_to_library(&self, item_id: &str) -> Result<(), String> {
        tracing::debug!("am.add_to_library: id={item_id}");
        let body = serde_json::json!({
            "id": item_id,
            "type": "songs",
        });
        let resp = self.post("/v1/me/library", &body).await?;
        if !resp.status().is_success() {
            let err = format!("add_to_library: HTTP {}", resp.status());
            tracing::warn!("am.add_to_library: {err}");
            return Err(err);
        }
        tracing::debug!("am.add_to_library: OK");
        Ok(())
    }

    pub async fn remove_from_library(&self, item_id: &str) -> Result<(), String> {
        tracing::debug!("am.remove_from_library: id={item_id}");
        let resp = self
            .delete(&format!("/v1/me/library/songs/{}", item_id))
            .await?;
        if !resp.status().is_success() {
            let err = format!("remove_from_library: HTTP {}", resp.status());
            tracing::warn!("am.remove_from_library: {err}");
            return Err(err);
        }
        tracing::debug!("am.remove_from_library: OK");
        Ok(())
    }

    pub async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, String> {
        tracing::debug!("am.create_playlist: name={name}, items={}", item_refs.len());
        let mut attributes = serde_json::json!({ "name": name });
        if !item_refs.is_empty() {
            let mut tracks = Vec::new();
            for id in item_refs {
                tracks.push(serde_json::json!({ "id": id }));
            }
            attributes["tracks"] = serde_json::json!({ "data": tracks });
        }
        let body = serde_json::json!({
            "attributes": attributes,
        });
        let resp = self.post("/v1/me/library/playlists", &body).await?;
        if !resp.status().is_success() {
            let err = format!("create_playlist: HTTP {}", resp.status());
            tracing::warn!("am.create_playlist: {err}");
            return Err(err);
        }
        let val: serde_json::Value = resp.json().await.map_err(|e| {
            tracing::warn!("am.create_playlist: parse failed: {e}");
            e.to_string()
        })?;
        let id = val["data"][0]["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| {
                let msg = "no id in playlist create response".to_string();
                tracing::warn!("am.create_playlist: {msg}");
                msg
            })?;
        tracing::debug!("am.create_playlist: created id={id}");
        Ok(id)
    }

    pub async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<(), String> {
        tracing::debug!(
            "am.add_to_playlist: playlist={playlist_id}, items={}",
            item_refs.len()
        );
        let mut tracks = Vec::new();
        for id in item_refs {
            tracks.push(serde_json::json!({ "id": id }));
        }
        let body = serde_json::json!({ "data": tracks });
        let resp = self
            .post(
                &format!("/v1/me/library/playlists/{}/tracks", playlist_id),
                &body,
            )
            .await?;
        if !resp.status().is_success() {
            let err = format!("add_to_playlist: HTTP {}", resp.status());
            tracing::warn!("am.add_to_playlist: {err}");
            return Err(err);
        }
        tracing::debug!("am.add_to_playlist: OK");
        Ok(())
    }

    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        tracing::debug!(
            "am.remove_from_playlist: playlist={playlist_id}, tracks={}",
            track_ids.len()
        );
        let resp = self
            .delete(&format!("/v1/me/library/playlists/{}/tracks", playlist_id))
            .await?;
        if !resp.status().is_success() {
            let err = format!("remove_from_playlist: HTTP {}", resp.status());
            tracing::warn!("am.remove_from_playlist: {err}");
            return Err(err);
        }
        tracing::debug!("am.remove_from_playlist: OK");
        Ok(())
    }

    pub async fn validate(&self) -> Result<(), String> {
        let Some(token) = self.media_user_token.as_deref() else {
            tracing::warn!("am.validate: no media user token stored");
            return Err("no media user token".to_string());
        };
        let bearer = match auth::get_bearer_token().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("am.validate: bearer token fetch failed: {e}");
                return Err(e);
            }
        };
        tracing::debug!(
            "am.validate: token_len={}, bearer_len={}",
            token.len(),
            bearer.len()
        );
        let resp = self
            .http
            .get(format!(
                "{BASE}/v1/me/library/songs?l={}&limit=1&platform=web",
                self.language
            ))
            .header("Authorization", format!("Bearer {bearer}"))
            .header("User-Agent", USER_AGENT)
            .header("Origin", "https://music.apple.com")
            .header("Referer", "https://music.apple.com/")
            .header("Cookie", format!("media-user-token={token}"))
            .send()
            .await
            .map_err(|e| format!("validate: {e}"))?;
        let status = resp.status();
        if status.is_success() {
            tracing::debug!("am.validate: OK");
            Ok(())
        } else if status.as_u16() == 401 {
            tracing::warn!("am.validate: 401 Unauthorized — token likely expired");
            Err("expired".to_string())
        } else {
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!("am.validate: HTTP {status} — {body}");
            Err(format!("HTTP {status}"))
        }
    }
}
