use async_trait::async_trait;
use config::Source;
use db::Db;

use crate::{server_ops::ServerConn, ytmusic::YouTubeMusicClient};

use super::{
    AlbumType, ArtistLookup, ArtistView, AuthOutcome, Capabilities, CatalogPageEntry,
    FavoritesPage, FavoritesSync, LibraryActions, MediaSource, PlaylistDetails, PlaylistMeta,
    PlaylistOps, PlaylistPage, RadioSeeds, RemoteAlbum, SearchFilterEntry, SourceError, StreamInfo,
    mirror_added, mirror_created,
};
use crate::ytmusic::browse::{
    self,
    search::{self, SearchPage, Suggestion},
};
use crate::ytmusic::discover::{self, BrowsePage};

/// YT Music's "Liked Music" auto-playlist. It is not browsed like the user's
/// other playlists: its contents are the liked songs, which kopuz already keeps
/// as this source's favorites, so it is served straight out of the favorites
/// table. That keeps the tile and the heart icons from ever disagreeing, and
/// costs no extra round-trip. Confined to this file on purpose — no other
/// source has such a playlist and the UI stays unaware of it.
const LIKED_MUSIC_ID: &str = "LM";

/// The entry at `new_index` and the one after it, by their set ids: the
/// move YouTube takes is "put this before that", or to the end.
fn move_plan(ordered: &[reader::PlaylistEntry], new_index: usize) -> Option<(&str, Option<&str>)> {
    let moved = ordered.get(new_index)?.item_id.as_deref()?;
    let successor = match ordered.get(new_index + 1) {
        Some(next) => Some(next.item_id.as_deref()?),
        None => None,
    };
    Some((moved, successor))
}

/// What a playlist allows, by its header: only one the account owns has the
/// editable header, which is where a privacy is read from.
fn playlist_ops(header: &BrowsePage) -> PlaylistOps {
    if header.privacy.is_some() {
        PlaylistOps::Reorder
    } else {
        PlaylistOps::None
    }
}

/// Where the entry now at `new_index` sat in `before`, given that `after` is
/// `before` with that one entry moved. `None` when nothing moved.
fn moved_from(
    before: &[reader::PlaylistEntry],
    after: &[reader::PlaylistEntry],
    new_index: usize,
) -> Option<usize> {
    let differs = |i: &usize| before[*i].key != after[*i].key;
    let first = (0..before.len().min(after.len())).find(differs)?;
    let last = (0..before.len().min(after.len())).rev().find(differs)?;
    Some(if first == new_index { last } else { first })
}

/// The same move made on the playlist as YouTube lists it, for when the
/// stored entries lack set ids. YouTube can hold a different video than
/// the one added (a song for its music video), so the listing is matched
/// by position, and only while it is as long as what is stored.
fn moved_on_listing(
    stored: &[reader::PlaylistEntry],
    ordered: &[reader::PlaylistEntry],
    new_index: usize,
    mut listing: Vec<reader::PlaylistEntry>,
) -> Option<Vec<reader::PlaylistEntry>> {
    if listing.len() != stored.len() || listing.iter().any(|entry| entry.item_id.is_none()) {
        return None;
    }
    let from = moved_from(stored, ordered, new_index)?;
    let moved = listing.remove(from);
    listing.insert(new_index, moved);
    Some(listing)
}

pub(super) struct YtSource {
    db: Db,
    source: Source,
    client: YouTubeMusicClient,
}

impl YtSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            client: YouTubeMusicClient::with_cookies(conn.token.clone()),
        }
    }

    /// The favorites, as playlist entries, in favorite order — `tracks_by_keys`
    /// answers in table order, which would shuffle the tile on every sync.
    async fn liked_music_entries(&self) -> Result<Vec<reader::Track>, SourceError> {
        let keys = self.db.favorites(self.source.as_str()).await?;
        let mut tracks = self.db.tracks_by_keys(&self.source, &keys).await?;
        let position = |t: &reader::Track| keys.iter().position(|k| k == t.id.key().as_ref());
        tracks.sort_by_key(|t| position(t).unwrap_or(usize::MAX));
        Ok(tracks)
    }
}

#[async_trait]
impl MediaSource for YtSource {
    fn source(&self) -> &Source {
        &self.source
    }
    fn db(&self) -> &Db {
        &self.db
    }

    fn capabilities(&self) -> Capabilities {
        // What one playlist allows narrows this: see `playlist_ops`.
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            browse_folders: false,
            external_devices: false,
            browser_playback: false,
            sync: true,
            downloads: true,
            discover: true,
            dont_recommend: true,
            radio: RadioSeeds {
                search: true,
                ..RadioSeeds::ALL
            },
            playlists: if self.client.is_authenticated() {
                PlaylistOps::Reorder
            } else {
                PlaylistOps::AddRemove
            },
            artist_view: ArtistView::Remote,
            albums: AlbumType::YtMusic,
            favorites_sync: FavoritesSync::Paginated,
            // Every one of these is the account's, so none works signed out.
            library_actions: if self.client.is_authenticated() {
                LibraryActions::ALL
            } else {
                LibraryActions::NONE
            },
        }
    }

    async fn rate(&self, item_ref: &str, rating: discover::Rating) -> Result<(), SourceError> {
        if item_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("nothing to rate".into()));
        }
        Ok(self.client.rate(item_ref, rating).await?)
    }

    async fn follow(&self, artist_ref: &str, follow: bool) -> Result<(), SourceError> {
        if !artist_ref.starts_with("UC") {
            return Err(SourceError::InvalidInput("not a channel to follow".into()));
        }
        Ok(self.client.subscribe(artist_ref, follow).await?)
    }

    async fn save(&self, item_ref: &str, saved: bool) -> Result<(), SourceError> {
        if item_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("nothing to save".into()));
        }
        Ok(self.client.save(item_ref, saved).await?)
    }

    async fn remove_from_history(&self, token: &str) -> Result<(), SourceError> {
        if token.trim().is_empty() {
            return Err(SourceError::InvalidInput("no history token".into()));
        }
        Ok(self.client.remove_from_history(token).await?)
    }

    async fn edit_playlist(
        &self,
        playlist_id: &str,
        details: &PlaylistDetails,
    ) -> Result<(), SourceError> {
        if playlist_id == LIKED_MUSIC_ID {
            return Err(SourceError::InvalidInput(
                "Liked Music cannot be edited".into(),
            ));
        }
        Ok(self
            .client
            .edit_playlist(
                playlist_id,
                details.name.as_deref(),
                details.description.as_deref(),
                details.privacy,
            )
            .await?)
    }

    /// A playlist the account owns is deleted; one it saved is taken out of
    /// the library, which is all a delete can mean for someone else's.
    async fn delete_remote_playlist(&self, playlist_id: &str) -> Result<(), SourceError> {
        if playlist_id == LIKED_MUSIC_ID || !self.client.is_authenticated() {
            return Ok(());
        }
        let (_, _, header) = self.client.playlist_page(playlist_id, None).await?;
        // Only an owned playlist's header carries a privacy. Without a header
        // there is no telling, and guessing wrong leaves it on YouTube.
        let header = header.ok_or_else(|| {
            SourceError::Backend("YouTube Music sent no header for the playlist".into())
        })?;
        if header.privacy.is_some() {
            self.client.delete_playlist(playlist_id).await?;
        } else {
            self.client
                .save(&crate::ytmusic::actions::playlist_ref(playlist_id), false)
                .await?;
        }
        Ok(())
    }

    async fn dont_recommend(&self, item_id: &str) -> Result<(), SourceError> {
        if item_id.trim().is_empty() {
            return Err(SourceError::InvalidInput("track has no video id".into()));
        }
        self.client
            .dislike_video(item_id)
            .await
            .map_err(SourceError::from)
    }

    async fn start_radio(&self, seed_ref: &str) -> Result<Vec<reader::Track>, SourceError> {
        if seed_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("track has no video id".into()));
        }
        // /next works anonymously (empty cookies), so no auth gate here.
        self.client
            .start_mix(seed_ref)
            .await
            .map_err(SourceError::from)
    }

    async fn start_playlist_radio(
        &self,
        playlist_ref: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        if playlist_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("playlist has no id".into()));
        }
        // Liked Music needs no special case here: YT builds `RDAMPLLM` like any
        // other playlist mix — that is exactly what its own web client asks for.
        self.client
            .start_playlist_mix(playlist_ref)
            .await
            .map_err(SourceError::from)
    }

    fn web_url(&self, track: &reader::Track) -> Option<String> {
        let vid = track.id.key();
        (!vid.trim().is_empty()).then(|| format!("https://music.youtube.com/watch?v={vid}"))
    }

    fn album_web_url(&self, browse_id: &str) -> Option<String> {
        (!browse_id.trim().is_empty())
            .then(|| format!("https://music.youtube.com/browse/{browse_id}"))
    }

    async fn search(
        &self,
        query: &str,
    ) -> Result<(Vec<reader::Track>, Vec<reader::Album>), SourceError> {
        if query.trim().is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let tracks = self.client.search_tracks(query).await?;
        Ok((tracks, Vec::new()))
    }

    async fn discover_home(&self) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        self.client.discover_home().await.map_err(SourceError::from)
    }

    async fn discover_continuation(
        &self,
        token: &str,
    ) -> Result<crate::ytmusic::discover::DiscoverHome, SourceError> {
        self.client
            .discover_continuation(token)
            .await
            .map_err(SourceError::from)
    }

    fn catalog_pages(&self) -> Vec<CatalogPageEntry> {
        let page = |id: &str, label, icon| CatalogPageEntry {
            id: id.to_string(),
            label,
            icon,
        };
        let mut pages = vec![
            page(discover::HOME, "home", "fa-solid fa-house"),
            page(
                browse::EXPLORE,
                "catalog_page_explore",
                "fa-solid fa-compass",
            ),
            page(
                browse::NEW_RELEASES,
                "new_releases",
                "fa-solid fa-compact-disc",
            ),
            page(
                browse::CHARTS,
                "catalog_page_charts",
                "fa-solid fa-chart-simple",
            ),
            page(
                browse::MOODS,
                "catalog_page_moods",
                "fa-solid fa-masks-theater",
            ),
            page(
                browse::PODCASTS,
                "catalog_page_podcasts",
                "fa-solid fa-podcast",
            ),
        ];
        // The library and history are the account's; anonymously they are empty.
        if self.client.is_authenticated() {
            pages.extend([
                page(
                    browse::LIBRARY_SONGS,
                    "catalog_page_library_songs",
                    "fa-solid fa-music",
                ),
                page(
                    browse::LIBRARY_ALBUMS,
                    "catalog_page_library_albums",
                    "fa-solid fa-record-vinyl",
                ),
                page(
                    browse::LIBRARY_ARTISTS,
                    "catalog_page_library_artists",
                    "fa-solid fa-microphone",
                ),
                page(
                    browse::LIBRARY_SUBSCRIPTIONS,
                    "catalog_page_subscriptions",
                    "fa-solid fa-user-check",
                ),
                page(
                    browse::LIBRARY_PODCASTS,
                    "catalog_page_library_podcasts",
                    "fa-solid fa-podcast",
                ),
                page(
                    browse::LIBRARY_UPLOADS,
                    "catalog_page_uploads",
                    "fa-solid fa-upload",
                ),
                page(
                    browse::HISTORY,
                    "catalog_page_history",
                    "fa-solid fa-clock-rotate-left",
                ),
            ]);
        }
        pages
    }

    async fn browse_page(
        &self,
        id: &str,
        continuation: Option<&str>,
    ) -> Result<BrowsePage, SourceError> {
        if id.trim().is_empty() {
            return Err(SourceError::InvalidInput(
                "a page is opened by its id".into(),
            ));
        }
        Ok(match continuation {
            Some(token) => self.client.browse_continuation(token).await?,
            None => self.client.browse_page(id).await?,
        })
    }

    fn search_filters(&self) -> Vec<SearchFilterEntry> {
        let all = SearchFilterEntry {
            id: search::ALL,
            label: search::ALL_LABEL,
        };
        let library = self.client.is_authenticated().then_some(&search::LIBRARY);
        std::iter::once(all)
            .chain(
                search::FILTERS
                    .iter()
                    .chain(library)
                    .map(|filter| SearchFilterEntry {
                        id: filter.id,
                        label: filter.label,
                    }),
            )
            .collect()
    }

    async fn search_shelves(
        &self,
        query: &str,
        filter: &str,
        continuation: Option<&str>,
    ) -> Result<SearchPage, SourceError> {
        if let Some(token) = continuation {
            return Ok(search::continued(
                self.client.browse_continuation(token).await?,
            ));
        }
        if query.trim().is_empty() {
            return Ok(SearchPage::default());
        }
        let filter = match filter {
            search::ALL => None,
            id => Some(
                search::filter(id)
                    .ok_or_else(|| SourceError::InvalidInput(format!("no such filter: {id}")))?,
            ),
        };
        Ok(self.client.search_page(query, filter).await?)
    }

    async fn search_suggestions(&self, query: &str) -> Result<Vec<Suggestion>, SourceError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.client.search_suggestions(query).await?)
    }

    async fn related(&self, item_id: &str) -> Result<BrowsePage, SourceError> {
        if item_id.trim().is_empty() {
            return Err(SourceError::InvalidInput("track has no video id".into()));
        }
        Ok(self.client.related(item_id).await?)
    }

    async fn fetch_album_tracks(&self, browse_id: &str) -> Result<Vec<reader::Track>, SourceError> {
        self.client
            .fetch_album_tracks(browse_id)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_album(&self, browse_id: &str) -> Result<RemoteAlbum, SourceError> {
        self.client
            .fetch_album(browse_id)
            .await
            .map(RemoteAlbum::from)
            .map_err(SourceError::from)
    }

    async fn fetch_album_by_ref(&self, id: &str) -> Result<Option<RemoteAlbum>, SourceError> {
        // Resolve the id (raw browse id, `ytmusic:album:MPRE…`, or a synthesized
        // `ytmusic:album:<hash>`) to a real browse id before fetching.
        let browse_id = if let Some(bid) = crate::ytmusic::search::album_browse_id(id) {
            Some(bid)
        } else if let Some((album, artist)) = crate::ytmusic::search::synth_album_parts(id) {
            self.resolve_album_browse_id(&album, &artist).await?
        } else {
            None
        };
        let Some(browse_id) = browse_id else {
            return Ok(None);
        };
        Ok(self
            .fetch_album(&browse_id)
            .await
            .ok()
            .filter(|a| !a.tracks.is_empty()))
    }

    async fn fetch_album_by_meta(
        &self,
        title: &str,
        artist: &str,
    ) -> Result<Option<RemoteAlbum>, SourceError> {
        let Some(browse_id) = self.resolve_album_browse_id(title, artist).await? else {
            return Ok(None);
        };
        Ok(self
            .fetch_album(&browse_id)
            .await
            .ok()
            .filter(|a| !a.tracks.is_empty()))
    }

    async fn fetch_playlist_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<(Vec<reader::Track>, Option<String>), SourceError> {
        let (tracks, next, _) = self
            .client
            .playlist_page(playlist_id, cursor.as_deref())
            .await?;
        Ok((tracks, next))
    }

    async fn resolve_album_browse_id(
        &self,
        album: &str,
        artist: &str,
    ) -> Result<Option<String>, SourceError> {
        self.client
            .resolve_album_browse_id(album, artist)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_artist(
        &self,
        channel_id: &str,
    ) -> Result<crate::ytmusic::discover::YtArtist, SourceError> {
        self.client
            .fetch_artist(channel_id)
            .await
            .map_err(SourceError::from)
    }

    async fn fetch_artist_image(
        &self,
        artist: &reader::ArtistCredit,
    ) -> Result<ArtistLookup, SourceError> {
        if let Some(channel) = artist.id.as_deref() {
            let header = self
                .client
                .artist_header(channel)
                .await
                .map_err(SourceError::from)?;
            return Ok(ArtistLookup {
                image: header.avatar,
                name: header.name,
            });
        }
        let name = artist.name.as_str();
        if let Some(url) = self
            .client
            .resolve_artist_image(name)
            .await
            .map_err(SourceError::from)?
        {
            return Ok(ArtistLookup {
                image: Some(url),
                name: None,
            });
        }
        // No artists-search entry (user channels for uploaded content) —
        // reconcile the channel from a library song and use its avatar.
        let Some(key) = artist.key.as_deref() else {
            return Ok(ArtistLookup::default());
        };
        let tracks = self
            .db
            .artist_tracks(&self.source, key, Some(3))
            .await
            .unwrap_or_default();
        for track in tracks.iter() {
            if let Ok(Some(cid)) = self
                .client
                .artist_channel_for_video(&track.id.key(), name)
                .await
                && let Ok(header) = self.client.artist_header(&cid).await
                && header.avatar.is_some()
            {
                return Ok(ArtistLookup {
                    image: header.avatar,
                    name: None,
                });
            }
        }
        Ok(ArtistLookup::default())
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, SourceError> {
        // Adding to Liked Music IS liking, so it goes through the favorite path
        // rather than a playlist mutation YT would reject. The local row is
        // written clean, not dirty — the push already happened here, and a dirty
        // row would have the reconciler push it a second time.
        if playlist_id == LIKED_MUSIC_ID {
            let sid = self.source.as_str();
            let mut added = Vec::new();
            for id in item_refs {
                if self.push_favorite(id, true).await.is_err() {
                    continue;
                }
                if self.db.set_favorite(sid, id, true).await.is_ok() {
                    let _ = self.db.clear_favorite_dirty(sid, id).await;
                    added.push(id.clone());
                }
            }
            return mirror_added(&self.db, &self.source, LIKED_MUSIC_ID, &added)
                .await
                .map(|()| added);
        }
        let mut added = Vec::new();
        for id in item_refs {
            if self.client.add_to_playlist(playlist_id, id).await.is_ok() {
                added.push(id.clone());
            }
        }
        mirror_added(&self.db, &self.source, playlist_id, &added).await?;
        Ok(added)
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, SourceError> {
        let refs: Vec<&str> = item_refs.iter().map(String::as_str).collect();
        let id = self.client.create_playlist(name, &refs).await?;
        mirror_created(&self.db, &self.source, &id, name, item_refs).await?;
        Ok(id)
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &reader::Track,
        position: usize,
    ) -> Result<(), SourceError> {
        let vid = track.id.key();
        if vid.is_empty() {
            return Err(SourceError::InvalidInput("track has no video id".into()));
        }
        // Removing from Liked Music is unliking (see `add_to_playlist`). Local
        // first so the row disappears immediately, then push, reverting the
        // local write if YT rejects it — the same order the favorites hook uses.
        if playlist_id == LIKED_MUSIC_ID {
            self.record_favorite(track, false).await?;
            if let Err(e) = self.push_favorite(&vid, false).await {
                let _ = self.record_favorite(track, true).await;
                return Err(e);
            }
            return self
                .db
                .remove_playlist_tracks(&self.source, LIKED_MUSIC_ID, &[vid.into_owned()])
                .await
                .map_err(SourceError::from);
        }
        self.client.remove_from_playlist(playlist_id, &vid).await?;
        self.remove_playlist_entry(playlist_id, position).await
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        ordered: &[reader::PlaylistEntry],
        _moved: &reader::Track,
        new_index: usize,
    ) -> Result<(), SourceError> {
        if playlist_id == LIKED_MUSIC_ID {
            return Err(SourceError::InvalidInput(
                "Liked Music keeps the order songs were liked in".into(),
            ));
        }
        // An entry this app added is stored before YouTube has said what its
        // id is, so the move is made on YouTube's own listing instead.
        let ordered = if move_plan(ordered, new_index).is_some() {
            ordered.to_vec()
        } else {
            let stored = self.db.playlist_entries(&self.source, playlist_id).await?;
            let listing = self
                .client
                .get_playlist_entries(playlist_id)
                .await?
                .iter()
                .map(reader::PlaylistEntry::from_track)
                .collect();
            moved_on_listing(&stored, ordered, new_index, listing).ok_or_else(|| {
                SourceError::InvalidInput(
                    "the playlist changed on YouTube Music, refresh it and try again".into(),
                )
            })?
        };
        let (moved, successor) = move_plan(&ordered, new_index)
            .ok_or_else(|| SourceError::InvalidInput("playlist entry has no set id".into()))?;
        self.client
            .move_playlist_item(playlist_id, moved, successor)
            .await?;
        self.db
            .set_playlist_tracks(&self.source, playlist_id, &ordered)
            .await
            .map_err(SourceError::from)
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, SourceError> {
        let info = self.client.get_stream(item_id).await?;
        Ok(StreamInfo {
            url: info.url,
            format: Some((info.format, info.range_safe)),
            user_agent: Some(info.user_agent),
            duration_secs: info.duration_secs,
            bitrate: info.bitrate,
            content_length: info.content_length,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        match self.client.validate_cookies().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.contains("cookies expired") || e.contains("signed out") => {
                AuthOutcome::Expired
            }
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, SourceError> {
        let mut ids = Vec::new();
        self.client
            .stream_liked_songs(|page| {
                ids.extend(page.into_iter().map(|t| t.id.key().into_owned()));
            })
            .await?;
        Ok(ids)
    }

    async fn push_favorite(&self, item_id: &str, on: bool) -> Result<(), SourceError> {
        if on {
            self.client.like_video(item_id).await
        } else {
            self.client.unlike_video(item_id).await
        }
        .map_err(SourceError::from)
    }

    async fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>, SourceError> {
        let mut out: Vec<PlaylistMeta> = self
            .client
            .list_playlists()
            .await?
            .into_iter()
            .map(|s| PlaylistMeta {
                id: s.id,
                name: s.title,
                image_tag: s
                    .thumbnail_url
                    .as_ref()
                    .map(|u| reader::CoverRef::encode_url(u)),
            })
            .collect();
        // YT's own library grid normally carries the Liked Music tile (with its
        // artwork), so this only fills in when that tile is missing — the grid's
        // shape is not something to depend on. Anonymous sessions have no likes,
        // so nothing is added there.
        if self.client.is_authenticated() && !out.iter().any(|p| p.id == LIKED_MUSIC_ID) {
            out.insert(
                0,
                PlaylistMeta {
                    id: LIKED_MUSIC_ID.to_string(),
                    name: "Liked Music".to_string(),
                    image_tag: None,
                },
            );
        }
        Ok(out)
    }

    async fn fetch_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<reader::Track>, SourceError> {
        if playlist_id == LIKED_MUSIC_ID {
            return self.liked_music_entries().await;
        }
        // The YT client already returns typed tracks.
        Ok(self.client.get_playlist_entries(playlist_id).await?)
    }

    async fn fetch_playlist_entries_page(
        &self,
        playlist_id: &str,
        cursor: Option<String>,
    ) -> Result<PlaylistPage, SourceError> {
        if playlist_id == LIKED_MUSIC_ID {
            // Favorites are already local, so there's nothing to page through.
            return Ok(PlaylistPage {
                tracks: self.liked_music_entries().await?,
                next: None,
                header: None,
                // Adding and removing are liking and unliking; the order is
                // the order songs were liked in.
                ops: Some(PlaylistOps::AddRemove),
            });
        }
        // True per-page InnerTube walk so a long playlist streams into the cache
        // (and the UI) instead of blocking on a full fetch every visit.
        let (tracks, next, header) = self
            .client
            .playlist_page(playlist_id, cursor.as_deref())
            .await?;
        let ops = header.as_ref().map(playlist_ops);
        Ok(PlaylistPage {
            tracks,
            next,
            header,
            ops,
        })
    }

    async fn fetch_favorites_page(
        &self,
        cursor: Option<String>,
    ) -> Result<FavoritesPage, SourceError> {
        let (tracks, next) = self.client.liked_songs_page(cursor.as_deref()).await?;
        Ok(FavoritesPage { tracks, next })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, item_id: Option<&str>) -> reader::PlaylistEntry {
        reader::PlaylistEntry {
            key: key.into(),
            item_id: item_id.map(str::to_string),
        }
    }

    #[test]
    fn a_move_lands_before_the_next_entry_or_at_the_end() {
        let ordered = [
            entry("d", Some("D")),
            entry("a", Some("A")),
            entry("b", Some("B")),
        ];
        assert_eq!(move_plan(&ordered, 0), Some(("D", Some("A"))));
        assert_eq!(move_plan(&ordered, 2), Some(("B", None)));
        let unknown = [entry("d", Some("D")), entry("a", None)];
        assert_eq!(move_plan(&unknown, 0), None, "the successor has no id");
    }

    #[test]
    fn only_an_owned_playlist_is_edited() {
        let page: serde_json::Value =
            serde_json::from_str(include_str!("../ytmusic/testdata/playlist.json")).unwrap();
        let chart = crate::ytmusic::browse::page_header(&page).expect("a header");
        assert_eq!(playlist_ops(&chart), PlaylistOps::None);
        let owned = BrowsePage {
            privacy: Some(discover::Privacy::Private),
            ..BrowsePage::default()
        };
        assert_eq!(playlist_ops(&owned), PlaylistOps::Reorder);
    }

    #[test]
    fn a_move_is_found_from_either_direction() {
        let keys = |ks: &[&str]| ks.iter().map(|k| entry(k, None)).collect::<Vec<_>>();
        let before = keys(&["a", "b", "c", "d"]);
        assert_eq!(
            moved_from(&before, &keys(&["d", "a", "b", "c"]), 0),
            Some(3)
        );
        assert_eq!(
            moved_from(&before, &keys(&["b", "c", "a", "d"]), 2),
            Some(0)
        );
        assert_eq!(moved_from(&before, &before, 1), None);
    }

    #[test]
    fn a_move_without_set_ids_is_made_on_the_listing() {
        let page: serde_json::Value =
            serde_json::from_str(include_str!("../ytmusic/testdata/playlist.json")).unwrap();
        let (remote, _) = crate::ytmusic::search::walk_playlist_shelf(&page);
        let listing: Vec<reader::PlaylistEntry> = remote[..4]
            .iter()
            .map(reader::PlaylistEntry::from_track)
            .collect();
        // Stored as added: other video ids than YouTube now lists, no set ids.
        let stored = ["w", "x", "y", "z"].map(|k| entry(k, None));
        let ordered = ["z", "w", "x", "y"].map(|k| entry(k, None));
        let moved = moved_on_listing(&stored, &ordered, 0, listing.clone()).unwrap();
        assert_eq!(
            moved,
            [&listing[3], &listing[0], &listing[1], &listing[2]].map(Clone::clone)
        );
        assert_eq!(
            move_plan(&moved, 0),
            Some((
                listing[3].item_id.as_deref().unwrap(),
                listing[0].item_id.as_deref()
            ))
        );
        assert_eq!(
            moved_on_listing(&stored[..3], &ordered[..3], 0, listing),
            None,
            "a listing of another length is not the same playlist"
        );
    }
}
