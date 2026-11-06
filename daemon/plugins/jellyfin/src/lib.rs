//! Jellyfin, as a kopuz plugin.

mod client;

use client::{Item, JellyfinClient, NamedItem};
use kopuz_plugin_sdk::{
    Album, ArtistCredit, ArtistListing, AuthOutcome, AuthSession, Connection, LibrarySnapshot,
    PlaylistEntry, PlaylistMeta, Result, Source, SourceError, StreamInfo, StreamQuality, Track,
    cover,
};

const SERVICE: &str = "jellyfin";

/// Items per page while syncing.
const PAGE: usize = 500;

pub struct Jellyfin {
    client: JellyfinClient,
}

kopuz_plugin_sdk::export_source!(Jellyfin);

fn credits_of(items: Option<&[NamedItem]>) -> Vec<ArtistCredit> {
    items
        .unwrap_or_default()
        .iter()
        .filter(|item| !item.name.trim().is_empty())
        .map(|item| match item.id.is_empty() {
            true => ArtistCredit::unlinked(&item.name),
            false => ArtistCredit::linked(&item.name, &item.id),
        })
        .collect()
}

/// An authenticated image URL, for the artist photos the host stores as is.
fn item_image_url(
    server_url: &str,
    item_id: &str,
    image_tag: Option<&str>,
    access_token: Option<&str>,
    max_width: u32,
    quality: u32,
) -> String {
    let mut params = vec![
        format!("maxWidth={max_width}"),
        format!("quality={quality}"),
    ];
    if let Some(tag) = image_tag {
        params.push(format!("tag={tag}"));
    }
    if let Some(token) = access_token {
        params.push(format!("api_key={token}"));
    }
    format!(
        "{server_url}/Items/{item_id}/Images/Primary?{}",
        params.join("&")
    )
}

/// A track as a library listing reads it. The cover is the bare image tag,
/// which the host resolves against the item itself.
fn library_track(item: Item) -> Track {
    let artist = item
        .album_artist
        .clone()
        .or_else(|| item.artists.as_ref().map(|artists| artists.join(", ")))
        .unwrap_or_default();
    Track {
        cover: item.primary_image(),
        album_id: item
            .album_id
            .as_deref()
            .map(|id| format!("{SERVICE}:{id}"))
            .unwrap_or_default(),
        album: item.album.clone().unwrap_or_default(),
        duration_secs: item.run_time_ticks.unwrap_or(0) / 10_000_000,
        sample_rate: item.sample_rate.unwrap_or(0),
        bitrate_kbps: item.bitrate.unwrap_or(0) / 1000,
        track_number: item.index_number,
        disc_number: item.parent_index_number,
        playlist_item_id: item.playlist_item_id.clone(),
        credits: credits_of(item.artist_items.as_deref()),
        replay_gain: item.replay_gain(),
        artist,
        artists: item
            .artists
            .clone()
            .unwrap_or_else(|| item.album_artist.clone().into_iter().collect()),
        item_id: item.id,
        title: item.name,
        ..Track::default()
    }
}

/// A playlist entry keeps only the artists it lists itself.
fn playlist_track(item: Item) -> Track {
    let artists = item.artists.clone().unwrap_or_default();
    Track {
        artists,
        ..library_track(item)
    }
}

impl Source for Jellyfin {
    fn login(connection: Connection, username: &str, password: &str) -> Result<AuthSession> {
        let client = JellyfinClient::new(&connection.url, None, &connection.device_id, None);
        let (access_token, user_id) = client.login(username, password)?;
        Ok(AuthSession {
            access_token,
            user_id,
        })
    }

    fn open(connection: Connection) -> Self {
        Self {
            client: JellyfinClient::new(
                &connection.url,
                Some(&connection.token),
                &connection.device_id,
                Some(&connection.user_id),
            ),
        }
    }

    fn validate(&self) -> AuthOutcome {
        match self.client.ping() {
            Ok(()) => AuthOutcome::Valid,
            Err(SourceError::Auth) => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    fn keepalive(&self) -> Result<()> {
        self.client.ping()
    }

    fn fetch_library(&self) -> Result<LibrarySnapshot> {
        let mut albums = Vec::new();
        let mut tracks = Vec::new();

        for library in self.client.get_music_libraries()? {
            let mut start = 0;
            loop {
                let page = self.client.get_albums_paginated(&library.id, start, PAGE)?;
                if page.is_empty() {
                    break;
                }
                let count = page.len();
                for album in page {
                    let image_tag = album
                        .image_tags
                        .as_ref()
                        .and_then(|tags| tags.get("Primary").cloned());
                    let artist_id = album
                        .album_artists
                        .as_ref()
                        .and_then(|artists| artists.first())
                        .map(|artist| artist.id.clone());
                    albums.push(Album {
                        cover: Some(cover::item_ref(SERVICE, &album.id, image_tag.as_deref())),
                        artist_id,
                        id: format!("{SERVICE}:{}", album.id),
                        title: album.name,
                        artist: album
                            .album_artist
                            .or_else(|| album.artists.as_ref().map(|x| x.join(", ")))
                            .unwrap_or_default(),
                        genre: album
                            .genres
                            .as_ref()
                            .map(|g| g.join(", "))
                            .unwrap_or_default(),
                        year: album
                            .production_year
                            .map(|y| u16::try_from(y).unwrap_or(u16::MAX))
                            .unwrap_or(0),
                    });
                }
                start += count;
                if count < PAGE {
                    break;
                }
            }

            let mut start = 0;
            loop {
                let items =
                    self.client
                        .get_music_library_items_paginated(&library.id, start, PAGE)?;
                if items.is_empty() {
                    break;
                }
                let count = items.len();
                tracks.extend(items.into_iter().map(library_track));
                start += count;
                if count < PAGE {
                    break;
                }
            }
        }

        let artists = self.fetch_artists().unwrap_or_default();
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
                photo: artist.primary_image().map(|tag| {
                    item_image_url(
                        self.client.base_url(),
                        &artist.id,
                        Some(&tag),
                        self.client.token(),
                        512,
                        90,
                    )
                }),
                credit: ArtistCredit::linked(&artist.name, &artist.id),
            })
            .collect())
    }

    fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>> {
        Ok(self
            .client
            .get_playlists()?
            .into_iter()
            .map(|playlist| PlaylistMeta {
                image_tag: playlist.primary_image(),
                id: playlist.id,
                name: playlist.name,
            })
            .collect())
    }

    fn fetch_playlist_entries(&self, playlist_id: &str) -> Result<Vec<Track>> {
        Ok(self
            .client
            .get_playlist_items(playlist_id)?
            .into_iter()
            .map(playlist_track)
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

    fn remove_from_playlist(&self, playlist_id: &str, entry: &PlaylistEntry, _: u32) -> Result<()> {
        let entry_id = entry
            .playlist_item_id
            .as_deref()
            .ok_or_else(|| SourceError::InvalidInput("track has no playlist-entry id".into()))?;
        self.client.remove_from_playlist(playlist_id, entry_id)
    }

    fn reorder_playlist(
        &self,
        playlist_id: &str,
        _: &[PlaylistEntry],
        moved: &PlaylistEntry,
        new_index: u32,
    ) -> Result<()> {
        let entry_id = moved
            .playlist_item_id
            .as_deref()
            .ok_or_else(|| SourceError::InvalidInput("track has no playlist-entry id".into()))?;
        self.client
            .move_playlist_item(playlist_id, entry_id, new_index)
    }

    fn set_playlist_image(
        &self,
        playlist_id: &str,
        image: &[u8],
        content_type: &str,
    ) -> Result<()> {
        self.client
            .set_playlist_image(playlist_id, image, content_type)
    }

    fn fetch_favorites(&self) -> Result<Vec<String>> {
        Ok(self
            .client
            .get_favorite_items()?
            .into_iter()
            .map(|item| item.id)
            .collect())
    }

    fn push_favorite(&self, item_id: &str, on: bool) -> Result<()> {
        self.client.set_favorite(item_id, on)
    }

    fn resolve_stream(&self, item_id: &str, _: StreamQuality) -> Result<StreamInfo> {
        Ok(StreamInfo::url(self.client.stream_url(item_id)))
    }

    fn report_playback_start(&self, item_id: &str) -> Result<()> {
        self.client.report_playback_start(item_id)
    }

    fn report_playback_stopped(&self, item_id: &str, position_ticks: u64) -> Result<()> {
        self.client.report_playback_stopped(item_id, position_ticks)
    }

    fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: u64,
        is_paused: bool,
    ) -> Result<()> {
        self.client
            .report_playback_progress(item_id, position_ticks, is_paused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(json: serde_json::Value) -> Item {
        serde_json::from_value(json).expect("valid Jellyfin item")
    }

    #[test]
    fn a_library_track_takes_its_album_artist_and_filed_album() {
        let track = library_track(item(serde_json::json!({
            "Name": "One",
            "Id": "t1",
            "Type": "Audio",
            "AlbumId": "a1",
            "AlbumArtist": "Ada",
            "Artists": ["Ada", "Boris"],
            "ArtistItems": [{ "Name": "Ada", "Id": "ar1" }, { "Name": "", "Id": "ar2" }],
            "RunTimeTicks": 1_230_000_000u64,
            "Bitrate": 320_000,
            "ImageTags": { "Primary": "tag1" }
        })));
        assert_eq!(track.album_id, "jellyfin:a1");
        assert_eq!(track.artist, "Ada");
        assert_eq!(track.artists, ["Ada", "Boris"]);
        assert_eq!(track.duration_secs, 123);
        assert_eq!(track.bitrate_kbps, 320);
        assert_eq!(track.cover.as_deref(), Some("tag1"));
        assert_eq!(track.credits, [ArtistCredit::linked("Ada", "ar1")]);
    }

    #[test]
    fn a_track_without_listed_artists_falls_back_to_the_album_artist_only_in_the_library() {
        let json =
            serde_json::json!({ "Name": "One", "Id": "t1", "Type": "Audio", "AlbumArtist": "Ada" });
        assert_eq!(library_track(item(json.clone())).artists, ["Ada"]);
        assert!(playlist_track(item(json)).artists.is_empty());
    }

    #[test]
    fn an_artist_photo_url_is_signed_with_the_token() {
        assert_eq!(
            item_image_url("https://jf.test", "ar1", Some("t"), Some("tok"), 512, 90),
            "https://jf.test/Items/ar1/Images/Primary?maxWidth=512&quality=90&tag=t&api_key=tok"
        );
    }
}
