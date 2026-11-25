//! Subsonic, OpenSubsonic and anything speaking their API, as a kopuz plugin.
//!
//! One plugin serves two services: `subsonic`, and `custom` for a server the
//! person points at by hand. Both speak the same API; the service slug only
//! decides the prefix the host files their items under.

mod client;
mod stats;

use std::collections::HashSet;

use client::{SubsonicClient, SubsonicPlaylist, SubsonicSong};
use kopuz_plugin_sdk::{
    Album, ArtistCredit, ArtistInfo, ArtistListing, AuthOutcome, AuthSession, Connection,
    LibrarySnapshot, PlaylistEntry, PlaylistMeta, RadioStation, RemotePlayQueue, Result, Source,
    SourceError, StreamInfo, StreamQuality, Track, TrackPage, cover,
};

/// How many neighbours a radio start asks for.
const SIMILAR_SONGS_COUNT: usize = 50;

/// How many playlist songs seed a playlist radio. Each is one similarity
/// lookup, and the metadata fallback can take seconds per lookup.
const PLAYLIST_RADIO_SEEDS: usize = 3;

/// Albums per `getAlbumList2` page during a sync.
const ALBUM_PAGE: usize = 250;

pub struct Subsonic {
    client: SubsonicClient,
    /// `subsonic` or `custom`: the prefix this session's item refs carry, so
    /// a stored ref resolves against the server that issued it.
    service: String,
}

kopuz_plugin_sdk::export_source!(Subsonic);

/// The artists a song credits. An OpenSubsonic server lists them one by one, so
/// a collaboration files under each artist; a plain Subsonic server only names
/// the joined `billed` string, which stays one credit.
fn credits_of(song: &SubsonicSong, billed: &str) -> Vec<ArtistCredit> {
    let listed: Vec<ArtistCredit> = song
        .artists
        .iter()
        .filter(|artist| !artist.name.trim().is_empty())
        .map(|artist| match artist.id.is_empty() {
            true => ArtistCredit::unlinked(&artist.name),
            false => ArtistCredit::linked(&artist.name, &artist.id),
        })
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    match song.artist_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => vec![ArtistCredit::linked(billed, id)],
        None => vec![ArtistCredit::unlinked(billed)],
    }
}

/// The server's album id alone: a signed cover URL in it made every listing of the album mint a new one.
fn album_ref(service: &str, album_id: &str) -> String {
    cover::item_ref(service, album_id, None)
}

/// A cover-art id kept as an id, so the credentials that sign it are added only when it is fetched.
fn cover_ref(service: &str, cover_art: Option<&str>) -> String {
    match cover_art.filter(|id| !id.is_empty()) {
        Some(id) => cover::item_ref(service, id, None),
        None => cover::NO_COVER.to_string(),
    }
}

fn album_item_id(stored: &str) -> &str {
    ["subsonic:", "custom:"]
        .into_iter()
        .find_map(|prefix| stored.strip_prefix(prefix))
        .and_then(|rest| rest.split(':').next())
        .filter(|id| !id.is_empty())
        .unwrap_or(stored)
}

/// A song as a track, its album and artist falling back to the album it was
/// listed under when the song itself leaves them out.
fn song_to_track(service: &str, song: SubsonicSong, album: Option<(&str, &str, &str)>) -> Track {
    let (fallback_album_id, fallback_album, fallback_artist) = album.unwrap_or_default();
    let album_id = match song.album_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => album_ref(service, id),
        None => fallback_album_id.to_string(),
    };
    let artist = song
        .artist
        .clone()
        .unwrap_or_else(|| fallback_artist.to_string());
    let credits = credits_of(&song, &artist);
    let (play_count, last_played, rating) = (song.play_count, song.last_played(), song.rating());
    if let Some(bits) = song.bit_depth.and_then(|bits| u8::try_from(bits).ok()) {
        kopuz_plugin_sdk::report_bit_depth(&song.id, bits);
    }
    Track {
        cover: Some(cover_ref(service, song.cover_art.as_deref())),
        album_id,
        artist,
        album: song
            .album
            .clone()
            .unwrap_or_else(|| fallback_album.to_string()),
        duration_secs: song.duration.unwrap_or(0),
        sample_rate: song.sampling_rate.unwrap_or(0),
        bitrate_kbps: song.bit_rate.unwrap_or(0),
        track_number: song.track,
        disc_number: song.disc_number,
        artists: credits.iter().map(|credit| credit.name.clone()).collect(),
        credits,
        replay_gain: song.replay_gain(),
        item_id: song.id,
        title: song.title,
        play_count,
        last_played,
        rating,
        ..Track::default()
    }
}

/// The playlist's cover kept as its id, like an album's, so no signed URL is stored.
fn playlist_meta(service: &str, playlist: SubsonicPlaylist) -> PlaylistMeta {
    let image_tag = playlist
        .cover_art
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(|id| cover::item_ref(service, id, None));
    PlaylistMeta {
        id: playlist.id,
        name: playlist.name,
        image_tag,
    }
}

/// `count` songs spread over the whole list, so a long playlist is not seeded by its opening alone.
/// E.g. 10 songs, 3 seeds: indices 0, 3, 6; 4 songs, 3 seeds: 0, 1, 2.
fn spread<T>(items: &[T], count: usize) -> Vec<&T> {
    let seeds = count.min(items.len());
    (0..seeds)
        .map(|seed| &items[seed * items.len() / seeds])
        .collect()
}

/// Each seed's neighbours interleaved by rank, without the playlist's own songs and without repeats.
fn blend(
    playlist: &HashSet<String>,
    lists: Vec<Vec<SubsonicSong>>,
    limit: usize,
) -> Vec<SubsonicSong> {
    let mut seen = playlist.clone();
    let mut lists: Vec<_> = lists.into_iter().map(Vec::into_iter).collect();
    let mut blended = Vec::new();
    while blended.len() < limit {
        let round: Vec<SubsonicSong> = lists.iter_mut().filter_map(Iterator::next).collect();
        if round.is_empty() {
            break;
        }
        for song in round {
            if seen.insert(song.id.clone()) {
                blended.push(song);
            }
        }
    }
    blended.truncate(limit);
    blended
}

/// Plain text of a Last.fm biography: its "Read more" anchor goes with its label, other tags only lose their markup.
fn plain_biography(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            rest = "";
            break;
        };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        if !tag.starts_with(['a', 'A'])
            || tag.len() > 1 && !tag[1..].starts_with(char::is_whitespace)
        {
            continue;
        }
        // Skip the anchor's label up to its closing tag.
        let lower = rest.to_ascii_lowercase();
        rest = lower
            .find("</a>")
            .map_or("", |end| &rest[end + "</a>".len()..]);
    }
    text.push_str(rest);

    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

impl Subsonic {
    fn track(&self, song: SubsonicSong) -> Track {
        song_to_track(&self.service, song, None)
    }
}

impl Source for Subsonic {
    fn login(connection: Connection, username: &str, password: &str) -> Result<AuthSession> {
        SubsonicClient::new(&connection.url, username, password).ping()?;
        Ok(AuthSession {
            access_token: password.to_string(),
            user_id: username.to_string(),
        })
    }

    fn open(connection: Connection) -> Self {
        Self {
            client: SubsonicClient::new(&connection.url, &connection.user_id, &connection.token),
            service: connection.service,
        }
    }

    fn validate(&self) -> AuthOutcome {
        match self.client.ping() {
            Ok(()) => AuthOutcome::Valid,
            Err(SourceError::Auth) => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    fn fetch_library(&self) -> Result<LibrarySnapshot> {
        let mut albums = Vec::new();
        let mut tracks = Vec::new();
        let mut seen = HashSet::new();
        let artists = self.fetch_artists().unwrap_or_default();

        let mut offset = 0;
        loop {
            let page = self.client.get_album_list(offset, ALBUM_PAGE)?;
            if page.is_empty() {
                break;
            }
            let count = page.len();
            for album in page {
                let album_id = album_ref(&self.service, &album.id);
                let album_artist = album.artist.clone().unwrap_or_default();
                albums.push(Album {
                    id: album_id.clone(),
                    title: album.name.clone(),
                    artist: album_artist.clone(),
                    genre: album.genre.clone().unwrap_or_default(),
                    year: album.year.unwrap_or(0),
                    cover: Some(cover_ref(&self.service, album.cover_art.as_deref())),
                    artist_id: album.artist_id.clone(),
                });

                let songs = self
                    .client
                    .get_album_songs(&album.id)
                    .map_err(|e| match e {
                        SourceError::Backend(message) => SourceError::Backend(format!(
                            "failed to fetch songs for album {}: {message}",
                            album.id
                        )),
                        kind => kind,
                    })?;
                for song in songs {
                    if !seen.insert(song.id.clone()) {
                        continue;
                    }
                    // A song in an album listing belongs to that album, so the
                    // listing's id wins over whatever the song names.
                    let mut track = song_to_track(
                        &self.service,
                        song,
                        Some((&album_id, &album.name, &album_artist)),
                    );
                    track.album_id = album_id.clone();
                    tracks.push(track);
                }
            }
            offset += count;
            if count < ALBUM_PAGE {
                break;
            }
        }

        Ok(LibrarySnapshot {
            albums,
            tracks,
            artists,
        })
    }

    fn fetch_artists(&self) -> Result<Vec<ArtistListing>> {
        Ok(self
            .client
            .get_artists()?
            .into_iter()
            .map(|artist| ArtistListing {
                photo: artist
                    .cover_art
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .map(|id| cover_ref(&self.service, Some(id))),
                credit: ArtistCredit::linked(&artist.name, &artist.id),
            })
            .collect())
    }

    fn fetch_album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self
            .client
            .get_album_songs(album_item_id(album_id))?
            .into_iter()
            .map(|song| self.track(song))
            .collect())
    }

    fn fetch_track(&self, item_id: &str) -> Result<Option<Track>> {
        Ok(self.client.get_song(item_id)?.map(|song| self.track(song)))
    }

    fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>> {
        Ok(self
            .client
            .get_playlists()?
            .into_iter()
            .map(|playlist| playlist_meta(&self.service, playlist))
            .collect())
    }

    fn fetch_playlist_entries(&self, playlist_id: &str) -> Result<Vec<Track>> {
        Ok(self
            .client
            .get_playlist_entries(playlist_id)?
            .into_iter()
            .map(|song| self.track(song))
            .collect())
    }

    fn add_to_playlist(&self, playlist_id: &str, item_refs: &[String]) -> Result<Vec<String>> {
        Ok(item_refs
            .iter()
            .filter(|id| self.client.add_to_playlist(playlist_id, id).is_ok())
            .cloned()
            .collect())
    }

    fn create_playlist(&self, name: &str, item_refs: &[String]) -> Result<String> {
        self.client.create_playlist(name, item_refs)
    }

    fn remove_from_playlist(
        &self,
        playlist_id: &str,
        _: &PlaylistEntry,
        position: u32,
    ) -> Result<()> {
        self.client.remove_from_playlist(playlist_id, position)
    }

    fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered: &[PlaylistEntry],
        _: &PlaylistEntry,
        _: u32,
    ) -> Result<()> {
        let ids: Vec<&str> = ordered.iter().map(|entry| entry.key.as_str()).collect();
        self.client.reorder_playlist(playlist_id, &ids, ids.len())
    }

    /// Without it the SDK default reports success, and the next sync brings the playlist back.
    fn delete_remote_playlist(&self, playlist_id: &str) -> Result<()> {
        self.client.delete_playlist(playlist_id)
    }

    fn fetch_favorites(&self) -> Result<Vec<String>> {
        self.client.get_starred_song_ids()
    }

    fn push_favorite(&self, item_id: &str, on: bool) -> Result<()> {
        if on {
            self.client.star(item_id)
        } else {
            self.client.unstar(item_id)
        }
    }

    fn set_rating(&self, item_id: &str, stars: u8) -> Result<()> {
        self.client.set_rating(item_id, stars)
    }

    fn fetch_artist_info(&self, artist_id: &str) -> Result<Option<ArtistInfo>> {
        let Some(info) = self.client.get_artist_info(artist_id)? else {
            return Ok(None);
        };
        let biography = info
            .biography
            .as_deref()
            .map(plain_biography)
            .filter(|text| !text.is_empty());
        if biography.is_none() {
            return Ok(None);
        }

        let image = [info.large_image_url, info.medium_image_url]
            .into_iter()
            .flatten()
            .find(|url| !url.is_empty());
        Ok(Some(ArtistInfo {
            biography,
            image: image.as_deref().map(cover::url),
            similar: info
                .similar_artist
                .into_iter()
                .map(|artist| ArtistCredit::linked(artist.name, artist.id))
                .collect(),
            link: info.last_fm_url.filter(|url| !url.is_empty()),
        }))
    }

    fn fetch_radio_stations(&self) -> Result<Vec<RadioStation>> {
        Ok(self
            .client
            .get_radio_stations()?
            .into_iter()
            .map(|station| RadioStation {
                id: station.id,
                name: station.name,
                stream_url: station.stream_url,
                homepage: station.home_page_url.filter(|url| !url.is_empty()),
            })
            .collect())
    }

    fn resolve_stream(&self, item_id: &str, _: StreamQuality) -> Result<StreamInfo> {
        Ok(StreamInfo::url(self.client.stream_url(item_id)?))
    }

    /// Song-seeded radio. Which endpoint answers is the client's call (see
    /// [`SubsonicClient::get_similar_songs`]); either way it returns neighbours
    /// only, so the seed goes at the head of the queue rather than jumping the
    /// user off the track they started from. A seed the server cannot look up
    /// is not fatal: the neighbours still make a queue.
    fn start_radio(&self, seed_ref: &str) -> Result<TrackPage> {
        let similar: Vec<_> = self
            .client
            .get_similar_songs(seed_ref, SIMILAR_SONGS_COUNT)?
            .into_iter()
            .filter(|song| song.id != seed_ref)
            .collect();
        // Nothing similar means no radio. Returning the seed on its own would
        // read as success and replace the queue with the one song already
        // playing, so hand back empty and let the caller say so.
        if similar.is_empty() {
            return Ok(TrackPage::last(Vec::new()));
        }

        let seed = self.client.get_song(seed_ref).unwrap_or_else(|e| {
            kopuz_plugin_sdk::debug!("radio seed {seed_ref} lookup failed: {e}");
            None
        });
        Ok(TrackPage::last(
            seed.into_iter()
                .chain(similar)
                .map(|song| self.track(song))
                .collect(),
        ))
    }

    /// Songs like the playlist's, by the same similarity track radio uses
    /// (AudioMuse-AI's analysis where the server has it). A seed whose lookup
    /// fails is skipped; the others still make a mix.
    fn start_playlist_radio(&self, playlist_ref: &str) -> Result<TrackPage> {
        let entries = self.client.get_playlist_entries(playlist_ref)?;
        let playlist: HashSet<String> = entries.iter().map(|song| song.id.clone()).collect();

        let per_seed = SIMILAR_SONGS_COUNT.div_ceil(PLAYLIST_RADIO_SEEDS);
        let lists = spread(&entries, PLAYLIST_RADIO_SEEDS)
            .into_iter()
            .filter_map(
                |seed| match self.client.get_similar_songs(&seed.id, per_seed) {
                    Ok(songs) => Some(songs),
                    Err(e) => {
                        kopuz_plugin_sdk::debug!("playlist radio seed {} failed: {e}", seed.id);
                        None
                    }
                },
            )
            .collect();

        Ok(TrackPage::last(
            blend(&playlist, lists, SIMILAR_SONGS_COUNT)
                .into_iter()
                .map(|song| self.track(song))
                .collect(),
        ))
    }

    fn save_play_queue(
        &self,
        item_ids: &[String],
        current_index: u32,
        position_ms: u64,
    ) -> Result<bool> {
        let current = item_ids.get(current_index as usize).map(String::as_str);
        let position = (!item_ids.is_empty()).then_some(position_ms);
        self.client.save_play_queue(item_ids, current, position)?;
        Ok(true)
    }

    fn get_play_queue(&self) -> Result<Option<RemotePlayQueue>> {
        let Some(queue) = self.client.get_play_queue()? else {
            return Ok(None);
        };
        Ok(Some(RemotePlayQueue {
            tracks: queue
                .entry
                .into_iter()
                .map(|song| self.track(song))
                .collect(),
            current_id: queue.current,
            position_ms: queue.position.unwrap_or(0),
            changed_by: queue.changed_by,
        }))
    }

    fn scrobble_now_playing(&self, item_id: &str) -> Result<()> {
        self.client.scrobble(item_id, false)
    }

    fn scrobble(&self, item_id: &str) -> Result<()> {
        self.client.scrobble(item_id, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spread_takes_as_many_seeds_as_it_can() {
        let four = [0, 1, 2, 3];
        let ten: Vec<usize> = (0..10).collect();

        assert_eq!(spread(&four, 3), [&0, &1, &2]);
        assert_eq!(spread(&ten, 3), [&0, &3, &6]);
        assert_eq!(spread(&four[..2], 3), [&0, &1]);
        assert!(spread(&four, 0).is_empty());
        assert!(spread::<usize>(&[], 3).is_empty());
    }

    fn song(id: &str, album_id: Option<&str>) -> SubsonicSong {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": "Song",
            "album": "Album",
            "albumId": album_id,
            "artist": "Ada",
            "coverArt": "al-1_5f3a",
        }))
        .expect("valid Subsonic song")
    }

    #[test]
    fn a_biography_loses_its_markup_and_read_more_anchor() {
        let html = "Ada &amp; Co play <b>jazz</b>. <a target=\"_blank\" href=\"https://www.last.fm/music/Ada\" rel=\"nofollow\">Read more on Last.fm</a>";
        assert_eq!(plain_biography(html), "Ada & Co play jazz.");
        assert_eq!(plain_biography("<abbr>x</abbr> 1 < 2"), "x 1");
    }

    #[test]
    fn an_album_keeps_one_id_and_its_cover_stays_an_unsigned_ref() {
        let first = song_to_track("custom", song("s1", Some("al-1")), None);
        let again = song_to_track("custom", song("s1", Some("al-1")), None);
        let sibling = song_to_track("custom", song("s2", Some("al-1")), None);
        assert_eq!(first.album_id, "custom:al-1");
        assert_eq!(first.album_id, again.album_id);
        assert_eq!(first.album_id, sibling.album_id);
        assert_eq!(first.cover, again.cover);
        assert_eq!(first.cover.as_deref(), Some("custom:al-1_5f3a"));
        assert_eq!(album_item_id(&first.album_id), "al-1");
    }

    #[test]
    fn a_song_without_an_album_is_filed_under_no_album() {
        let track = song_to_track("subsonic", song("s1", None), None);
        assert_eq!(track.album_id, "");
    }

    #[test]
    fn stored_album_refs_are_reduced_to_provider_ids() {
        assert_eq!(album_item_id("subsonic:album-42:urlhex_00"), "album-42");
        assert_eq!(album_item_id("custom:album-7"), "album-7");
        assert_eq!(album_item_id("already-raw"), "already-raw");
    }

    #[test]
    fn playlist_cover_art_is_kept_as_an_id() {
        let playlist: SubsonicPlaylist = serde_json::from_value(serde_json::json!({
            "id": "playlist-1",
            "name": "Mix",
            "songCount": 3,
            "coverArt": "playlist-cover-1"
        }))
        .expect("valid Subsonic playlist");
        let meta = playlist_meta("subsonic", playlist);
        assert_eq!(meta.image_tag.as_deref(), Some("subsonic:playlist-cover-1"));

        let bare = playlist_meta(
            "subsonic",
            SubsonicPlaylist {
                id: "playlist-2".into(),
                name: "Mix".into(),
                cover_art: None,
            },
        );
        assert!(bare.image_tag.is_none());
    }

    #[test]
    fn open_subsonic_artists_credit_each_collaborator() {
        let song: SubsonicSong = serde_json::from_value(serde_json::json!({
            "id": "song-1",
            "title": "Collab",
            "artist": "Ada feat. Boris",
            "artistId": "joined",
            "artists": [{ "id": "ar-1", "name": "Ada" }, { "id": "ar-2", "name": "Boris" }]
        }))
        .expect("valid OpenSubsonic song");

        let credits = credits_of(&song, "Ada feat. Boris");
        let named: Vec<_> = credits
            .iter()
            .map(|credit| (credit.name.as_str(), credit.id.as_deref()))
            .collect();
        assert_eq!(named, [("Ada", Some("ar-1")), ("Boris", Some("ar-2"))]);
    }

    #[test]
    fn plain_subsonic_song_keeps_its_one_credit() {
        let song: SubsonicSong = serde_json::from_value(serde_json::json!({
            "id": "song-1",
            "title": "Collab",
            "artist": "Ada feat. Boris",
            "artistId": "ar-9"
        }))
        .expect("valid Subsonic song");

        let credits = credits_of(&song, "Ada feat. Boris");
        assert_eq!(credits.len(), 1);
        assert_eq!(credits[0].name, "Ada feat. Boris");
        assert_eq!(credits[0].id.as_deref(), Some("ar-9"));
    }

    #[test]
    fn an_open_subsonic_song_carries_its_sample_rate_and_bitrate() {
        let song: SubsonicSong = serde_json::from_value(serde_json::json!({
            "id": "song-1",
            "title": "Hi-res",
            "bitRate": 2_304,
            "samplingRate": 96_000,
            "bitDepth": 24
        }))
        .expect("valid OpenSubsonic song");
        assert_eq!(song.bit_depth, Some(24));

        let track = song_to_track("subsonic", song, None);
        assert_eq!(track.sample_rate, 96_000);
        assert_eq!(track.bitrate_kbps, 2_304);
    }
}
