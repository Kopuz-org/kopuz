use reqwest::Client;

use super::auth;
use super::types::*;

const BASE: &str = "https://amp-api.music.apple.com";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Playlist track references. Apple requires the resource `type` alongside the
/// id; without it the request is accepted and the tracks silently dropped.
fn track_refs(item_refs: &[String]) -> Vec<serde_json::Value> {
    item_refs
        .iter()
        .map(|id| serde_json::json!({ "id": id, "type": "songs" }))
        .collect()
}

// The `me` endpoints take their arguments as `ids[type]=` query parameters,
// percent-encoded as Apple's own client sends them. They're built here, apart
// from the requests, because nothing short of mutating a real library can check
// them at runtime — so the tests check them here instead.

fn favorites_path(item_id: &str) -> String {
    format!("/v1/me/favorites?ids%5Bsongs%5D={item_id}")
}

fn library_add_path(item_id: &str) -> String {
    format!("/v1/me/library?ids%5Bsongs%5D={item_id}&representation=ids")
}

fn playlist_add_path(playlist_id: &str) -> String {
    format!("/v1/me/library/playlists/{playlist_id}/tracks?representation=resources")
}

fn playlist_entry_delete_path(playlist_id: &str, entry_id: &str) -> String {
    format!(
        "/v1/me/library/playlists/{playlist_id}/tracks?ids%5Blibrary-songs%5D={entry_id}&mode=all"
    )
}

/// Asks for the `tags` attribute, which is how Apple marks its own playlists.
///
/// It has to be requested by resource type: a plain `extend=tags` is accepted
/// and silently ignored, which is why the tag looked unavailable and the
/// favorites playlist ended up being matched by its English name instead. This
/// is the form Apple's own web client sends.
const PLAYLIST_TAGS_EXTEND: &str = "extend%5Blibrary-playlists%5D=tags";

/// The tag Apple puts on the favorites playlist, and on nothing else.
const FAVORITES_TAG: &str = "favorited";

/// The English name of the favorites playlist. A fallback only: Apple localises
/// it, so `l=fr` returns "Morceaux préférés" and matching on the name alone
/// loses every favorite outside English.
const FAVORITES_NAME_EN: &str = "Favorite Songs";

/// Whether this is the playlist Apple keeps the user's favorites in.
///
/// The tag is Apple's own marker and the only locale-independent one — there is
/// no identity filter on the endpoint, and the name is translated.
fn is_favorites_playlist(attributes: &LibraryPlaylistAttributes) -> bool {
    attributes
        .tags
        .as_ref()
        .is_some_and(|tags| tags.iter().any(|tag| tag == FAVORITES_TAG))
}

/// Relationships for the songs *inside* a playlist.
///
/// A plain `include=` on the playlist request applies to the playlist, so its
/// tracks come back with no relationships at all however many values are listed
/// there — the entries have to be asked for by resource type. Without this a
/// playlist track has no album, and therefore no genre.
const PLAYLIST_TRACK_INCLUDE: &str = "include%5Blibrary-songs%5D=albums,artists";

/// The same relationships, as the `…/tracks` endpoint the pagination cursor
/// points at wants them — that endpoint returns songs directly, so it takes a
/// plain `include`. Adding `catalog` here is not an option: the endpoint answers
/// 500 for it.
const PLAYLIST_TRACK_PAGE_INCLUDE: &str = "include=albums,artists";

/// Re-attach the parameters of `original` that `next` dropped.
///
/// Apple's pagination cursor is not the request it came from: for library
/// endpoints it comes back as `?l=…&offset=…`, keeping only some of what it was
/// given. Two things go missing and both matter — `include`, so every page after
/// the first arrives with no relationships at all, and `limit`, so the walk
/// silently falls back to 25 rows a page and makes four times the requests.
///
/// Whatever `next` does carry wins: `offset` is the cursor's whole purpose, and
/// a parameter Apple chose to echo is Apple's answer for that page.
fn carry_query_forward(next: &str, original: &str) -> String {
    let (_, original_query) = match original.split_once('?') {
        Some(parts) => parts,
        None => return next.to_string(),
    };
    let (next_path, next_query) = match next.split_once('?') {
        Some((p, q)) => (p, q),
        None => (next, ""),
    };

    let key_of = |param: &str| param.split_once('=').map_or(param, |(k, _)| k).to_string();
    let present: Vec<String> = next_query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(key_of)
        .collect();

    let mut merged: Vec<&str> = next_query.split('&').filter(|p| !p.is_empty()).collect();
    for param in original_query.split('&').filter(|p| !p.is_empty()) {
        if !present.contains(&key_of(param)) {
            merged.push(param);
        }
    }

    if merged.is_empty() {
        return next_path.to_string();
    }
    format!("{next_path}?{}", merged.join("&"))
}

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
            // Both, as Apple's own client does: `Media-User-Token` is what the
            // API documents, the cookie is what the web player carries.
            req = req
                .header("Media-User-Token", token)
                .header("Cookie", format!("media-user-token={token}"));
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
            // Both, as Apple's own client does: `Media-User-Token` is what the
            // API documents, the cookie is what the web player carries.
            req = req
                .header("Media-User-Token", token)
                .header("Cookie", format!("media-user-token={token}"));
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

    /// POST with no body. The library and favorites mutations take their
    /// arguments as query parameters and reject a JSON payload.
    async fn post_empty(&self, path: &str) -> Result<reqwest::Response, String> {
        let bearer = auth::get_bearer_token().await?;
        let url = format!("{BASE}{path}");
        let mut req = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("User-Agent", USER_AGENT)
            .header("Origin", "https://music.apple.com")
            .header("Referer", "https://music.apple.com/")
            .header("Content-Length", "0");

        if let Some(token) = &self.media_user_token {
            req = req
                .header("Media-User-Token", token)
                .header("Cookie", format!("media-user-token={token}"));
        }

        tracing::debug!("am.post_empty: {url}");
        let resp = req.send().await.map_err(|e| format!("POST {path}: {e}"))?;
        let status = resp.status();
        tracing::debug!("am.post_empty: {path} → {status}");
        if !status.is_success() {
            tracing::warn!("am.post_empty: {path} failed ({status})");
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
            // Both, as Apple's own client does: `Media-User-Token` is what the
            // API documents, the cookie is what the web player carries.
            req = req
                .header("Media-User-Token", token)
                .header("Cookie", format!("media-user-token={token}"));
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
    ///
    /// Each `next` is put back through [`carry_query_forward`], because Apple
    /// echoes only some of the parameters it was given.
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
                    super::head(&body, 2000)
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
            // `self.get` prefixes BASE, so an absolute `next` has to lose it
            // first or the URL becomes `https://amp-api…https://…`. Apple does
            // return absolute cursors on some library endpoints — see
            // `get_library_playlist_tracks`, which strips it the same way. A
            // cursor pointing back at the page that produced it would page
            // forever, so that ends the walk too.
            next = parsed
                .get("next")
                .and_then(|n| n.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.strip_prefix(BASE).unwrap_or(s).to_string())
                .map(|s| carry_query_forward(&s, initial_path))
                .filter(|s| *s != path);
            if next.is_none() {
                break;
            }
        }
        tracing::info!("am.library_page: done — {} items total", all.len());
        Ok(all)
    }

    /// `include=albums` is what ties a track to its album row: the library song
    /// carries an album *name* but not its id, and genre is stored per album, so
    /// without the relationship nothing in the library can be grouped by genre.
    pub async fn get_library_songs(&self) -> Result<Vec<LibrarySongResource>, String> {
        tracing::debug!("am.get_library_songs: starting");
        self.library_page(&format!(
            "/v1/me/library/songs?l={}&limit=100&sort=dateAdded&include=catalog,albums",
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
            "/v1/me/library/playlists?l={}&limit=100&{PLAYLIST_TAGS_EXTEND}",
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
            "/v1/me/library/playlists/{}?l={}&include=tracks,artists&{PLAYLIST_TRACK_INCLUDE}&omit[resource]=autos",
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
                super::head(&body, 2000)
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
            // Strip absolute prefix so self.get() adds auth headers. The cursor
            // carries only `l` and `offset`, so the relationships have to be
            // asked for again or every page but the first arrives bare.
            let path = next_path.strip_prefix(BASE).unwrap_or(&next_path);
            let separator = if path.contains('?') { '&' } else { '?' };
            let path = format!("{path}{separator}{PLAYLIST_TRACK_PAGE_INCLUDE}");
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

    /// Find the Favorite Songs playlist in the user's library.
    /// Apple Music doesn't have a separate favorites endpoint — the
    /// Favorite Songs playlist IS the favorites.
    pub async fn find_favorite_songs_playlist(&self) -> Result<Option<String>, String> {
        tracing::debug!("am.find_favorite_songs: scanning playlists");
        let playlists = self.get_library_playlists().await?;
        tracing::debug!(
            "am.find_favorite_songs: {} playlists to scan",
            playlists.len()
        );

        if let Some(pl) = playlists
            .iter()
            .find(|pl| is_favorites_playlist(&pl.attributes))
        {
            tracing::debug!(
                "am.find_favorite_songs: found by tag — id={} name={:?}",
                pl.id,
                pl.attributes.name
            );
            return Ok(Some(pl.id.clone()));
        }

        // Only correct in English, so it runs second.
        if let Some(pl) = playlists
            .iter()
            .find(|pl| pl.attributes.name == FAVORITES_NAME_EN)
        {
            tracing::debug!("am.find_favorite_songs: found by name — id={}", pl.id);
            return Ok(Some(pl.id.clone()));
        }

        tracing::warn!(
            "am.find_favorite_songs: no Favorite Songs playlist found among {} playlists",
            playlists.len()
        );
        Ok(None)
    }

    /// Fetch favorited track IDs from the Favorite Songs playlist.
    ///
    /// The ids come out of the same conversion the tracks themselves go
    /// through, rather than being read off the response here. A playlist row
    /// carries both a row id and a catalog id, and everything else keys tracks
    /// by the catalog one — returning the row id instead marks a track that no
    /// track in the library answers to, so nothing ever shows as favorited.
    pub async fn get_favorites(&self) -> Result<Vec<String>, String> {
        tracing::debug!("am.get_favorites: starting");
        let Some(playlist_id) = self.find_favorite_songs_playlist().await? else {
            tracing::warn!("am.get_favorites: no Favorite Songs playlist — returning empty");
            return Ok(Vec::new());
        };
        let tracks = self.get_library_playlist_tracks(&playlist_id).await?;
        tracing::debug!("am.get_favorites: {} favorited tracks", tracks.len());
        Ok(tracks
            .iter()
            .map(|t| super::track_from_playlist_entry(t).id.key().into_owned())
            .collect())
    }

    // ── Stations ────────────────────────────────────────────────────

    /// The id of the station seeded by a catalog song, if it has one.
    ///
    /// Read from the song rather than assembled as `ra.{id}`. The shorthand does
    /// hold for the songs checked, but it's undocumented, and the relationship
    /// also answers the question that actually matters — whether a station
    /// exists at all.
    pub async fn song_station_id(&self, catalog_id: &str) -> Result<Option<String>, String> {
        let path = format!(
            "/v1/catalog/{}/songs/{}?l={}&include=station",
            self.storefront, catalog_id, self.language
        );
        let resp = self.get(&path).await?;
        if !resp.status().is_success() {
            return Err(format!("station lookup: HTTP {}", resp.status()));
        }
        let parsed: SongResp = resp
            .json()
            .await
            .map_err(|e| format!("parse station lookup: {e}"))?;
        Ok(parsed
            .data
            .first()
            .and_then(|song| song.relationships.station.data.first())
            .map(|station| station.id.clone()))
    }

    /// One turn of a station: the next couple of tracks.
    ///
    /// A POST with no body — the endpoint takes no count and ignores one, which
    /// is why [`station_queue`](Self::station_queue) exists.
    async fn next_station_tracks(&self, station_id: &str) -> Result<Vec<TrackData>, String> {
        let path = format!("/v1/me/stations/next-tracks/{station_id}");
        let resp = self.post_empty(&path).await?;
        if !resp.status().is_success() {
            return Err(format!("station next-tracks: HTTP {}", resp.status()));
        }
        let parsed: SongResp = resp
            .json()
            .await
            .map_err(|e| format!("parse station tracks: {e}"))?;
        Ok(parsed.data)
    }

    /// Build a queue of about `target` distinct tracks from a station.
    ///
    /// Apple returns exactly two tracks per call, so a queue is many calls
    /// stitched together. They advance a shared cursor, so firing them all off
    /// at once repeats tracks — measured at 25% duplicates across six
    /// concurrent calls and 42% across twelve, against none when sequential.
    /// Small rounds are the compromise: four at a time reaches thirty distinct
    /// tracks in about a fifth of the requests' sequential wall-clock, and the
    /// dedup below absorbs what overlap remains.
    ///
    /// A station that starts repeating itself ends the walk rather than
    /// spinning out the round budget.
    pub async fn station_queue(
        &self,
        station_id: &str,
        target: usize,
    ) -> Result<Vec<TrackData>, String> {
        const ROUND: usize = 4;
        const MAX_ROUNDS: usize = 15;

        let mut queue: Vec<TrackData> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut barren_rounds = 0;

        for round in 0..MAX_ROUNDS {
            if queue.len() >= target {
                break;
            }
            let batch = futures_util::future::join_all(
                (0..ROUND).map(|_| self.next_station_tracks(station_id)),
            )
            .await;

            let before = queue.len();
            for result in batch {
                match result {
                    Ok(tracks) => {
                        for track in tracks {
                            if seen.insert(track.id.clone()) {
                                queue.push(track);
                            }
                        }
                    }
                    // One failed call in a round isn't fatal — the others in the
                    // same round usually carry it.
                    Err(e) => tracing::debug!("am.station: a next-tracks call failed ({e})"),
                }
            }

            if queue.len() == before {
                barren_rounds += 1;
                if barren_rounds == 2 {
                    tracing::info!(
                        "am.station: stopped after {} tracks — the station stopped offering new ones",
                        queue.len()
                    );
                    break;
                }
            } else {
                barren_rounds = 0;
            }
            tracing::debug!(
                "am.station: round {} — {} distinct tracks so far",
                round + 1,
                queue.len()
            );
        }

        if queue.is_empty() {
            return Err("the station returned no tracks".to_string());
        }
        queue.truncate(target);
        tracing::info!("am.station: queued {} tracks", queue.len());
        Ok(queue)
    }

    // ── Library mutations ───────────────────────────────────────────

    /// Favourite (or un-favourite) a catalog song.
    ///
    /// Distinct from the library: `/v1/me/library` adds the song to your
    /// collection, this sets the heart. `POST` favourites, `DELETE` clears it —
    /// the same pair Apple's own client issues for `UpdateFavoritesIntent`.
    pub async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String> {
        tracing::debug!("am.set_favorite: id={item_id}, on={on}");
        let path = favorites_path(item_id);
        let resp = if on {
            self.post_empty(&path).await?
        } else {
            self.delete(&path).await?
        };
        if !resp.status().is_success() {
            let err = format!("set_favorite: HTTP {}", resp.status());
            tracing::warn!("am.set_favorite: {err}");
            return Err(err);
        }
        tracing::debug!("am.set_favorite: OK");
        Ok(())
    }

    /// Add a catalog song to the user's library. Arguments go in the query
    /// string — this endpoint takes no body.
    pub async fn add_to_library(&self, item_id: &str) -> Result<(), String> {
        tracing::debug!("am.add_to_library: id={item_id}");
        let resp = self.post_empty(&library_add_path(item_id)).await?;
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
        // Tracks belong under `relationships`, not `attributes` — Apple ignores
        // (or rejects) them anywhere else, which is why playlists created with
        // songs came out empty.
        let body = serde_json::json!({
            "attributes": { "name": name },
            "relationships": { "tracks": { "data": track_refs(item_refs) } },
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
        let body = serde_json::json!({ "data": track_refs(item_refs) });
        let resp = self.post(&playlist_add_path(playlist_id), &body).await?;
        if !resp.status().is_success() {
            let err = format!("add_to_playlist: HTTP {}", resp.status());
            tracing::warn!("am.add_to_playlist: {err}");
            return Err(err);
        }
        tracing::debug!("am.add_to_playlist: OK");
        Ok(())
    }

    /// Remove one entry from a library playlist.
    ///
    /// `entry_id` is the *library* song id of that playlist row, not the catalog
    /// Adam ID — the same track added twice is two rows with two ids. The id has
    /// to be in the query: a bare `DELETE` on the collection is unscoped and
    /// takes the playlist's whole contents with it.
    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        entry_id: &str,
    ) -> Result<(), String> {
        tracing::debug!("am.remove_from_playlist: playlist={playlist_id}, entry={entry_id}");
        if entry_id.is_empty() {
            return Err("remove_from_playlist: empty entry id".to_string());
        }
        let resp = self
            .delete(&playlist_entry_delete_path(playlist_id, entry_id))
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
    /// Resolve a library ID (starts with `i.`) to its catalog Adam ID.
    /// Returns the ID unchanged if it's already numeric.
    pub async fn resolve_catalog_id(&self, id: &str) -> Result<String, String> {
        // Catalog IDs are numeric — library IDs contain ".".
        if id.chars().all(|c| c.is_ascii_digit()) {
            return Ok(id.to_string());
        }

        tracing::debug!("am.resolve_catalog_id: resolving library id {id}");
        let path = format!("/v1/me/library/songs/{id}/catalog?l={}", self.language);
        let resp = self.get(&path).await?;
        let status = resp.status();
        if !status.is_success() {
            // Expected for uploaded iCloud Music Library tracks: they have no
            // catalog equivalent because Apple never sold them. Keeping the
            // library id is right — `web_playback_body` dispatches on it.
            tracing::debug!(
                "am.resolve_catalog_id: no catalog equivalent for {id} ({status}), \
                 playing it as a library track"
            );
            return Ok(id.to_string());
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("parse catalog response: {e}"))?;

        if let Some(data) = body["data"].as_array()
            && let Some(first) = data.first()
            && let Some(catalog_id) = first["id"].as_str()
        {
            tracing::debug!("am.resolve_catalog_id: {id} → {catalog_id}");
            return Ok(catalog_id.to_string());
        }

        tracing::debug!("am.resolve_catalog_id: no catalog id in response for {id}");
        Ok(id.to_string())
    }

    /// Fetch timed lyrics (TTML) for a song.
    /// Tries `syllable-lyrics` first (word-level timing), falls back to
    /// `lyrics` (line-level). Handles both catalog IDs and library IDs.
    pub async fn get_lyrics(&self, id: &str) -> Result<String, String> {
        let media_token = self
            .media_user_token
            .as_deref()
            .ok_or("media-user-token not set")?;
        if media_token.len() < 50 {
            return Err("media-user-token too short".into());
        }

        // Resolve library IDs to catalog IDs — lyrics API only works with catalog IDs.
        let catalog_id = self.resolve_catalog_id(id).await?;

        // Try syllable-lyrics first (word-level timing, quality 2).
        for lrc_type in &["syllable-lyrics", "lyrics"] {
            let path = format!(
                "/v1/catalog/{}/songs/{}/{lrc_type}?l={}&extend=ttmlLocalizations",
                self.storefront, catalog_id, self.language
            );
            match self.get(&path).await {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        tracing::debug!("am.get_lyrics: {lrc_type} → {status} for {catalog_id}");
                        continue;
                    }
                    let body = resp
                        .text()
                        .await
                        .map_err(|e| format!("read lyrics body: {e}"))?;
                    let parsed: SongLyricsResponse = serde_json::from_str(&body)
                        .map_err(|e| format!("parse lyrics response: {e}"))?;
                    if let Some(data) = parsed.data.first() {
                        let ttml = if !data.attributes.ttml.is_empty() {
                            &data.attributes.ttml
                        } else {
                            &data.attributes.ttml_localizations
                        };
                        if !ttml.is_empty() {
                            tracing::debug!(
                                "am.get_lyrics: got {lrc_type} for {catalog_id} ({} bytes)",
                                ttml.len()
                            );
                            return Ok(ttml.clone());
                        }
                    }
                    tracing::debug!("am.get_lyrics: {lrc_type} empty for {catalog_id}");
                }
                Err(e) => {
                    tracing::warn!("am.get_lyrics: {lrc_type} error for {catalog_id}: {e}");
                }
            }
        }
        Err("no lyrics available".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Favouriting is not adding to the library. They're different endpoints
    /// against different sets, and `fetch_favorites` reads the one this writes.
    #[test]
    fn favoriting_targets_the_heart_not_the_library() {
        let path = favorites_path("1811922756");
        assert_eq!(path, "/v1/me/favorites?ids%5Bsongs%5D=1811922756");
        assert!(!path.starts_with("/v1/me/library"));
    }

    /// The add-to-library call carries its id in the query string; this endpoint
    /// takes no JSON body, so an id in a payload goes nowhere.
    #[test]
    fn adding_to_the_library_passes_the_id_as_a_parameter() {
        assert_eq!(
            library_add_path("1811922756"),
            "/v1/me/library?ids%5Bsongs%5D=1811922756&representation=ids"
        );
    }

    /// The one that matters most: a `DELETE` on the tracks collection with no
    /// `ids[...]` is unscoped, and empties the playlist rather than removing the
    /// one row. The id has to survive into the query.
    #[test]
    fn removing_a_playlist_entry_is_scoped_to_that_entry() {
        let path = playlist_entry_delete_path("p.abc123", "i.entry456");
        assert!(
            path.contains("ids%5Blibrary-songs%5D=i.entry456"),
            "unscoped delete would wipe the playlist: {path}"
        );
        assert_eq!(
            path,
            "/v1/me/library/playlists/p.abc123/tracks\
             ?ids%5Blibrary-songs%5D=i.entry456&mode=all"
        );
    }

    #[test]
    fn adding_to_a_playlist_asks_for_the_created_resources() {
        assert_eq!(
            playlist_add_path("p.abc123"),
            "/v1/me/library/playlists/p.abc123/tracks?representation=resources"
        );
    }

    /// Apple drops track references silently when the resource `type` is
    /// missing — the request still returns 201, with an empty playlist.
    #[test]
    fn track_references_carry_the_resource_type() {
        let refs = track_refs(&["123".to_string(), "456".to_string()]);
        assert_eq!(
            serde_json::Value::Array(refs),
            serde_json::json!([
                {"id": "123", "type": "songs"},
                {"id": "456", "type": "songs"},
            ])
        );
    }

    /// The exact cursor the live library endpoint returns. Apple keeps `l`,
    /// `offset` and `sort`, and drops `limit` and `include` — so every page
    /// after the first came back without relationships and at a quarter of the
    /// page size, which is why library tracks had no album to join a genre to.
    #[test]
    fn pagination_restores_the_parameters_apple_drops() {
        let original = "/v1/me/library/songs?l=en&limit=100&sort=dateAdded&include=catalog,albums";
        let merged = carry_query_forward("/v1/me/library/songs?l=en-US&offset=100", original);

        assert!(merged.contains("include=catalog,albums"), "{merged}");
        assert!(merged.contains("limit=100"), "{merged}");
        assert!(merged.contains("offset=100"), "{merged}");
        assert!(
            merged.starts_with("/v1/me/library/songs?"),
            "the path is untouched: {merged}"
        );
    }

    /// The cursor's own values are the server's answer for that page. Taking
    /// ours instead would re-request page one forever, since `offset` is the
    /// only thing that advances.
    #[test]
    fn the_cursor_wins_where_the_two_disagree() {
        let merged = carry_query_forward(
            "/v1/me/library/albums?l=en-US&offset=100&sort=name",
            "/v1/me/library/albums?l=en&limit=100&sort=name",
        );
        assert!(merged.contains("l=en-US"), "{merged}");
        assert!(
            !merged.contains("l=en&"),
            "ours must not be re-added: {merged}"
        );
        assert_eq!(merged.matches("sort=").count(), 1, "{merged}");
        assert_eq!(merged.matches("offset=").count(), 1, "{merged}");
    }

    #[test]
    fn pagination_handles_cursors_and_requests_without_a_query() {
        // A cursor with no query at all still needs our parameters.
        let merged = carry_query_forward("/v1/me/library/songs", "/v1/me/library/songs?limit=100");
        assert_eq!(merged, "/v1/me/library/songs?limit=100");

        // Nothing to carry forward.
        assert_eq!(
            carry_query_forward("/v1/me/library/songs?offset=100", "/v1/me/library/songs"),
            "/v1/me/library/songs?offset=100"
        );
    }

    /// `include=catalog,albums` carries a comma, and a bare `mode=all`-style
    /// flag carries no `=` at all. Splitting a parameter on the wrong character
    /// would turn either into a different key and duplicate it.
    #[test]
    fn parameter_keys_survive_commas_and_valueless_flags() {
        let merged = carry_query_forward(
            "/v1/me/library/songs?offset=100&include=catalog",
            "/v1/me/library/songs?include=catalog,albums&extend",
        );
        assert_eq!(
            merged.matches("include=").count(),
            1,
            "the cursor's own include must not be duplicated: {merged}"
        );
        assert!(merged.contains("include=catalog&"), "{merged}");
        assert!(merged.contains("extend"), "{merged}");
    }

    fn playlist_attributes(json: serde_json::Value) -> LibraryPlaylistAttributes {
        serde_json::from_value(json).expect("playlist attributes parse")
    }

    /// The favorites playlist is found by Apple's own tag, not by its name.
    /// The name is localised — the same account returns "Morceaux préférés"
    /// under `l=fr` — so a name match loses every favorite outside English.
    #[test]
    fn the_favorites_playlist_is_found_by_tag_not_by_name() {
        let favorites = playlist_attributes(serde_json::json!({
            "name": "Morceaux préférés", "tags": ["favorited"],
        }));
        assert!(is_favorites_playlist(&favorites));

        let ordinary = playlist_attributes(serde_json::json!({ "name": "bgm" }));
        assert!(!is_favorites_playlist(&ordinary));
    }

    /// The tag has to be asked for by resource type. A plain `extend=tags` is
    /// accepted and ignored, and the attribute then never arrives — which is
    /// how the tag check came to look useless and get replaced by a name match.
    #[test]
    fn the_playlist_request_asks_for_tags_by_resource_type() {
        assert_eq!(PLAYLIST_TAGS_EXTEND, "extend%5Blibrary-playlists%5D=tags");
        let decoded = PLAYLIST_TAGS_EXTEND.replace("%5B", "[").replace("%5D", "]");
        assert_eq!(decoded, "extend[library-playlists]=tags");
    }

    /// Untagged playlists must not match, whether the attribute is absent
    /// entirely or present and carrying something else.
    #[test]
    fn only_the_favorited_tag_counts() {
        for attributes in [
            serde_json::json!({ "name": "bgm" }),
            serde_json::json!({ "name": "bgm", "tags": [] }),
            serde_json::json!({ "name": "bgm", "tags": ["shared", "collaborative"] }),
        ] {
            assert!(!is_favorites_playlist(&playlist_attributes(attributes)));
        }
    }

    /// The station relationship is what says whether a song can seed radio at
    /// all, so it has to survive deserialisation — an ignored field would read
    /// as "no station" and refuse every seed.
    #[test]
    fn a_songs_station_is_read_from_its_relationship() {
        let song: TrackData = serde_json::from_value(serde_json::json!({
            "id": "1760828970",
            "type": "songs",
            "relationships": {
                "station": { "data": [{ "id": "ra.1760828970", "type": "stations" }] }
            }
        }))
        .expect("song parses");
        assert_eq!(
            song.relationships
                .station
                .data
                .first()
                .map(|s| s.id.as_str()),
            Some("ra.1760828970")
        );
    }

    /// Anything Apple doesn't sell has no station. The field is simply absent
    /// then, rather than present and empty, so both have to read as "none".
    #[test]
    fn a_song_without_a_station_reports_none() {
        for song in [
            serde_json::json!({ "id": "1", "type": "songs" }),
            serde_json::json!({ "id": "1", "type": "songs", "relationships": {} }),
            serde_json::json!({
                "id": "1", "type": "songs",
                "relationships": { "station": { "data": [] } }
            }),
        ] {
            let song: TrackData = serde_json::from_value(song).expect("song parses");
            assert!(song.relationships.station.data.is_empty());
        }
    }

    /// Station tracks arrive with no relationships at all, so the artist has to
    /// come off the attributes. Reading it from the (absent) artists
    /// relationship would leave every radio track with a blank artist.
    #[test]
    fn a_station_track_keeps_its_artist_without_relationships() {
        let song: TrackData = serde_json::from_value(serde_json::json!({
            "id": "1817382430",
            "type": "songs",
            "attributes": {
                "name": "DARK THINGS",
                "artistName": "STARSET",
                "albumName": "an album",
                "durationInMillis": 246774_u64
            }
        }))
        .expect("station track parses");
        let track = super::super::track_from_song_data(&song);
        assert_eq!(track.artist, "STARSET");
        assert_eq!(track.title, "DARK THINGS");
        assert_eq!(track.duration, 246);
        assert_eq!(track.id.key(), "1817382430");
    }
}
