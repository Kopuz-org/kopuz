use jellyfin_sdk_rust::JellyfinSDK;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize)]
struct PlaybackProgressRequest<'a> {
    #[serde(rename = "ItemId")]
    item_id: &'a str,
    #[serde(rename = "PositionTicks")]
    #[serde(skip_serializing_if = "Option::is_none")]
    position_ticks: Option<u64>,
    #[serde(rename = "IsPaused")]
    #[serde(skip_serializing_if = "Option::is_none")]
    is_paused: Option<bool>,
    #[serde(rename = "CanSeek")]
    #[serde(skip_serializing_if = "Option::is_none")]
    can_seek: Option<bool>,
}

#[derive(Serialize)]
struct PlaybackStopRequest<'a> {
    #[serde(rename = "ItemId")]
    item_id: &'a str,
    #[serde(rename = "PositionTicks")]
    #[serde(skip_serializing_if = "Option::is_none")]
    position_ticks: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct CreatePlaylistRequest<'a> {
    name: &'a str,
    user_id: &'a str,
    media_type: &'a str,
    is_public: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ids: Vec<&'a str>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PlaylistCreationResult {
    pub id: String,
}

pub(crate) struct JellyfinClient {
    client: JellyfinSDK,
    http_client: reqwest::Client,
    base_url: String,
    device_id: String,
    user_id: Option<String>,
    access_token: Option<String>,
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    #[serde(rename = "Username")]
    username: &'a str,
    #[serde(rename = "Pw")]
    password: &'a str,
}

#[derive(Deserialize)]
struct LoginResponse {
    #[serde(rename = "AccessToken")]
    access_token: String,
    #[serde(rename = "User")]
    #[allow(dead_code)]
    user: UserObj,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct UserObj {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct ViewItemsResponse {
    pub items: Vec<ViewItem>,
    pub total_record_count: u32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct ViewItem {
    pub name: String,
    pub id: String,
    pub collection_type: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct ItemsResponse {
    pub items: Vec<Item>,
    pub total_record_count: u32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub name: String,
    pub id: String,
    #[serde(rename = "PlaylistItemId")]
    pub playlist_item_id: Option<String>,
    #[serde(rename = "Type")]
    pub item_type: String,
    pub run_time_ticks: Option<u64>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub artists: Option<Vec<String>>,
    pub album_artist: Option<String>,
    pub image_tags: Option<std::collections::HashMap<String, String>>,
    pub index_number: Option<u32>,
    pub parent_index_number: Option<u32>,
    pub production_year: Option<u32>,
    pub genres: Option<Vec<String>>,
    pub container: Option<String>,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct AlbumItem {
    pub name: String,
    pub id: String,
    pub album_artist: Option<String>,
    pub artists: Option<Vec<String>>,
    pub production_year: Option<u32>,
    pub genres: Option<Vec<String>>,
    pub image_tags: Option<std::collections::HashMap<String, String>>,
    pub child_count: Option<u32>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct AlbumsResponse {
    pub items: Vec<AlbumItem>,
    pub total_record_count: u32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Genre {
    pub name: String,
    pub id: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct GenresResponse {
    pub items: Vec<Genre>,
    pub total_record_count: u32,
}

impl JellyfinClient {
    pub fn new(
        base_url: &str,
        api_key: Option<&str>,
        device_id: &str,
        user_id: Option<&str>,
    ) -> Self {
        let mut client = JellyfinSDK::new();
        let clean_base_url = base_url.trim_end_matches('/');
        client.create_api(clean_base_url, api_key);

        let builder = reqwest::Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder.timeout(std::time::Duration::from_secs(10));
        let http_client = builder.build().unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            http_client,
            base_url: clean_base_url.to_string(),
            device_id: device_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            access_token: api_key.map(|s| s.to_string()),
        }
    }

    fn user_id(&self) -> Result<&str, String> {
        self.user_id
            .as_deref()
            .ok_or_else(|| "No user ID available".to_string())
    }

    fn access_token(&self) -> Result<&str, String> {
        self.access_token
            .as_deref()
            .ok_or_else(|| "No access token available".to_string())
    }

    /// The server base URL — for building image URLs outside the client.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The access token, if signed in — for building authed image URLs.
    pub fn token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    fn build_url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}/{}", self.base_url, path)
        }
    }

    fn auth_header(&self) -> Result<String, String> {
        let token = self.access_token()?;
        Ok(format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        ))
    }

    fn authorized_request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, String> {
        let auth_header = self.auth_header()?;
        Ok(self
            .http_client
            .request(method, self.build_url(path))
            .header("X-Emby-Authorization", auth_header))
    }

    async fn ensure_success(
        resp: reqwest::Response,
        context: &str,
    ) -> Result<reqwest::Response, String> {
        if resp.status().is_success() {
            return Ok(resp);
        }

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if text.is_empty() {
            Err(format!("{}: {}", context, status))
        } else {
            Err(format!("{}: {} - {}", context, status, text))
        }
    }

    #[tracing::instrument(name = "jellyfin.request", skip_all, fields(path = %path))]
    async fn request<T>(&self, path: &str) -> Result<T, String>
    where
        T: DeserializeOwned,
    {
        let resp = self
            .authorized_request(reqwest::Method::GET, path)?
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let context = format!("Request failed for {}", path);
        let resp = Self::ensure_success(resp, &context).await?;

        resp.json::<T>().await.map_err(|e| e.to_string())
    }

    #[tracing::instrument(name = "jellyfin.request", skip_all, fields(path = %path))]
    async fn request_with_query<T, Q>(&self, path: &str, query: &Q) -> Result<T, String>
    where
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
    {
        let resp = self
            .authorized_request(reqwest::Method::GET, path)?
            .query(query)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let context = format!("Request failed for {}", path);
        let resp = Self::ensure_success(resp, &context).await?;

        resp.json::<T>().await.map_err(|e| e.to_string())
    }

    pub async fn login(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<(String, String), String> {
        let url = format!("{}/Users/AuthenticateByName", self.base_url);

        let body = LoginRequest { username, password };

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\"",
            self.device_id, APP_VERSION
        );

        let resp = self
            .http_client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Login failed with status: {} - {}", status, text));
        }

        let login_resp: LoginResponse = resp.json().await.map_err(|e| e.to_string())?;

        self.access_token = Some(login_resp.access_token.clone());
        self.user_id = Some(login_resp.user.id.clone());

        self.client
            .create_api(&self.base_url, Some(&login_resp.access_token));

        Ok((login_resp.access_token, login_resp.user.id))
    }

    pub async fn get_views(&self) -> Result<Vec<ViewItem>, String> {
        let user_id = self.user_id()?;
        let path = format!("/Users/{}/Views", user_id);
        self.request::<ViewItemsResponse>(&path)
            .await
            .map(|r| r.items)
    }

    pub async fn get_music_libraries(&self) -> Result<Vec<ViewItem>, String> {
        let views = self.get_views().await?;
        let music_libs = views
            .into_iter()
            .filter(|v| v.collection_type.as_deref() == Some("music"))
            .collect();
        Ok(music_libs)
    }

    pub async fn get_music_library_items_paginated(
        &self,
        parent_id: &str,
        start_index: usize,
        limit: usize,
    ) -> Result<Vec<Item>, String> {
        let user_id = self.user_id()?;
        let path = format!("/Users/{}/Items", user_id);

        let start = start_index.to_string();
        let limit_val = limit.to_string();
        let query = [
            ("ParentId", parent_id),
            ("Recursive", "true"),
            ("IncludeItemTypes", "Audio"),
            (
                "Fields",
                "DateCreated,DateLastMediaAdded,MediaSources,ImageTags,Genres,ParentIndexNumber,IndexNumber,AlbumId,AlbumArtist,ProductionYear,Container",
            ),
            ("StartIndex", start.as_str()),
            ("Limit", limit_val.as_str()),
        ];

        let items_resp: ItemsResponse = self.request_with_query(&path, &query).await?;
        Ok(items_resp.items)
    }

    pub async fn get_playlists(&self) -> Result<Vec<Item>, String> {
        let user_id = self.user_id()?;
        let path = format!("/Users/{}/Items", user_id);

        let fields = "DateCreated,DateLastMediaAdded,ImageTags".to_string();
        let query = [
            ("IncludeItemTypes", "Playlist"),
            ("Recursive", "true"),
            ("Fields", fields.as_str()),
            ("MediaTypes", "Audio"),
        ];

        let items_resp: ItemsResponse = self.request_with_query(&path, &query).await?;
        Ok(items_resp.items)
    }

    /// The progressive audio stream URL for one item, with the api_key inlined.
    pub fn stream_url(&self, item_id: &str) -> String {
        let mut url = format!("{}/Audio/{}/stream?static=true", self.base_url, item_id);
        if let Some(token) = &self.access_token {
            url.push_str(&format!("&api_key={token}"));
        }
        url
    }

    #[tracing::instrument(name = "jellyfin.playlist_create", skip_all, fields(count = item_ids.len()))]
    pub async fn create_playlist(&self, name: &str, item_ids: &[&str]) -> Result<String, String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Playlists", self.base_url);

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let body = CreatePlaylistRequest {
            name,
            user_id,
            media_type: "Audio",
            is_public: true,
            ids: item_ids.to_vec(),
        };

        let resp = self
            .http_client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Failed to create playlist: {} - {}", status, text));
        }

        let result: PlaylistCreationResult = resp.json().await.map_err(|e| e.to_string())?;
        Ok(result.id)
    }

    #[tracing::instrument(name = "jellyfin.playlist_add", skip(self), fields(playlist_id = %playlist_id, item_id = %item_id))]
    pub async fn add_to_playlist(&self, playlist_id: &str, item_id: &str) -> Result<(), String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Playlists/{}/Items", self.base_url, playlist_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let resp = self
            .http_client
            .post(&url)
            .query(&[("Ids", item_id), ("UserId", user_id)])
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to add item to playlist: {}", resp.status()));
        }

        Ok(())
    }

    pub async fn get_playlist_items(&self, playlist_id: &str) -> Result<Vec<Item>, String> {
        let user_id = self.user_id()?;
        let path = format!("/Playlists/{}/Items", playlist_id);

        let fields = "DateCreated,DateLastMediaAdded,MediaSources,ImageTags,Genres,ParentIndexNumber,IndexNumber,AlbumId,AlbumArtist,ProductionYear,Container,PlaylistItemId".to_string();
        let query = [("UserId", user_id), ("Fields", fields.as_str())];
        let items_resp: ItemsResponse = self.request_with_query(&path, &query).await?;
        Ok(items_resp.items)
    }

    #[tracing::instrument(name = "jellyfin.playlist_remove", skip(self), fields(playlist_id = %playlist_id, entry_id = %entry_id))]
    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        entry_id: &str,
    ) -> Result<(), String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Playlists/{}/Items", self.base_url, playlist_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let resp = self
            .http_client
            .delete(&url)
            .query(&[("EntryIds", entry_id)])
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!(
                "Failed to remove item from playlist: {}",
                resp.status()
            ));
        }

        Ok(())
    }

    #[tracing::instrument(name = "jellyfin.playlist_move", skip(self), fields(playlist_id = %playlist_id, item_id = %item_id, new_index))]
    pub async fn move_playlist_item(
        &self,
        playlist_id: &str,
        item_id: &str,
        new_index: usize,
    ) -> Result<(), String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!(
            "{}/Playlists/{}/Items/{}/Move/{}",
            self.base_url, playlist_id, item_id, new_index
        );

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let resp = self
            .http_client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .header("Content-Length", "0")
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to move playlist item: {}", resp.status()));
        }

        Ok(())
    }

    #[tracing::instrument(name = "jellyfin.set_playlist_image", skip_all, fields(playlist_id = %playlist_id))]
    pub async fn set_playlist_image(
        &self,
        playlist_id: &str,
        image_bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), String> {
        let resp = self
            .authorized_request(
                reqwest::Method::POST,
                &format!("/Items/{}/Images/Primary", playlist_id),
            )?
            .header("Content-Type", content_type)
            .body(image_bytes)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        Self::ensure_success(resp, "Failed to upload playlist image").await?;
        Ok(())
    }

    pub async fn get_albums_paginated(
        &self,
        parent_id: &str,
        start_index: usize,
        limit: usize,
    ) -> Result<(Vec<AlbumItem>, u32), String> {
        let user_id = self.user_id()?;
        let path = format!("/Users/{}/Items", user_id);

        let start = start_index.to_string();
        let limit_val = limit.to_string();
        let query = [
            ("ParentId", parent_id),
            ("Recursive", "true"),
            ("IncludeItemTypes", "MusicAlbum"),
            (
                "Fields",
                "ImageTags,Genres,ProductionYear,AlbumArtist,ChildCount",
            ),
            ("SortBy", "SortName"),
            ("SortOrder", "Ascending"),
            ("StartIndex", start.as_str()),
            ("Limit", limit_val.as_str()),
        ];

        let albums_resp: AlbumsResponse = self.request_with_query(&path, &query).await?;
        Ok((albums_resp.items, albums_resp.total_record_count))
    }

    #[tracing::instrument(name = "jellyfin.playback_report", skip(self), fields(item_id = %item_id))]
    pub async fn report_playback_start(&self, item_id: &str) -> Result<(), String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;
        let url = format!("{}/Sessions/Playing", self.base_url);

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let body = PlaybackProgressRequest {
            item_id,
            position_ticks: Some(0),
            is_paused: Some(false),
            can_seek: Some(true),
        };

        let resp = self
            .http_client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!(
                "Failed to report playback start: {}",
                resp.status()
            ));
        }

        Ok(())
    }

    #[tracing::instrument(name = "jellyfin.playback_report", skip(self), fields(item_id = %item_id))]
    pub async fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: u64,
        is_paused: bool,
    ) -> Result<(), String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;
        let url = format!("{}/Sessions/Playing/Progress", self.base_url);

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let body = PlaybackProgressRequest {
            item_id,
            position_ticks: Some(position_ticks),
            is_paused: Some(is_paused),
            can_seek: Some(true),
        };

        let resp = self
            .http_client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!(
                "Failed to report playback progress: {}",
                resp.status()
            ));
        }

        Ok(())
    }

    #[tracing::instrument(name = "jellyfin.playback_report", skip(self), fields(item_id = %item_id))]
    pub async fn report_playback_stopped(
        &self,
        item_id: &str,
        position_ticks: u64,
    ) -> Result<(), String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;
        let url = format!("{}/Sessions/Playing/Stopped", self.base_url);

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let body = PlaybackStopRequest {
            item_id,
            position_ticks: Some(position_ticks),
        };

        let resp = self
            .http_client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!(
                "Failed to report playback stopped: {}",
                resp.status()
            ));
        }

        Ok(())
    }

    /// Validate the connection + token. Hits `GET /Users/Me` (token-gated, present
    /// on every Jellyfin) rather than `POST /Sessions/Ping`, which 404s on 10.11+.
    /// A 401/403 surfaces in the error string so callers can tell auth-expired
    /// (`Expired`) from unreachable.
    #[tracing::instrument(name = "jellyfin.ping", skip_all)]
    pub async fn ping(&self) -> Result<(), String> {
        let resp = self
            .authorized_request(reqwest::Method::GET, "/Users/Me")?
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Self::ensure_success(resp, "Ping failed").await?;
        Ok(())
    }

    pub async fn mark_favorite(&self, item_id: &str) -> Result<(), String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!(
            "{}/Users/{}/FavoriteItems/{}",
            self.base_url, user_id, item_id
        );

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let resp = self
            .http_client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to mark favorite: {}", resp.status()));
        }

        Ok(())
    }

    pub async fn unmark_favorite(&self, item_id: &str) -> Result<(), String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!(
            "{}/Users/{}/FavoriteItems/{}",
            self.base_url, user_id, item_id
        );

        let auth_header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
            self.device_id, APP_VERSION, token
        );

        let resp = self
            .http_client
            .delete(&url)
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to unmark favorite: {}", resp.status()));
        }

        Ok(())
    }

    pub async fn get_artists(&self) -> Result<Vec<Item>, String> {
        let user_id = self.user_id()?;
        let path = format!("/Users/{}/Items", user_id);
        let fields = "ImageTags";
        let query = [
            ("IncludeItemTypes", "MusicArtist"),
            ("Recursive", "true"),
            ("Fields", fields),
        ];
        let items_resp: ItemsResponse = self.request_with_query(&path, &query).await?;
        Ok(items_resp.items)
    }

    pub async fn get_favorite_items(&self) -> Result<Vec<Item>, String> {
        let user_id = self.user_id()?;
        let path = format!("/Users/{}/Items", user_id);

        let fields = "DateCreated,MediaSources,ImageTags,Genres,ParentIndexNumber,IndexNumber,AlbumId,AlbumArtist,ProductionYear,Container".to_string();
        let query = [
            ("Filters", "IsFavorite"),
            ("IncludeItemTypes", "Audio"),
            ("Recursive", "true"),
            ("Fields", fields.as_str()),
        ];

        let items_resp: ItemsResponse = self.request_with_query(&path, &query).await?;
        Ok(items_resp.items)
    }
}
