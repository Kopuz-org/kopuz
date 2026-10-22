use std::cell::OnceCell;
use std::time::Duration;

use kopuz_plugin_sdk::http::{self, Client};
use kopuz_plugin_sdk::{ReplayGain, SourceError};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const SUBSONIC_API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "kopuz";

/// Budget for an ordinary library call, which the server answers out of its own
/// database.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Budget for the similar-songs call. Navidrome answers it from its metadata
/// agents, and when no agent has track-level similarity it falls back to one
/// `getSimilarArtists` lookup plus a sequential top-songs lookup per similar
/// artist, each a live Last.fm round trip on its own 10s budget. Measured
/// against a real library that ranges from 0.15s (warm) to over 50s (cold), so
/// the ordinary budget turned most radio starts into a silent timeout.
const SIMILAR_SONGS_TIMEOUT: Duration = Duration::from_secs(90);

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
    http: Client,
    base_url: String,
    username: String,
    password: String,
    /// Memoized [`SONIC_SIMILARITY_EXTENSION`] probe. The answer only changes
    /// when the server's plugins do, and radio would otherwise pay for an extra
    /// round trip on every start.
    sonic_similarity: OnceCell<bool>,
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
    pub fn replay_gain(&self) -> ReplayGain {
        let Some(gain) = &self.replay_gain else {
            return ReplayGain::default();
        };
        ReplayGain {
            track_gain_db: gain.track_gain.filter(|db| db.is_finite()),
            track_peak: gain
                .track_peak
                .filter(|peak| peak.is_finite() && *peak > 0.0),
            album_gain_db: gain.album_gain.filter(|db| db.is_finite()),
            album_peak: gain
                .album_peak
                .filter(|peak| peak.is_finite() && *peak > 0.0),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsonicPlaylist {
    pub id: String,
    pub name: String,
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

impl SubsonicClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> Self {
        Self {
            http: Client::new().with_timeout(DEFAULT_TIMEOUT),
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: if password.is_empty() {
                "__missing_subsonic_secret__".to_string()
            } else {
                password.to_string()
            },
            sonic_similarity: OnceCell::new(),
        }
    }

    pub fn ping(&self) -> Result<(), SourceError> {
        self.call::<EmptyData>("ping.view", Vec::new()).map(|_| ())
    }

    pub fn get_album_list(
        &self,
        offset: usize,
        size: usize,
    ) -> Result<Vec<SubsonicAlbum>, SourceError> {
        let data = self.call::<GetAlbumList2Data>(
            "getAlbumList2.view",
            vec![
                ("type".to_string(), "alphabeticalByName".to_string()),
                ("offset".to_string(), offset.to_string()),
                ("size".to_string(), size.to_string()),
            ],
        )?;
        Ok(data.album_list2.album)
    }

    pub fn get_album_songs(&self, album_id: &str) -> Result<Vec<SubsonicSong>, SourceError> {
        let data = self.call::<GetAlbumData>(
            "getAlbum.view",
            vec![("id".to_string(), album_id.to_string())],
        )?;
        Ok(data.album.map(|a| a.song).unwrap_or_default())
    }

    /// Songs to build a radio queue from, seeded by one song.
    ///
    /// Server-side audio analysis wins when the server has it: the
    /// `sonicSimilarity` extension answers from the server's own analysis of the
    /// library, which is both better matched and far quicker than the
    /// metadata-agent route. Everything else falls back to plain
    /// `getSimilarSongs`, which every Subsonic server since API 1.11.0 exposes.
    pub fn get_similar_songs(
        &self,
        song_id: &str,
        count: usize,
    ) -> Result<Vec<SubsonicSong>, SourceError> {
        if self.has_sonic_similarity() {
            match self.get_sonic_similar_tracks(song_id, count) {
                Ok(songs) if !songs.is_empty() => return Ok(songs),
                Ok(_) => {
                    kopuz_plugin_sdk::debug!("sonic similarity returned nothing for {song_id}")
                }
                Err(e) => kopuz_plugin_sdk::warn!("sonic similarity failed for {song_id}: {e}"),
            }
        }
        self.get_similar_songs_by_metadata(song_id, count)
    }

    /// Whether the server advertises [`SONIC_SIMILARITY_EXTENSION`]. Probed once
    /// per client; a failed probe is taken as "no" so radio still falls back
    /// rather than erroring on an unrelated call.
    fn has_sonic_similarity(&self) -> bool {
        *self.sonic_similarity.get_or_init(|| {
            match self
                .call::<GetOpenSubsonicExtensionsData>("getOpenSubsonicExtensions.view", Vec::new())
            {
                Ok(data) => data
                    .open_subsonic_extensions
                    .iter()
                    .any(|ext| ext.name == SONIC_SIMILARITY_EXTENSION),
                Err(e) => {
                    kopuz_plugin_sdk::debug!("OpenSubsonic extension probe failed: {e}");
                    false
                }
            }
        })
    }

    /// OpenSubsonic `getSonicSimilarTracks`. Results come back ordered by
    /// similarity, so the queue order is the server's ranking.
    fn get_sonic_similar_tracks(
        &self,
        song_id: &str,
        count: usize,
    ) -> Result<Vec<SubsonicSong>, SourceError> {
        let data = self.call_within::<GetSonicSimilarTracksData>(
            "getSonicSimilarTracks.view",
            vec![
                ("id".to_string(), song_id.to_string()),
                ("count".to_string(), count.to_string()),
            ],
            SIMILAR_SONGS_TIMEOUT,
        )?;
        Ok(data.sonic_match.into_iter().map(|m| m.entry).collect())
    }

    /// Subsonic `getSimilarSongs` (API 1.11.0). This is the variant whose `id`
    /// accepts a song; `getSimilarSongs2` is specified as artist-seeded, and
    /// only happens to accept a song id on servers that alias the two.
    fn get_similar_songs_by_metadata(
        &self,
        song_id: &str,
        count: usize,
    ) -> Result<Vec<SubsonicSong>, SourceError> {
        let data = self.call_within::<GetSimilarSongsData>(
            "getSimilarSongs.view",
            vec![
                ("id".to_string(), song_id.to_string()),
                ("count".to_string(), count.to_string()),
            ],
            SIMILAR_SONGS_TIMEOUT,
        )?;
        Ok(data.similar_songs.map(|s| s.song).unwrap_or_default())
    }

    /// One song by id. Radio uses it to put the seed at the head of the queue:
    /// `getSimilarSongs2` never includes the seed itself.
    pub fn get_song(&self, song_id: &str) -> Result<Option<SubsonicSong>, SourceError> {
        let data = self.call::<GetSongData>(
            "getSong.view",
            vec![("id".to_string(), song_id.to_string())],
        )?;
        Ok(data.song)
    }

    pub fn get_playlists(&self) -> Result<Vec<SubsonicPlaylist>, SourceError> {
        let data = self.call::<GetPlaylistsData>("getPlaylists.view", Vec::new())?;
        Ok(data.playlists.playlist)
    }

    pub fn get_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<SubsonicSong>, SourceError> {
        let data = self.call::<GetPlaylistData>(
            "getPlaylist.view",
            vec![("id".to_string(), playlist_id.to_string())],
        )?;
        Ok(data.playlist.map(|p| p.entry).unwrap_or_default())
    }

    pub fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, SourceError> {
        let mut params = vec![("name".to_string(), name.to_string())];
        for item_id in item_ids {
            params.push(("songId".to_string(), item_id.clone()));
        }

        let data = self.call::<PlaylistCreationData>("createPlaylist.view", params)?;
        if let Some(playlist) = data.playlist {
            return Ok(playlist.id);
        }
        Err(SourceError::Backend(
            "Subsonic createPlaylist did not return a playlist id".to_string(),
        ))
    }

    pub fn add_to_playlist(&self, playlist_id: &str, item_id: &str) -> Result<(), SourceError> {
        self.call::<EmptyData>(
            "updatePlaylist.view",
            vec![
                ("playlistId".to_string(), playlist_id.to_string()),
                ("songIdToAdd".to_string(), item_id.to_string()),
            ],
        )
        .map(|_| ())
    }

    pub fn remove_from_playlist(
        &self,
        playlist_id: &str,
        song_index: u32,
    ) -> Result<(), SourceError> {
        self.call::<EmptyData>(
            "updatePlaylist.view",
            vec![
                ("playlistId".to_string(), playlist_id.to_string()),
                ("songIndexToRemove".to_string(), song_index.to_string()),
            ],
        )
        .map(|_| ())
    }

    pub fn reorder_playlist(
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
            .map(|_| ())
    }

    pub fn get_artists(&self) -> Result<Vec<SubsonicArtist>, SourceError> {
        let data = self.call::<GetArtistsData>("getArtists.view", Vec::new())?;
        Ok(data
            .artists
            .index
            .into_iter()
            .flat_map(|idx| idx.artist)
            .collect())
    }

    pub fn get_starred_song_ids(&self) -> Result<Vec<String>, SourceError> {
        let data = self.call::<GetStarred2Data>("getStarred2.view", Vec::new())?;
        Ok(data
            .starred2
            .map(|s| s.song.into_iter().map(|song| song.id).collect())
            .unwrap_or_default())
    }

    pub fn star(&self, item_id: &str) -> Result<(), SourceError> {
        self.call::<EmptyData>("star.view", vec![("id".to_string(), item_id.to_string())])
            .map(|_| ())
    }

    pub fn unstar(&self, item_id: &str) -> Result<(), SourceError> {
        self.call::<EmptyData>("unstar.view", vec![("id".to_string(), item_id.to_string())])
            .map(|_| ())
    }

    pub fn stream_url(&self, item_id: &str) -> Result<String, SourceError> {
        let mut params = self.auth_params();
        params.push(("id".to_string(), item_id.to_string()));
        let request = self
            .http
            .get(format!("{}/rest/stream.view", self.base_url))
            .query(&params);
        request.url().map(|url| url.to_string()).map_err(|_| {
            SourceError::InvalidInput(format!("invalid Subsonic base URL '{}'", self.base_url))
        })
    }

    pub fn scrobble(&self, item_id: &str, submission: bool) -> Result<(), SourceError> {
        self.call::<EmptyData>(
            "scrobble.view",
            vec![
                ("id".to_string(), item_id.to_string()),
                ("submission".to_string(), submission.to_string()),
            ],
        )
        .map(|_| ())
    }

    pub fn get_play_queue(&self) -> Result<Option<SubsonicPlayQueue>, SourceError> {
        let data = self.call::<GetPlayQueueData>("getPlayQueue.view", Vec::new())?;
        Ok(data.play_queue)
    }

    /// An empty `item_ids` with no `current` clears the saved queue.
    pub fn save_play_queue(
        &self,
        item_ids: &[String],
        current_id: Option<&str>,
        position_ms: Option<u64>,
    ) -> Result<(), SourceError> {
        let mut params: Vec<(String, String)> = item_ids
            .iter()
            .map(|id| ("id".to_string(), id.clone()))
            .collect();
        if let Some(current) = current_id {
            params.push(("current".to_string(), current.to_string()));
        }
        if let Some(position) = position_ms {
            params.push(("position".to_string(), position.to_string()));
        }
        self.call::<EmptyData>("savePlayQueue.view", params)
            .map(|_| ())
    }

    fn auth_params(&self) -> Vec<(String, String)> {
        let salt = random_salt();
        let token = format!("{:x}", md5::compute(format!("{}{}", self.password, salt)));
        vec![
            ("u".to_string(), self.username.clone()),
            ("t".to_string(), token),
            ("s".to_string(), salt),
            ("v".to_string(), SUBSONIC_API_VERSION.to_string()),
            ("c".to_string(), CLIENT_NAME.to_string()),
            ("f".to_string(), "json".to_string()),
        ]
    }

    fn call<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        extra_params: Vec<(String, String)>,
    ) -> Result<T, SourceError> {
        self.call_within(endpoint, extra_params, DEFAULT_TIMEOUT)
    }

    fn call_within<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        mut extra_params: Vec<(String, String)>,
        timeout: Duration,
    ) -> Result<T, SourceError> {
        let mut params = self.auth_params();
        params.append(&mut extra_params);

        let response = self
            .http
            .get(format!("{}/rest/{endpoint}", self.base_url))
            .query(&params)
            .timeout(timeout)
            .send()
            .map_err(redacted)?;
        if !response.is_success() {
            return Err(SourceError::from_status(
                response.status(),
                "Subsonic request",
            ));
        }
        let parsed: SubsonicEnvelope<T> = response.json().map_err(redacted)?;
        unwrap_envelope(parsed)
    }
}

fn unwrap_envelope<T>(parsed: SubsonicEnvelope<T>) -> Result<T, SourceError> {
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

/// 40 is wrong credentials, 44 an invalid API key (OpenSubsonic) and 50 an account not allowed to do it.
fn envelope_error(code: i32, message: &str) -> SourceError {
    match code {
        40 | 44 | 50 => SourceError::Auth,
        _ => SourceError::Backend(format!("Subsonic request failed ({code}): {message}")),
    }
}

/// A request error as a source error. The host's errors never carry the URL,
/// whose query holds the user name, token and salt, and nor do these.
fn redacted(error: http::Error) -> SourceError {
    match error {
        http::Error::Request(_) => SourceError::InvalidInput("invalid Subsonic request".into()),
        error => error.into(),
    }
}

fn random_salt() -> String {
    const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 16];
    // A salt only has to differ between requests; if the host's randomness is
    // unavailable a fixed one still authenticates.
    let _ = getrandom::fill(&mut bytes);
    bytes
        .iter()
        .map(|byte| ALPHANUMERIC[*byte as usize % ALPHANUMERIC.len()] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_failed_envelope_is_classified_by_its_code() {
        let parsed: SubsonicEnvelope<EmptyData> = serde_json::from_str(
            r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password"}}}"#,
        )
        .unwrap();
        assert_eq!(unwrap_envelope(parsed).err(), Some(SourceError::Auth));
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

        let gain = song.replay_gain();
        assert_eq!(gain.track_gain_db, Some(-7.5));
        assert_eq!(gain.album_gain_db, Some(-5.25));
        assert_eq!(gain.track_peak, Some(0.98));
        assert_eq!(gain.album_peak, None);
    }

    #[test]
    fn a_song_without_the_object_reports_nothing() {
        let song: SubsonicSong = serde_json::from_str(r#"{"id": "1", "title": "T"}"#).unwrap();
        assert_eq!(song.replay_gain(), ReplayGain::default());
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

    #[test]
    fn a_stream_url_is_signed_and_names_the_item() {
        let client = SubsonicClient::new("https://music.example.test/", "user", "password");
        let url = client.stream_url("song-1").expect("valid URL");
        assert!(url.starts_with("https://music.example.test/rest/stream.view?"));
        assert!(url.contains("id=song-1"));
        assert!(url.contains("u=user"));
        assert!(
            !url.contains("password"),
            "the password is hashed, never sent"
        );
    }
}
