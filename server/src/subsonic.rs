use rand::{Rng, distributions::Alphanumeric};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const SUBSONIC_API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "rusic";

#[derive(Debug, Deserialize)]
struct SubsonicEnvelope<T> {
    #[serde(rename = "subsonic-response")]
    response: SubsonicResponse<T>,
}

#[derive(Debug, Deserialize)]
struct SubsonicResponse<T> {
    status: String,
    #[serde(default)]
    error: Option<SubsonicError>,
    #[serde(flatten)]
    data: T,
}

#[derive(Debug, Deserialize)]
struct SubsonicError {
    code: i32,
    message: String,
}

pub struct SubsonicClient {
    http_client: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicAlbum {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u16>,
    pub cover_art: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicSong {
    pub id: String,
    pub title: String,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub artist: Option<String>,
    pub duration: Option<u64>,
    pub bit_rate: Option<u32>,
    pub sampling_rate: Option<u32>,
    pub track: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicPlaylist {
    pub id: String,
    pub name: String,
    pub song_count: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmptyData {}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumList2Container {
    #[serde(default)]
    album: Vec<SubsonicAlbum>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetAlbumList2Data {
    #[serde(default, rename = "albumList2")]
    album_list2: AlbumList2Container,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumSongsContainer {
    #[serde(default)]
    song: Vec<SubsonicSong>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetAlbumData {
    #[serde(default)]
    album: Option<AlbumSongsContainer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistsContainer {
    #[serde(default)]
    playlist: Vec<SubsonicPlaylist>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetPlaylistsData {
    #[serde(default)]
    playlists: PlaylistsContainer,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistEntriesContainer {
    #[serde(default)]
    entry: Vec<SubsonicSong>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetPlaylistData {
    #[serde(default)]
    playlist: Option<PlaylistEntriesContainer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StarredSongsContainer {
    #[serde(default)]
    song: Vec<SubsonicSong>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetStarred2Data {
    #[serde(default)]
    starred2: Option<StarredSongsContainer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistCreationData {
    #[serde(default)]
    playlist: Option<SubsonicPlaylist>,
}

impl SubsonicClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            http_client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: crate::provider::resolve_subsonic_secret(password)
                .unwrap_or_else(|| "__missing_subsonic_secret__".to_string()),
        }
    }

    pub async fn ping(&self) -> Result<(), String> {
        self.call::<EmptyData>("ping.view", vec![])
            .await
            .map(|_| ())
    }

    pub async fn get_album_list(
        &self,
        offset: usize,
        size: usize,
    ) -> Result<Vec<SubsonicAlbum>, String> {
        let data = self
            .call::<GetAlbumList2Data>(
                "getAlbumList2.view",
                vec![
                    ("type".to_string(), "alphabeticalByName".to_string()),
                    ("offset".to_string(), offset.to_string()),
                    ("size".to_string(), size.to_string()),
                ],
            )
            .await?;
        Ok(data.album_list2.album)
    }

    pub async fn get_album_songs(&self, album_id: &str) -> Result<Vec<SubsonicSong>, String> {
        let data = self
            .call::<GetAlbumData>(
                "getAlbum.view",
                vec![("id".to_string(), album_id.to_string())],
            )
            .await?;
        Ok(data.album.map(|a| a.song).unwrap_or_default())
    }

    pub async fn get_playlists(&self) -> Result<Vec<SubsonicPlaylist>, String> {
        let data = self
            .call::<GetPlaylistsData>("getPlaylists.view", vec![])
            .await?;
        Ok(data.playlists.playlist)
    }

    pub async fn get_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<SubsonicSong>, String> {
        let data = self
            .call::<GetPlaylistData>(
                "getPlaylist.view",
                vec![("id".to_string(), playlist_id.to_string())],
            )
            .await?;
        Ok(data.playlist.map(|p| p.entry).unwrap_or_default())
    }

    pub async fn create_playlist(&self, name: &str, item_ids: &[&str]) -> Result<String, String> {
        let mut params = vec![("name".to_string(), name.to_string())];
        for item_id in item_ids {
            params.push(("songId".to_string(), (*item_id).to_string()));
        }

        let data = self
            .call::<PlaylistCreationData>("createPlaylist.view", params)
            .await?;

        if let Some(playlist) = data.playlist {
            return Ok(playlist.id);
        }

        Err("Subsonic createPlaylist did not return a playlist id".to_string())
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, item_id: &str) -> Result<(), String> {
        self.call::<EmptyData>(
            "updatePlaylist.view",
            vec![
                ("playlistId".to_string(), playlist_id.to_string()),
                ("songIdToAdd".to_string(), item_id.to_string()),
            ],
        )
        .await
        .map(|_| ())
    }

    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        song_index: usize,
    ) -> Result<(), String> {
        self.call::<EmptyData>(
            "updatePlaylist.view",
            vec![
                ("playlistId".to_string(), playlist_id.to_string()),
                ("songIndexToRemove".to_string(), song_index.to_string()),
            ],
        )
        .await
        .map(|_| ())
    }

    pub async fn get_starred_song_ids(&self) -> Result<Vec<String>, String> {
        let data = self
            .call::<GetStarred2Data>("getStarred2.view", vec![])
            .await?;
        Ok(data
            .starred2
            .map(|s| s.song.into_iter().map(|song| song.id).collect())
            .unwrap_or_default())
    }

    pub async fn star(&self, item_id: &str) -> Result<(), String> {
        self.call::<EmptyData>("star.view", vec![("id".to_string(), item_id.to_string())])
            .await
            .map(|_| ())
    }

    pub async fn unstar(&self, item_id: &str) -> Result<(), String> {
        self.call::<EmptyData>("unstar.view", vec![("id".to_string(), item_id.to_string())])
            .await
            .map(|_| ())
    }

    pub fn stream_url(&self, item_id: &str) -> Result<String, String> {
        let mut url = reqwest::Url::parse(&format!("{}/rest/stream.view", self.base_url))
            .map_err(|e| format!("Invalid Subsonic base URL '{}': {}", self.base_url, e))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in self.auth_params() {
                pairs.append_pair(&k, &v);
            }
            pairs.append_pair("id", item_id);
        }
        Ok(url.to_string())
    }

    pub fn cover_art_url(
        &self,
        cover_art_id: &str,
        max_size: Option<u32>,
    ) -> Result<String, String> {
        let mut url = reqwest::Url::parse(&format!("{}/rest/getCoverArt.view", self.base_url))
            .map_err(|e| format!("Invalid Subsonic base URL '{}': {}", self.base_url, e))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in self.auth_params() {
                pairs.append_pair(&k, &v);
            }
            pairs.append_pair("id", cover_art_id);
            if let Some(size) = max_size {
                pairs.append_pair("size", &size.to_string());
            }
        }
        Ok(url.to_string())
    }

    fn auth_params(&self) -> Vec<(String, String)> {
        let salt = self.random_salt();
        let token_input = format!("{}{}", self.password, salt);
        let token = format!("{:x}", md5::compute(token_input));

        vec![
            ("u".to_string(), self.username.clone()),
            ("t".to_string(), token),
            ("s".to_string(), salt),
            ("v".to_string(), SUBSONIC_API_VERSION.to_string()),
            ("c".to_string(), CLIENT_NAME.to_string()),
            ("f".to_string(), "json".to_string()),
        ]
    }

    fn random_salt(&self) -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect()
    }

    async fn call<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        mut extra_params: Vec<(String, String)>,
    ) -> Result<T, String> {
        let url = format!("{}/rest/{}", self.base_url, endpoint);

        let mut params = self.auth_params();
        params.append(&mut extra_params);

        let resp = self
            .http_client
            .get(&url)
            .query(&params)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!("Subsonic request failed: {}", resp.status()));
        }

        let parsed: SubsonicEnvelope<T> = resp.json().await.map_err(|e| e.to_string())?;

        if parsed.response.status.eq_ignore_ascii_case("ok") {
            return Ok(parsed.response.data);
        }

        if let Some(err) = parsed.response.error {
            return Err(format!(
                "Subsonic request failed ({}): {}",
                err.code, err.message
            ));
        }

        Err("Subsonic request failed with unknown error".to_string())
    }
}
