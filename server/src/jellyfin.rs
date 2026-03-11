use jellyfin_sdk_rust::JellyfinSDK;
use serde::{Deserialize, Serialize};

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

pub struct JellyfinRemote {
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
    pub production_year: Option<u16>,
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
    pub production_year: Option<u16>,
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

impl JellyfinRemote {
    pub fn new(
        base_url: &str,
        api_key: Option<&str>,
        device_id: &str,
        user_id: Option<&str>,
    ) -> Self {
        let mut client = JellyfinSDK::new();
        let clean_base_url = base_url.trim_end_matches('/');
        client.create_api(clean_base_url, api_key);

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            http_client,
            base_url: clean_base_url.to_string(),
            device_id: device_id.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            access_token: api_key.map(|s| s.to_string()),
        }
    }

    pub async fn login(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<(String, String), String> {
        let url = format!("{}/Users/AuthenticateByName", self.base_url);

        let body = LoginRequest { username, password };

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\"",
            self.device_id
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

    pub async fn get_metadata(&self, user_id: &str, item_id: &str) -> Result<Item, String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!(
            "{}/Users/{}/Items/{}/Metadata",
            self.base_url, user_id, item_id
        );

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let resp = self
            .http_client
            .get(&url)
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to get metadata: {}", resp.status()));
        }

        let metadata_resp: Item = resp.json().await.map_err(|e| e.to_string())?;
        Ok(metadata_resp)
    }

    pub async fn get_views(&self) -> Result<Vec<ViewItem>, String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Users/{}/Views", self.base_url, user_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let resp = self
            .http_client
            .get(&url)
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to get views: {}", resp.status()));
        }

        let views_resp: ViewItemsResponse = resp.json().await.map_err(|e| e.to_string())?;
        Ok(views_resp.items)
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
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Users/{}/Items", self.base_url, user_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let start = start_index.to_string();
        let limit_val = limit.to_string();

        let resp = self.http_client
            .get(&url)
            .query(&[
                ("ParentId", parent_id),
                ("Recursive", "true"),
                ("IncludeItemTypes", "Audio"),
                (
                    "Fields",
                    "DateCreated,DateLastMediaAdded,MediaSources,ImageTags,Genres,ParentIndexNumber,IndexNumber,AlbumId,AlbumArtist,ProductionYear,Container",
                ),
                ("StartIndex", start.as_str()),
                ("Limit", limit_val.as_str()),
            ])
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to get music items: {}", resp.status()));
        }

        let items_resp: ItemsResponse = resp.json().await.map_err(|e| e.to_string())?;
        Ok(items_resp.items)
    }

    pub async fn get_playlists(&self) -> Result<Vec<Item>, String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Users/{}/Items", self.base_url, user_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let fields = "DateCreated,DateLastMediaAdded".to_string();
        let resp = self
            .http_client
            .get(&url)
            .query(&[
                ("IncludeItemTypes", "Playlist"),
                ("Recursive", "true"),
                ("Fields", &fields),
                ("MediaTypes", "Audio"),
            ])
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to get playlists: {}", resp.status()));
        }

        let items_resp: ItemsResponse = resp.json().await.map_err(|e| e.to_string())?;
        Ok(items_resp.items)
    }

    pub async fn create_playlist(&self, name: &str, item_ids: &[&str]) -> Result<String, String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Playlists", self.base_url);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
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

    pub async fn add_to_playlist(&self, playlist_id: &str, item_id: &str) -> Result<(), String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Playlists/{}/Items", self.base_url, playlist_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
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
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Playlists/{}/Items", self.base_url, playlist_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let fields = "DateCreated,DateLastMediaAdded,MediaSources,ImageTags,Genres,ParentIndexNumber,IndexNumber,AlbumId,AlbumArtist,ProductionYear,Container".to_string();
        let resp = self
            .http_client
            .get(&url)
            .query(&[("UserId", user_id), ("Fields", &fields)])
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to get playlist items: {}", resp.status()));
        }

        let items_resp: ItemsResponse = resp.json().await.map_err(|e| e.to_string())?;
        Ok(items_resp.items)
    }

    pub async fn get_genres(&self) -> Result<Vec<Genre>, String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Genres", self.base_url);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let resp = self
            .http_client
            .get(&url)
            .query(&[
                ("UserId", user_id.as_str()),
                ("Recursive", "true"),
                ("IncludeItemTypes", "Audio"),
            ])
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to get genres: {}", resp.status()));
        }

        let genres_resp: GenresResponse = resp.json().await.map_err(|e| e.to_string())?;
        Ok(genres_resp.items)
    }

    pub async fn get_albums_paginated(
        &self,
        parent_id: &str,
        start_index: usize,
        limit: usize,
    ) -> Result<(Vec<AlbumItem>, u32), String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Users/{}/Items", self.base_url, user_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let start = start_index.to_string();
        let limit_val = limit.to_string();

        let resp = self
            .http_client
            .get(&url)
            .query(&[
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
            ])
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to get albums: {}", resp.status()));
        }

        let albums_resp: AlbumsResponse = resp.json().await.map_err(|e| e.to_string())?;
        Ok((albums_resp.items, albums_resp.total_record_count))
    }

    pub async fn report_playback_start(&self, item_id: &str) -> Result<(), String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;
        let url = format!("{}/Sessions/Playing", self.base_url);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
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
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
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
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
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

    pub async fn ping(&self) -> Result<(), String> {
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;
        let url = format!("{}/Sessions/Ping", self.base_url);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let resp = self
            .http_client
            .post(&url)
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Ping failed: {}", resp.status()));
        }

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
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
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
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
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

    pub async fn get_favorite_items(&self) -> Result<Vec<Item>, String> {
        let user_id = self.user_id.as_ref().ok_or("No user ID available")?;
        let token = self
            .access_token
            .as_ref()
            .ok_or("No access token available")?;

        let url = format!("{}/Users/{}/Items", self.base_url, user_id);

        let auth_header = format!(
            "MediaBrowser Client=\"Rusic\", Device=\"Rusic\", DeviceId=\"{}\", Version=\"0.3.1\", Token=\"{}\"",
            self.device_id, token
        );

        let fields = "DateCreated,MediaSources,ImageTags,Genres,ParentIndexNumber,IndexNumber,AlbumId,AlbumArtist,ProductionYear,Container".to_string();

        let resp = self
            .http_client
            .get(&url)
            .query(&[
                ("Filters", "IsFavorite"),
                ("IncludeItemTypes", "Audio"),
                ("Recursive", "true"),
                ("Fields", &fields),
            ])
            .header("X-Emby-Authorization", auth_header)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Failed to get favorite items: {}", resp.status()));
        }

        let items_resp: ItemsResponse = resp.json().await.map_err(|e| e.to_string())?;
        Ok(items_resp.items)
    }
}
