use std::collections::HashMap;
use std::time::Duration;

use kopuz_plugin_sdk::http::{Client, RequestBuilder, Response};
use kopuz_plugin_sdk::{ReplayGain, SourceError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Fields every track listing asks for.
const TRACK_FIELDS: &str = "DateCreated,DateLastMediaAdded,MediaSources,ImageTags,Genres,ParentIndexNumber,IndexNumber,AlbumId,AlbumArtist,ProductionYear,Container,ArtistItems";

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PlaybackProgressRequest<'a> {
    item_id: &'a str,
    position_ticks: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_paused: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_seek: Option<bool>,
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

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PlaylistCreationResult {
    id: String,
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
    user: UserObj,
}

#[derive(Deserialize)]
struct UserObj {
    #[serde(rename = "Id")]
    id: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ViewItemsResponse {
    items: Vec<ViewItem>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct ViewItem {
    pub id: String,
    pub collection_type: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ItemsResponse {
    items: Vec<Item>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub name: String,
    pub id: String,
    #[serde(rename = "PlaylistItemId")]
    pub playlist_item_id: Option<String>,
    pub run_time_ticks: Option<u64>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub artists: Option<Vec<String>>,
    /// Absent unless the request's `Fields` asked for `ArtistItems`.
    pub artist_items: Option<Vec<NamedItem>>,
    pub album_artist: Option<String>,
    pub image_tags: Option<HashMap<String, String>>,
    pub index_number: Option<u32>,
    pub parent_index_number: Option<u32>,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub normalization_gain: Option<f32>,
    pub album_normalization_gain: Option<f32>,
}

impl Item {
    /// Jellyfin derives these from LUFS and publishes no peak, so clip
    /// prevention has nothing to work with on a Jellyfin-supplied gain.
    pub fn replay_gain(&self) -> ReplayGain {
        ReplayGain {
            track_gain_db: self.normalization_gain.filter(|db| db.is_finite()),
            album_gain_db: self.album_normalization_gain.filter(|db| db.is_finite()),
            ..ReplayGain::default()
        }
    }

    pub fn primary_image(&self) -> Option<String> {
        self.image_tags.as_ref()?.get("Primary").cloned()
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct NamedItem {
    pub name: String,
    pub id: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct AlbumItem {
    pub name: String,
    pub id: String,
    pub album_artist: Option<String>,
    pub album_artists: Option<Vec<NamedItem>>,
    pub artists: Option<Vec<String>>,
    pub production_year: Option<u32>,
    pub genres: Option<Vec<String>>,
    pub image_tags: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct AlbumsResponse {
    items: Vec<AlbumItem>,
}

pub(crate) struct JellyfinClient {
    http: Client,
    base_url: String,
    device_id: String,
    user_id: Option<String>,
    access_token: Option<String>,
}

impl JellyfinClient {
    pub fn new(
        base_url: &str,
        access_token: Option<&str>,
        device_id: &str,
        user_id: Option<&str>,
    ) -> Self {
        Self {
            http: Client::new().with_timeout(Duration::from_secs(10)),
            base_url: base_url.trim_end_matches('/').to_string(),
            device_id: device_id.to_string(),
            user_id: user_id.filter(|id| !id.is_empty()).map(str::to_string),
            access_token: access_token
                .filter(|token| !token.is_empty())
                .map(str::to_string),
        }
    }

    fn user_id(&self) -> Result<&str, SourceError> {
        self.user_id.as_deref().ok_or(SourceError::Auth)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    fn auth_header(&self, token: Option<&str>) -> String {
        let mut header = format!(
            "MediaBrowser Client=\"Kopuz\", Device=\"Kopuz\", DeviceId=\"{}\", Version=\"{APP_VERSION}\"",
            self.device_id
        );
        if let Some(token) = token {
            header.push_str(&format!(", Token=\"{token}\""));
        }
        header
    }

    fn authorized(&self, method: &str, path: &str) -> Result<RequestBuilder, SourceError> {
        let token = self.access_token.as_deref().ok_or(SourceError::Auth)?;
        Ok(self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .header("Authorization", self.auth_header(Some(token))))
    }

    fn send(request: RequestBuilder, context: &str) -> Result<Response, SourceError> {
        let response = request.send()?;
        if response.is_success() {
            return Ok(response);
        }
        let status = response.status();
        if matches!(status, 401 | 403) {
            return Err(SourceError::Auth);
        }
        let text = response.text();
        Err(SourceError::Backend(if text.is_empty() {
            format!("{context}: HTTP {status}")
        } else {
            format!("{context}: HTTP {status} - {text}")
        }))
    }

    fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, SourceError> {
        let response = Self::send(
            self.authorized("GET", path)?.query(query),
            &format!("Request failed for {path}"),
        )?;
        response
            .json()
            .map_err(|error| SourceError::Backend(error.to_string()))
    }

    pub fn login(&self, username: &str, password: &str) -> Result<(String, String), SourceError> {
        let response = self
            .http
            .post(format!("{}/Users/AuthenticateByName", self.base_url))
            .header("Authorization", self.auth_header(None))
            .json(&LoginRequest { username, password })
            .send()?;
        if !response.is_success() {
            return Err(SourceError::from_status(
                response.status(),
                "Jellyfin login",
            ));
        }
        let login: LoginResponse = response
            .json()
            .map_err(|error| SourceError::Backend(error.to_string()))?;
        Ok((login.access_token, login.user.id))
    }

    pub fn get_music_libraries(&self) -> Result<Vec<ViewItem>, SourceError> {
        let path = format!("/Users/{}/Views", self.user_id()?);
        Ok(self
            .get::<ViewItemsResponse>(&path, &[])?
            .items
            .into_iter()
            .filter(|view| view.collection_type.as_deref() == Some("music"))
            .collect())
    }

    pub fn get_music_library_items_paginated(
        &self,
        parent_id: &str,
        start_index: usize,
        limit: usize,
    ) -> Result<Vec<Item>, SourceError> {
        let path = format!("/Users/{}/Items", self.user_id()?);
        let start = start_index.to_string();
        let limit = limit.to_string();
        let query = [
            ("ParentId", parent_id),
            ("Recursive", "true"),
            ("IncludeItemTypes", "Audio"),
            ("Fields", TRACK_FIELDS),
            ("StartIndex", start.as_str()),
            ("Limit", limit.as_str()),
        ];
        Ok(self.get::<ItemsResponse>(&path, &query)?.items)
    }

    pub fn get_albums_paginated(
        &self,
        parent_id: &str,
        start_index: usize,
        limit: usize,
    ) -> Result<Vec<AlbumItem>, SourceError> {
        let path = format!("/Users/{}/Items", self.user_id()?);
        let start = start_index.to_string();
        let limit = limit.to_string();
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
            ("Limit", limit.as_str()),
        ];
        Ok(self.get::<AlbumsResponse>(&path, &query)?.items)
    }

    pub fn get_playlists(&self) -> Result<Vec<Item>, SourceError> {
        let path = format!("/Users/{}/Items", self.user_id()?);
        let query = [
            ("IncludeItemTypes", "Playlist"),
            ("Recursive", "true"),
            ("Fields", "DateCreated,DateLastMediaAdded,ImageTags"),
            ("MediaTypes", "Audio"),
        ];
        Ok(self.get::<ItemsResponse>(&path, &query)?.items)
    }

    pub fn get_playlist_items(&self, playlist_id: &str) -> Result<Vec<Item>, SourceError> {
        let path = format!("/Playlists/{playlist_id}/Items");
        let fields = format!("{TRACK_FIELDS},PlaylistItemId");
        let query = [("UserId", self.user_id()?), ("Fields", fields.as_str())];
        Ok(self.get::<ItemsResponse>(&path, &query)?.items)
    }

    pub fn get_artists(&self) -> Result<Vec<Item>, SourceError> {
        let path = format!("/Users/{}/Items", self.user_id()?);
        let query = [
            ("IncludeItemTypes", "MusicArtist"),
            ("Recursive", "true"),
            ("Fields", "ImageTags"),
        ];
        Ok(self.get::<ItemsResponse>(&path, &query)?.items)
    }

    pub fn get_favorite_items(&self) -> Result<Vec<Item>, SourceError> {
        let path = format!("/Users/{}/Items", self.user_id()?);
        let query = [
            ("Filters", "IsFavorite"),
            ("IncludeItemTypes", "Audio"),
            ("Recursive", "true"),
            (
                "Fields",
                "DateCreated,MediaSources,ImageTags,Genres,ParentIndexNumber,IndexNumber,AlbumId,AlbumArtist,ProductionYear,Container",
            ),
        ];
        Ok(self.get::<ItemsResponse>(&path, &query)?.items)
    }

    /// The progressive audio stream URL for one item, with the api_key inlined.
    pub fn stream_url(&self, item_id: &str) -> String {
        let mut url = format!("{}/Audio/{item_id}/stream?static=true", self.base_url);
        if let Some(token) = &self.access_token {
            url.push_str(&format!("&api_key={token}"));
        }
        url
    }

    pub fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, SourceError> {
        let body = CreatePlaylistRequest {
            name,
            user_id: self.user_id()?,
            media_type: "Audio",
            is_public: true,
            ids: item_ids.iter().map(String::as_str).collect(),
        };
        let response = Self::send(
            self.authorized("POST", "/Playlists")?.json(&body),
            "Failed to create playlist",
        )?;
        let created: PlaylistCreationResult = response
            .json()
            .map_err(|error| SourceError::Backend(error.to_string()))?;
        Ok(created.id)
    }

    pub fn add_to_playlist(&self, playlist_id: &str, item_id: &str) -> Result<(), SourceError> {
        let user_id = self.user_id()?;
        let response = self
            .authorized("POST", &format!("/Playlists/{playlist_id}/Items"))?
            .query(&[("Ids", item_id), ("UserId", user_id)])
            .send()?;
        status_only(&response, "Jellyfin: add item to playlist")
    }

    pub fn remove_from_playlist(
        &self,
        playlist_id: &str,
        entry_id: &str,
    ) -> Result<(), SourceError> {
        let response = self
            .authorized("DELETE", &format!("/Playlists/{playlist_id}/Items"))?
            .query(&[("EntryIds", entry_id)])
            .send()?;
        status_only(&response, "Jellyfin: remove item from playlist")
    }

    pub fn move_playlist_item(
        &self,
        playlist_id: &str,
        entry_id: &str,
        new_index: u32,
    ) -> Result<(), SourceError> {
        let response = self
            .authorized(
                "POST",
                &format!("/Playlists/{playlist_id}/Items/{entry_id}/Move/{new_index}"),
            )?
            .header("Content-Length", "0")
            .send()?;
        status_only(&response, "Jellyfin: move playlist item")
    }

    pub fn set_playlist_image(
        &self,
        playlist_id: &str,
        image: &[u8],
        content_type: &str,
    ) -> Result<(), SourceError> {
        Self::send(
            self.authorized("POST", &format!("/Items/{playlist_id}/Images/Primary"))?
                .header("Content-Type", content_type)
                .body(image.to_vec()),
            "Failed to upload playlist image",
        )
        .map(|_| ())
    }

    fn report(
        &self,
        path: &str,
        body: &PlaybackProgressRequest<'_>,
        what: &str,
    ) -> Result<(), SourceError> {
        let response = self.authorized("POST", path)?.json(body).send()?;
        status_only(&response, what)
    }

    pub fn report_playback_start(&self, item_id: &str) -> Result<(), SourceError> {
        self.report(
            "/Sessions/Playing",
            &PlaybackProgressRequest {
                item_id,
                position_ticks: 0,
                is_paused: Some(false),
                can_seek: Some(true),
            },
            "Jellyfin: report playback start",
        )
    }

    pub fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: u64,
        is_paused: bool,
    ) -> Result<(), SourceError> {
        self.report(
            "/Sessions/Playing/Progress",
            &PlaybackProgressRequest {
                item_id,
                position_ticks,
                is_paused: Some(is_paused),
                can_seek: Some(true),
            },
            "Jellyfin: report playback progress",
        )
    }

    pub fn report_playback_stopped(
        &self,
        item_id: &str,
        position_ticks: u64,
    ) -> Result<(), SourceError> {
        self.report(
            "/Sessions/Playing/Stopped",
            &PlaybackProgressRequest {
                item_id,
                position_ticks,
                is_paused: None,
                can_seek: None,
            },
            "Jellyfin: report playback stopped",
        )
    }

    /// Validate the connection + token. Hits `GET /Users/Me` (token-gated, present
    /// on every Jellyfin) rather than `POST /Sessions/Ping`, which 404s on 10.11+.
    pub fn ping(&self) -> Result<(), SourceError> {
        Self::send(self.authorized("GET", "/Users/Me")?, "Ping failed").map(|_| ())
    }

    pub fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        let path = format!("/Users/{}/FavoriteItems/{item_id}", self.user_id()?);
        let (method, what) = if on {
            ("POST", "Jellyfin: mark favorite")
        } else {
            ("DELETE", "Jellyfin: unmark favorite")
        };
        let response = self.authorized(method, &path)?.send()?;
        status_only(&response, what)
    }
}

fn status_only(response: &Response, what: &str) -> Result<(), SourceError> {
    if response.is_success() {
        Ok(())
    } else {
        Err(SourceError::from_status(response.status(), what))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_normalization_gains() {
        let item: Item = serde_json::from_str(
            r#"{
                "Name": "T",
                "Id": "1",
                "Type": "Audio",
                "NormalizationGain": -7.5,
                "AlbumNormalizationGain": -5.25
            }"#,
        )
        .unwrap();

        let gain = item.replay_gain();
        assert_eq!(gain.track_gain_db, Some(-7.5));
        assert_eq!(gain.album_gain_db, Some(-5.25));
        // Jellyfin publishes no peaks, so clip prevention stays inert here.
        assert_eq!(gain.track_peak, None);
        assert_eq!(gain.album_peak, None);
    }

    #[test]
    fn an_item_from_an_older_server_reports_nothing() {
        let item: Item =
            serde_json::from_str(r#"{"Name": "T", "Id": "1", "Type": "Audio"}"#).unwrap();
        assert_eq!(item.replay_gain(), ReplayGain::default());
    }

    #[test]
    fn a_stream_url_carries_the_token_only_when_signed_in() {
        let signed_in = JellyfinClient::new("https://jf.test/", Some("tok"), "dev", Some("u"));
        assert_eq!(
            signed_in.stream_url("i1"),
            "https://jf.test/Audio/i1/stream?static=true&api_key=tok"
        );
        let anonymous = JellyfinClient::new("https://jf.test", None, "dev", None);
        assert_eq!(
            anonymous.stream_url("i1"),
            "https://jf.test/Audio/i1/stream?static=true"
        );
    }
}
