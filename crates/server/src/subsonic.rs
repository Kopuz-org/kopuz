use rand::{RngExt, distr::Alphanumeric};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};

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

enum CallFailure {
    Api { code: i32, message: String },
    Transport(String),
}

impl CallFailure {
    fn into_message(self) -> String {
        match self {
            CallFailure::Api { code, message } => {
                format!("Subsonic request failed ({code}): {message}")
            }
            CallFailure::Transport(message) => message,
        }
    }
}

/// Bad credentials (40), token auth unsupported (41), mechanism (42, 43).
const AUTH_REJECTED_CODES: [i32; 4] = [40, 41, 42, 43];

/// Global so the throwaway clients behind the URL builders see it too.
static LEGACY_AUTH_SERVERS: LazyLock<RwLock<HashSet<String>>> = LazyLock::new(Default::default);

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
        let builder = reqwest::Client::builder();
        let builder = builder.timeout(DEFAULT_TIMEOUT);
        let http_client = builder.build().unwrap_or_else(|_| reqwest::Client::new());

        Self {
            http_client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: crate::provider::resolve_subsonic_secret(password)
                .unwrap_or_else(|| "__missing_subsonic_secret__".to_string()),
            sonic_similarity: tokio::sync::OnceCell::new(),
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
    ) -> Result<Vec<SubsonicSong>, String> {
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
    ) -> Result<Vec<SubsonicSong>, String> {
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
    ) -> Result<Vec<SubsonicSong>, String> {
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
    pub async fn get_song(&self, song_id: &str) -> Result<Option<SubsonicSong>, String> {
        let data = self
            .call::<GetSongData>(
                "getSong.view",
                vec![("id".to_string(), song_id.to_string())],
            )
            .await?;
        Ok(data.song)
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

    pub async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered_song_ids: &[&str],
        total_tracks: usize,
    ) -> Result<(), String> {
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

    pub async fn get_artists(&self) -> Result<Vec<SubsonicArtist>, String> {
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

    pub async fn scrobble_now_playing(&self, item_id: &str) -> Result<(), String> {
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

    pub async fn scrobble(&self, item_id: &str) -> Result<(), String> {
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

    fn auth_params(&self) -> Vec<(String, String)> {
        if self.uses_legacy_auth() && self.allows_legacy_auth() {
            self.legacy_auth_params()
        } else {
            self.token_auth_params()
        }
    }

    /// `p=enc:` is reversible and rides in the query string, so it is never
    /// constructed or sent for a non-HTTPS base URL.
    fn allows_legacy_auth(&self) -> bool {
        self.base_url.starts_with("https://")
    }

    fn token_auth_params(&self) -> Vec<(String, String)> {
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

    fn legacy_auth_params(&self) -> Vec<(String, String)> {
        vec![
            ("u".to_string(), self.username.clone()),
            (
                "p".to_string(),
                format!("enc:{}", hex::encode(self.password.as_bytes())),
            ),
            ("v".to_string(), SUBSONIC_API_VERSION.to_string()),
            ("c".to_string(), CLIENT_NAME.to_string()),
            ("f".to_string(), "json".to_string()),
        ]
    }

    fn legacy_auth_key(&self) -> String {
        format!("{}\n{}", self.base_url, self.username)
    }

    fn uses_legacy_auth(&self) -> bool {
        LEGACY_AUTH_SERVERS
            .read()
            .map(|servers| servers.contains(&self.legacy_auth_key()))
            .unwrap_or(false)
    }

    fn remember_legacy_auth(&self) {
        if let Ok(mut servers) = LEGACY_AUTH_SERVERS.write() {
            servers.insert(self.legacy_auth_key());
        }
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
    ) -> Result<T, String> {
        self.call_within(endpoint, extra_params, DEFAULT_TIMEOUT)
            .await
    }

    #[tracing::instrument(name = "subsonic.call", skip_all, fields(endpoint = %endpoint))]
    async fn call_within<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        extra_params: Vec<(String, String)>,
        timeout: std::time::Duration,
    ) -> Result<T, String> {
        let url = format!("{}/rest/{}", self.base_url, endpoint);

        let first = self
            .request::<T>(&url, self.auth_params(), &extra_params, timeout)
            .await;

        let retry_auth = matches!(
            &first,
            Err(CallFailure::Api { code, .. })
                if AUTH_REJECTED_CODES.contains(code) && !self.uses_legacy_auth()
        );
        if !retry_auth {
            return first.map_err(CallFailure::into_message);
        }
        if !self.allows_legacy_auth() {
            tracing::warn!(
                "server rejected token auth; not falling back over plain http, the legacy scheme would put the password in the URL"
            );
            return first.map_err(CallFailure::into_message);
        }

        tracing::debug!("token auth rejected, retrying with the legacy password scheme");
        let retry = self
            .request::<T>(&url, self.legacy_auth_params(), &extra_params, timeout)
            .await;
        if retry.is_ok() {
            self.remember_legacy_auth();
        }
        retry.map_err(CallFailure::into_message)
    }

    async fn request<T: DeserializeOwned + Default>(
        &self,
        url: &str,
        mut params: Vec<(String, String)>,
        extra_params: &[(String, String)],
        timeout: std::time::Duration,
    ) -> Result<T, CallFailure> {
        params.extend_from_slice(extra_params);

        let resp = self
            .http_client
            .get(url)
            .query(&params)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| CallFailure::Transport(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(CallFailure::Transport(format!(
                "Subsonic request failed: {}",
                resp.status()
            )));
        }

        let parsed: SubsonicEnvelope<T> = resp
            .json()
            .await
            .map_err(|e| CallFailure::Transport(e.to_string()))?;

        if parsed.response.status.eq_ignore_ascii_case("ok") {
            return Ok(parsed.response.data);
        }

        match parsed.response.error {
            Some(err) => Err(CallFailure::Api {
                code: err.code,
                message: err.message,
            }),
            None => Err(CallFailure::Transport(
                "Subsonic request failed with unknown error".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn legacy_auth_params_hex_encode_the_password() {
        let client = SubsonicClient::new("https://music.example.test", "user", "password");
        let params = client.legacy_auth_params();

        let p = params.iter().find(|(k, _)| k == "p").map(|(_, v)| v);
        assert_eq!(p, Some(&"enc:70617373776f7264".to_string()));
        assert!(!params.iter().any(|(k, _)| k == "t" || k == "s"));
    }

    /// A server that refuses every credential with `code`, recording what it saw.
    async fn serve_rejecting(
        listener: tokio::net::TcpListener,
        code: i32,
        seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        loop {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf).await.expect("read");
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            seen.lock()
                .expect("seen lock")
                .push(String::from_utf8_lossy(&req).into_owned());

            let body = format!(
                r#"{{"subsonic-response":{{"status":"failed","version":"1.16.1","error":{{"code":{code},"message":"rejected"}}}}}}"#
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.expect("write");
        }
    }

    /// The legacy scheme would put a reversible password on an unencrypted
    /// wire, so a plain-http server that refuses the token gets no second try.
    #[tokio::test]
    async fn plain_http_never_sends_the_legacy_password() {
        for code in [41, 40] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().expect("local addr");
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            tokio::spawn(serve_rejecting(listener, code, seen.clone()));

            let client = SubsonicClient::new(&format!("http://{addr}"), "user", "password");
            let err = client
                .ping()
                .await
                .expect_err("plain http must not fall back");
            assert!(err.contains(&format!("({code})")), "{err}");

            let seen = seen.lock().expect("seen lock");
            assert_eq!(seen.len(), 1, "code {code} must not be retried");
            assert!(!seen.iter().any(|req| req.contains("p=enc")));
        }
    }

    #[test]
    fn sticky_legacy_auth_applies_only_over_https() {
        let secure = SubsonicClient::new("https://secure.example.test", "user", "password");
        let plain = SubsonicClient::new("http://plain.example.test", "user", "password");
        secure.remember_legacy_auth();
        plain.remember_legacy_auth();

        assert!(
            secure
                .auth_params()
                .iter()
                .any(|(k, v)| k == "p" && v.starts_with("enc:"))
        );
        assert!(secure.stream_url("song-1").expect("url").contains("p=enc"));
        assert!(
            secure
                .cover_art_url("cover-1", None)
                .expect("url")
                .contains("p=enc")
        );

        let plain_params = plain.auth_params();
        assert!(!plain_params.iter().any(|(k, _)| k == "p"));
        assert!(plain_params.iter().any(|(k, _)| k == "t"));
        assert!(!plain.stream_url("song-1").expect("url").contains("p=enc"));
        assert!(
            !plain
                .cover_art_url("cover-1", None)
                .expect("url")
                .contains("p=enc")
        );
    }
}
