use rand::{RngExt, distr::Alphanumeric};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::source::SourceError;

const SUBSONIC_API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "kopuz";

/// Budget for an ordinary library call, which the server answers out of its own
/// database.
const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Budget for the similar-songs call. Navidrome answers it from its metadata
/// agents, and when no agent has track-level similarity it falls back to one
/// `getSimilarArtists` lookup plus a sequential top-songs lookup per similar
/// artist, each a live Last.fm round trip on its own 10s budget. Measured
/// against a real library that ranges from 0.15s (warm) to over 50s (cold), so
/// the ordinary budget turned most radio starts into a silent timeout.
const SIMILAR_SONGS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// The OpenSubsonic extension that exposes analysis-backed similarity. A server
/// advertises it only when something implements it (on Navidrome, the
/// AudioMuse-AI plugin); the endpoints return a bare 404 otherwise, so it has to
/// be asked for rather than attempted.
const SONIC_SIMILARITY_EXTENSION: &str = "sonicSimilarity";

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

pub(crate) struct SubsonicClient {
    http_client: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    /// Memoized [`SONIC_SIMILARITY_EXTENSION`] probe. The answer only changes
    /// when the server's plugins do, and radio would otherwise pay for an extra
    /// round trip on every start.
    sonic_similarity: tokio::sync::OnceCell<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicAlbum {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
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
    /// Sent by Subsonic and OpenSubsonic both; absent on a tagless server.
    pub artist_id: Option<String>,
    /// OpenSubsonic only: each artist the song credits, in billing order, where
    /// `artist` holds the joined display string ("A feat. B").
    #[serde(default)]
    pub artists: Vec<SubsonicArtistRef>,
    pub duration: Option<u64>,
    pub bit_rate: Option<u32>,
    pub sampling_rate: Option<u32>,
    pub track: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
    pub replay_gain: Option<SubsonicReplayGain>,
}

/// One entry of an OpenSubsonic song's `artists` list.
#[derive(Debug, Clone, Deserialize)]
pub struct SubsonicArtistRef {
    #[serde(default)]
    pub id: String,
    pub name: String,
}

/// The OpenSubsonic `replayGain` object. `baseGain` is deliberately unread:
/// it exists for Ogg Opus output gain, which the decode worker already applies.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicReplayGain {
    pub track_gain: Option<f32>,
    pub album_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_peak: Option<f32>,
}

impl SubsonicSong {
    pub fn replay_gain_info(&self) -> config::ReplayGainInfo {
        let Some(gain) = &self.replay_gain else {
            return config::ReplayGainInfo::default();
        };
        config::ReplayGainInfo {
            track_gain_db: gain.track_gain.filter(|db| db.is_finite()),
            track_peak: gain
                .track_peak
                .filter(|peak| peak.is_finite() && *peak > 0.0),
            album_gain_db: gain.album_gain.filter(|db| db.is_finite()),
            album_peak: gain
                .album_peak
                .filter(|peak| peak.is_finite() && *peak > 0.0),
            loudness_db: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicPlaylist {
    pub id: String,
    pub name: String,
    pub song_count: Option<u32>,
    pub cover_art: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicArtist {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmptyData {}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtistIndex {
    #[serde(default)]
    artist: Vec<SubsonicArtist>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtistsContainer {
    #[serde(default)]
    index: Vec<ArtistIndex>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetArtistsData {
    #[serde(default)]
    artists: ArtistsContainer,
}

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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimilarSongsContainer {
    #[serde(default)]
    song: Vec<SubsonicSong>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicPlayQueue {
    pub current: Option<String>,
    pub position: Option<u64>,
    pub changed_by: Option<String>,
    #[serde(default)]
    pub entry: Vec<SubsonicSong>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetSimilarSongsData {
    #[serde(default)]
    similar_songs: Option<SimilarSongsContainer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetSongData {
    #[serde(default)]
    song: Option<SubsonicSong>,
}

/// One `getSonicSimilarTracks` hit. The response also carries a `similarity`
/// score, dropped here because the server already returns the list ordered by
/// it and a threshold would second-guess the analysis that produced it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SonicMatch {
    entry: SubsonicSong,
}

/// `sonicMatch` sits directly on the response body, not inside a container.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetSonicSimilarTracksData {
    #[serde(default)]
    sonic_match: Vec<SonicMatch>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenSubsonicExtension {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetOpenSubsonicExtensionsData {
    #[serde(default)]
    open_subsonic_extensions: Vec<OpenSubsonicExtension>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetPlayQueueData {
    #[serde(default)]
    play_queue: Option<SubsonicPlayQueue>,
}

/// Build a Subsonic `getCoverArt` URL without the caller holding a client — the
/// player's synchronous cover path needs it but must not construct a client.
pub fn cover_art_url(
    base_url: &str,
    username: &str,
    password: &str,
    cover_art_id: &str,
    max_size: Option<u32>,
) -> Result<String, String> {
    SubsonicClient::new(base_url, username, password).cover_art_url(cover_art_id, max_size)
}

/// Build a Subsonic `stream` URL at a bitrate cap, client-free — the synchronous
/// offline-download URL builder needs it without constructing a client itself.
pub fn stream_url_with_bitrate(
    base_url: &str,
    username: &str,
    password: &str,
    item_id: &str,
    max_bitrate_kbps: Option<u32>,
) -> Result<String, String> {
    SubsonicClient::new(base_url, username, password)
        .stream_url_with_bitrate(item_id, max_bitrate_kbps)
}

impl SubsonicClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> Self {
        let builder = utils::http::builder();
        let builder = builder.timeout(DEFAULT_TIMEOUT);
        let http_client = builder.build().unwrap_or_else(|_| utils::http::client());

        Self {
            http_client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: crate::provider::resolve_subsonic_secret(password)
                .unwrap_or_else(|| "__missing_subsonic_secret__".to_string()),
            sonic_similarity: tokio::sync::OnceCell::new(),
        }
    }

    pub async fn ping(&self) -> Result<(), SourceError> {
        self.request::<EmptyData>("ping.view", vec![], DEFAULT_TIMEOUT)
            .await
            .map(|_| ())
    }

    pub async fn get_album_list(
        &self,
        offset: usize,
        size: usize,
    ) -> Result<Vec<SubsonicAlbum>, SourceError> {
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

    pub async fn get_album_songs(&self, album_id: &str) -> Result<Vec<SubsonicSong>, SourceError> {
        let data = self
            .call::<GetAlbumData>(
                "getAlbum.view",
                vec![("id".to_string(), album_id.to_string())],
            )
            .await?;
        Ok(data.album.map(|a| a.song).unwrap_or_default())
    }

    /// Songs to build a radio queue from, seeded by one song.
    ///
    /// Server-side audio analysis wins when the server has it: the
    /// `sonicSimilarity` extension answers from the server's own analysis of the
    /// library, which is both better matched and far quicker than the
    /// metadata-agent route. Everything else falls back to plain
    /// `getSimilarSongs`, which every Subsonic server since API 1.11.0 exposes.
    pub async fn get_similar_songs(
        &self,
        song_id: &str,
        count: usize,
    ) -> Result<Vec<SubsonicSong>, SourceError> {
        if self.has_sonic_similarity().await {
            match self.get_sonic_similar_tracks(song_id, count).await {
                Ok(songs) if !songs.is_empty() => return Ok(songs),
                Ok(_) => tracing::debug!(song = %song_id, "sonic similarity returned nothing"),
                Err(e) => tracing::warn!(song = %song_id, error = %e, "sonic similarity failed"),
            }
        }
        self.get_similar_songs_by_metadata(song_id, count).await
    }

    /// Whether the server advertises [`SONIC_SIMILARITY_EXTENSION`]. Probed once
    /// per client; a failed probe is taken as "no" so radio still falls back
    /// rather than erroring on an unrelated call.
    async fn has_sonic_similarity(&self) -> bool {
        *self
            .sonic_similarity
            .get_or_init(|| async {
                match self
                    .call::<GetOpenSubsonicExtensionsData>("getOpenSubsonicExtensions.view", vec![])
                    .await
                {
                    Ok(data) => data
                        .open_subsonic_extensions
                        .iter()
                        .any(|ext| ext.name == SONIC_SIMILARITY_EXTENSION),
                    Err(e) => {
                        tracing::debug!(error = %e, "OpenSubsonic extension probe failed");
                        false
                    }
                }
            })
            .await
    }

    /// OpenSubsonic `getSonicSimilarTracks`. Results come back ordered by
    /// similarity, so the queue order is the server's ranking.
    async fn get_sonic_similar_tracks(
        &self,
        song_id: &str,
        count: usize,
    ) -> Result<Vec<SubsonicSong>, SourceError> {
        let data = self
            .call_within::<GetSonicSimilarTracksData>(
                "getSonicSimilarTracks.view",
                vec![
                    ("id".to_string(), song_id.to_string()),
                    ("count".to_string(), count.to_string()),
                ],
                SIMILAR_SONGS_TIMEOUT,
            )
            .await?;
        Ok(data.sonic_match.into_iter().map(|m| m.entry).collect())
    }

    /// Subsonic `getSimilarSongs` (API 1.11.0). This is the variant whose `id`
    /// accepts a song; `getSimilarSongs2` is specified as artist-seeded, and
    /// only happens to accept a song id on servers that alias the two.
    async fn get_similar_songs_by_metadata(
        &self,
        song_id: &str,
        count: usize,
    ) -> Result<Vec<SubsonicSong>, SourceError> {
        let data = self
            .call_within::<GetSimilarSongsData>(
                "getSimilarSongs.view",
                vec![
                    ("id".to_string(), song_id.to_string()),
                    ("count".to_string(), count.to_string()),
                ],
                SIMILAR_SONGS_TIMEOUT,
            )
            .await?;
        Ok(data.similar_songs.map(|s| s.song).unwrap_or_default())
    }

    /// One song by id. Radio uses it to put the seed at the head of the queue:
    /// `getSimilarSongs2` never includes the seed itself.
    pub async fn get_song(&self, song_id: &str) -> Result<Option<SubsonicSong>, SourceError> {
        let data = self
            .call::<GetSongData>(
                "getSong.view",
                vec![("id".to_string(), song_id.to_string())],
            )
            .await?;
        Ok(data.song)
    }

    pub async fn get_playlists(&self) -> Result<Vec<SubsonicPlaylist>, SourceError> {
        let data = self
            .call::<GetPlaylistsData>("getPlaylists.view", vec![])
            .await?;
        Ok(data.playlists.playlist)
    }

    pub async fn get_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<SubsonicSong>, SourceError> {
        let data = self
            .call::<GetPlaylistData>(
                "getPlaylist.view",
                vec![("id".to_string(), playlist_id.to_string())],
            )
            .await?;
        Ok(data.playlist.map(|p| p.entry).unwrap_or_default())
    }

    pub async fn create_playlist(
        &self,
        name: &str,
        item_ids: &[&str],
    ) -> Result<String, SourceError> {
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

        Err(SourceError::Backend(
            "Subsonic createPlaylist did not return a playlist id".to_string(),
        ))
    }

    pub async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_id: &str,
    ) -> Result<(), SourceError> {
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
    ) -> Result<(), SourceError> {
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

    pub async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_song_ids: &[&str],
        total_tracks: usize,
    ) -> Result<(), SourceError> {
        let mut params: Vec<(String, String)> =
            vec![("playlistId".to_string(), playlist_id.to_string())];
        for i in 0..total_tracks {
            params.push(("songIndexToRemove".to_string(), i.to_string()));
        }
        for id in ordered_song_ids {
            params.push(("songIdToAdd".to_string(), (*id).to_string()));
        }
        self.call::<EmptyData>("updatePlaylist.view", params)
            .await
            .map(|_| ())
    }

    pub async fn get_artists(&self) -> Result<Vec<SubsonicArtist>, SourceError> {
        let data = self
            .call::<GetArtistsData>("getArtists.view", vec![])
            .await?;
        Ok(data
            .artists
            .index
            .into_iter()
            .flat_map(|idx| idx.artist)
            .collect())
    }

    pub async fn get_starred_song_ids(&self) -> Result<Vec<String>, SourceError> {
        let data = self
            .call::<GetStarred2Data>("getStarred2.view", vec![])
            .await?;
        Ok(data
            .starred2
            .map(|s| s.song.into_iter().map(|song| song.id).collect())
            .unwrap_or_default())
    }

    pub async fn star(&self, item_id: &str) -> Result<(), SourceError> {
        self.call::<EmptyData>("star.view", vec![("id".to_string(), item_id.to_string())])
            .await
            .map(|_| ())
    }

    pub async fn unstar(&self, item_id: &str) -> Result<(), SourceError> {
        self.call::<EmptyData>("unstar.view", vec![("id".to_string(), item_id.to_string())])
            .await
            .map(|_| ())
    }

    pub fn stream_url(&self, item_id: &str) -> Result<String, String> {
        self.stream_url_with_bitrate(item_id, None)
    }

    pub fn stream_url_with_bitrate(
        &self,
        item_id: &str,
        max_bitrate_kbps: Option<u32>,
    ) -> Result<String, String> {
        let mut url = reqwest::Url::parse(&format!("{}/rest/stream.view", self.base_url))
            .map_err(|e| format!("Invalid Subsonic base URL '{}': {}", self.base_url, e))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in self.auth_params() {
                pairs.append_pair(&k, &v);
            }
            pairs.append_pair("id", item_id);
            if let Some(kbps) = max_bitrate_kbps {
                pairs.append_pair("maxBitRate", &kbps.to_string());
                if kbps > 0 {
                    pairs.append_pair("format", "mp3");
                }
            }
        }
        Ok(url.to_string())
    }

    pub async fn scrobble_now_playing(&self, item_id: &str) -> Result<(), SourceError> {
        self.call::<EmptyData>(
            "scrobble.view",
            vec![
                ("id".to_string(), item_id.to_string()),
                ("submission".to_string(), "false".to_string()),
            ],
        )
        .await
        .map(|_| ())
    }

    pub async fn scrobble(&self, item_id: &str) -> Result<(), SourceError> {
        self.call::<EmptyData>(
            "scrobble.view",
            vec![
                ("id".to_string(), item_id.to_string()),
                ("submission".to_string(), "true".to_string()),
            ],
        )
        .await
        .map(|_| ())
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

    pub async fn get_play_queue(&self) -> Result<Option<SubsonicPlayQueue>, SourceError> {
        let data = self
            .call::<GetPlayQueueData>("getPlayQueue.view", vec![])
            .await?;
        Ok(data.play_queue)
    }

    /// An empty `item_ids` with no `current` clears the saved queue.
    pub async fn save_play_queue(
        &self,
        item_ids: &[&str],
        current_id: Option<&str>,
        position_ms: Option<u64>,
    ) -> Result<(), SourceError> {
        let mut params: Vec<(String, String)> = item_ids
            .iter()
            .map(|id| ("id".to_string(), (*id).to_string()))
            .collect();
        if let Some(current) = current_id {
            params.push(("current".to_string(), current.to_string()));
        }
        if let Some(position) = position_ms {
            params.push(("position".to_string(), position.to_string()));
        }
        self.call::<EmptyData>("savePlayQueue.view", params)
            .await
            .map(|_| ())
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
        rand::rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect()
    }

    async fn call<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        extra_params: Vec<(String, String)>,
    ) -> Result<T, SourceError> {
        self.call_within(endpoint, extra_params, DEFAULT_TIMEOUT)
            .await
    }

    async fn call_within<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        extra_params: Vec<(String, String)>,
        timeout: std::time::Duration,
    ) -> Result<T, SourceError> {
        self.request(endpoint, extra_params, timeout).await
    }

    #[tracing::instrument(name = "subsonic.call", skip_all, fields(endpoint = %endpoint))]
    async fn request<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        mut extra_params: Vec<(String, String)>,
        timeout: std::time::Duration,
    ) -> Result<T, SourceError> {
        let url = format!("{}/rest/{}", self.base_url, endpoint);

        let mut params = self.auth_params();
        params.append(&mut extra_params);

        let resp = self
            .http_client
            .get(&url)
            .query(&params)
            .timeout(timeout)
            .send()
            .await
            .map_err(SourceError::from_request)?;

        if !resp.status().is_success() {
            return Err(SourceError::from_status(resp.status(), "Subsonic request"));
        }

        let parsed: SubsonicEnvelope<T> = resp
            .json()
            .await
            .map_err(|error| SourceError::Backend(redacted(error)))?;

        if parsed.response.status.eq_ignore_ascii_case("ok") {
            return Ok(parsed.response.data);
        }

        match parsed.response.error {
            Some(err) => Err(envelope_error(err.code, &err.message)),
            None => Err(SourceError::Backend(
                "Subsonic request failed with unknown error".to_string(),
            )),
        }
    }
}

/// 40 is wrong credentials, 44 an invalid API key (OpenSubsonic) and 50 an account not allowed to do it.
fn envelope_error(code: i32, message: &str) -> SourceError {
    match code {
        40 | 44 | 50 => SourceError::Auth,
        _ => SourceError::Backend(format!("Subsonic request failed ({code}): {message}")),
    }
}

/// A request error as text without its URL, whose query carries the user name, token and salt.
fn redacted(error: reqwest::Error) -> String {
    error.without_url().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_no_credentials(error: &str) {
        for secret in ["someone", "u=", "t=", "s="] {
            assert!(!error.contains(secret), "{secret} in {error}");
        }
    }

    #[tokio::test]
    async fn a_failed_call_keeps_the_credentials_out_of_its_error() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = closed.local_addr().expect("address");
        drop(closed);
        let unreachable = SubsonicClient::new(&format!("http://{address}"), "someone", "hunter2");
        let error = unreachable.ping().await.expect_err("nothing listens");
        assert_eq!(error, SourceError::Connectivity);
        assert_no_credentials(&error.to_string());
        assert_eq!(
            unreachable.get_album_list(0, 10).await.err(),
            Some(SourceError::Connectivity),
            "a library call keeps the kind too"
        );

        let server = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = server.local_addr().expect("address");
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok((mut stream, _)) = server.accept().await {
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnot",
                    )
                    .await;
            }
        });
        let garbled = SubsonicClient::new(&format!("http://{address}"), "someone", "hunter2");
        let error = garbled.ping().await.expect_err("not json");
        assert_no_credentials(&error.to_string());
    }

    /// One canned HTTP answer on a local port, for a client to call.
    async fn answering(status: &str, body: &str) -> String {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = server.local_addr().expect("address");
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok((mut stream, _)) = server.accept().await {
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn wrong_credentials_are_an_auth_failure() {
        let body = r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password"}}}"#;
        let url = answering("200 OK", body).await;
        let client = SubsonicClient::new(&url, "someone", "hunter2");
        assert_eq!(client.ping().await, Err(SourceError::Auth));

        let url = answering("401 Unauthorized", "").await;
        let client = SubsonicClient::new(&url, "someone", "hunter2");
        assert_eq!(client.ping().await, Err(SourceError::Auth));
    }

    #[test]
    fn only_a_credential_code_is_an_auth_failure() {
        for code in [40, 44, 50] {
            assert_eq!(envelope_error(code, "m"), SourceError::Auth, "{code}");
        }
        for code in [0, 10, 30, 41, 70] {
            assert!(
                matches!(envelope_error(code, "m"), SourceError::Backend(_)),
                "{code}"
            );
        }
    }

    fn parse<T: DeserializeOwned + Default>(body: serde_json::Value) -> T {
        let envelope: SubsonicEnvelope<T> =
            serde_json::from_value(body).expect("valid Subsonic envelope");
        envelope.response.data
    }

    /// The radio path reads these shapes and nothing else. A rename or a wrong
    /// nesting level deserializes to an empty list rather than an error, which
    /// reaches the user as a radio button that does nothing.
    #[test]
    fn similar_songs_response_parses() {
        let data: GetSimilarSongsData = parse(serde_json::json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "similarSongs": {
                    "song": [{
                        "id": "song-1",
                        "title": "Feels Like We Only Go Backwards",
                        "artist": "Tame Impala",
                        "albumId": "album-1",
                        "duration": 193,
                        "coverArt": "cover-1"
                    }]
                }
            }
        }));

        let songs = data.similar_songs.expect("similarSongs container").song;
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].id, "song-1");
    }

    /// `sonicMatch` sits on the response body itself and wraps each song in an
    /// `entry`, unlike every other list this client reads.
    #[test]
    fn sonic_similar_tracks_response_parses() {
        let data: GetSonicSimilarTracksData = parse(serde_json::json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "openSubsonic": true,
                "sonicMatch": [{
                    "entry": {
                        "id": "song-2",
                        "title": "The Less I Know the Better",
                        "artist": "Tame Impala"
                    },
                    "similarity": 0.95
                }]
            }
        }));

        assert_eq!(data.sonic_match.len(), 1);
        assert_eq!(data.sonic_match[0].entry.id, "song-2");
    }

    #[test]
    fn extension_probe_reads_the_advertised_names() {
        let data: GetOpenSubsonicExtensionsData = parse(serde_json::json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "openSubsonic": true,
                "openSubsonicExtensions": [
                    { "name": "songLyrics", "versions": [1, 2] },
                    { "name": "sonicSimilarity", "versions": [1] }
                ]
            }
        }));

        assert!(
            data.open_subsonic_extensions
                .iter()
                .any(|ext| ext.name == SONIC_SIMILARITY_EXTENSION)
        );
    }

    /// The shape a server with no similarity plugin returns (captured from
    /// Navidrome 0.63.2): radio has to fall back rather than treat it as an error.
    #[test]
    fn extension_probe_without_sonic_similarity() {
        let data: GetOpenSubsonicExtensionsData = parse(serde_json::json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "openSubsonic": true,
                "openSubsonicExtensions": [
                    { "name": "transcodeOffset", "versions": [1] },
                    { "name": "formPost", "versions": [1] },
                    { "name": "songLyrics", "versions": [1, 2] }
                ]
            }
        }));

        assert!(
            !data
                .open_subsonic_extensions
                .iter()
                .any(|ext| ext.name == SONIC_SIMILARITY_EXTENSION)
        );
    }

    #[test]
    fn reads_the_opensubsonic_replaygain_object() {
        let song: SubsonicSong = serde_json::from_str(
            r#"{
                "id": "1",
                "title": "T",
                "replayGain": {
                    "trackGain": -7.5,
                    "albumGain": -5.25,
                    "trackPeak": 0.98,
                    "baseGain": 0
                }
            }"#,
        )
        .unwrap();

        let info = song.replay_gain_info();
        assert_eq!(info.track_gain_db, Some(-7.5));
        assert_eq!(info.album_gain_db, Some(-5.25));
        assert_eq!(info.track_peak, Some(0.98));
        assert_eq!(info.album_peak, None);
    }

    #[test]
    fn a_song_without_the_object_reports_nothing() {
        let song: SubsonicSong = serde_json::from_str(r#"{"id": "1", "title": "T"}"#).unwrap();
        assert!(song.replay_gain_info().is_empty());
    }

    /// Captured from Navidrome 0.63.2, trimmed to the fields this crate reads.
    #[test]
    fn play_queue_response_parses() {
        let data: GetPlayQueueData = parse(serde_json::json!({
            "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "playQueue": {
                    "entry": [{
                        "id": "BgdZpTKEu8u6xLGYsZCCXq",
                        "title": "Expectation",
                        "album": "Innerspeaker",
                        "albumId": "7tbDxWiXuAInyHqxBR8ukM",
                        "artist": "Tame Impala",
                        "duration": 362,
                        "bitRate": 901,
                        "coverArt": "mf-BgdZpTKEu8u6xLGYsZCCXq_6a69cd29"
                    }],
                    "current": "BgdZpTKEu8u6xLGYsZCCXq",
                    "position": 45000,
                    "username": "someone",
                    "changed": "2026-08-09T18:08:10.49647068Z",
                    "changedBy": "Arpeggi"
                }
            }
        }));

        let queue = data.play_queue.expect("playQueue present");
        assert_eq!(queue.current.as_deref(), Some("BgdZpTKEu8u6xLGYsZCCXq"));
        assert_eq!(queue.position, Some(45000));
        assert_eq!(queue.changed_by.as_deref(), Some("Arpeggi"));
        assert_eq!(queue.entry.len(), 1);
        assert_eq!(queue.entry[0].title, "Expectation");
    }

    #[test]
    fn play_queue_absent_when_nothing_saved() {
        let data: GetPlayQueueData = parse(serde_json::json!({
            "subsonic-response": { "status": "ok", "version": "1.16.1" }
        }));

        assert!(data.play_queue.is_none());
    }
}
