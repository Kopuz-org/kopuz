use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use config::Source;
use db::Db;

use crate::server_ops::ServerConn;
use crate::ytmusic::tracking::{self, Watch};
use crate::ytmusic::{SessionCheck, YouTubeMusicClient, innertube, player};

use super::{
    AlbumType, ArtistLookup, ArtistView, AuthOutcome, Capabilities, CatalogPageEntry, Check,
    FavoritesPage, FavoritesSync, LibraryActions, MediaSource, PlaylistDetails, PlaylistMeta,
    PlaylistOps, PlaylistPage, RadioPage, RadioSeeds, RemoteAlbum, SearchFilterEntry,
    SourceAccount, SourceError, StreamInfo, VideoStream, mirror_added, mirror_created,
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

/// YouTube's personal mixes (My Supermix, My Mix 1, Discover Mix) carry this playlist id prefix; their titles are localized, the prefix is not.
const PERSONAL_MIX_PREFIX: &str = "RDTMAK5uy_";

/// Home feed pages read for mixes; they sit near its top.
const MIX_PAGES: usize = 3;

fn is_personal_mix(playlist_id: &str) -> bool {
    playlist_id.starts_with(PERSONAL_MIX_PREFIX)
}

/// The mix tiles on one feed page not already in `mixes`, in feed order.
fn collect_mixes(page: &BrowsePage, mixes: &mut Vec<discover::DiscoverItem>) {
    for item in page.shelves.iter().flat_map(|shelf| shelf.items.iter()) {
        if let discover::DiscoverItem::Playlist { playlist_id, .. } = item
            && is_personal_mix(playlist_id)
            && !mixes.iter().any(|held| {
                matches!(held, discover::DiscoverItem::Playlist { playlist_id: id, .. } if id == playlist_id)
            })
        {
            mixes.push(item.clone());
        }
    }
}

struct Reported {
    item_id: String,
    watch: Watch,
}

pub(super) struct YtSource {
    db: Db,
    source: Source,
    client: YouTubeMusicClient,
    reported: Arc<Mutex<Option<Reported>>>,
    account: Option<String>,
}

impl YtSource {
    pub(super) fn new(db: Db, source: Source, conn: &ServerConn) -> Self {
        Self {
            db,
            source,
            client: YouTubeMusicClient::signed_in_as(conn.token.clone(), conn.account.clone()),
            reported: Arc::default(),
            account: conn.account.clone(),
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
        // YT has discover + radio, can add/remove playlist entries, but its
        // InnerTube exposes no reorder mutation — so no `reorder_playlist`
        // override; it inherits the unsupported default.
        Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            browse_folders: false,
            external_devices: false,
            browser_playback: false,
            accounts: self.client.is_authenticated(),
            sync: true,
            downloads: true,
            uploads: false,
            storage_quota: false,
            discover: true,
            dont_recommend: true,
            music_videos: true,
            radio: RadioSeeds {
                search: true,
                ..RadioSeeds::ALL
            },
            play_queue: false,
            playlists: PlaylistOps::AddRemove,
            artist_view: ArtistView::Remote,
            albums: AlbumType::YtMusic,
            favorites_sync: FavoritesSync::Paginated,
            account_avatar: true,
            stream_quality: true,
            explicit_flags: true,
            watch_history: self.client.is_authenticated(),
            complete_play_history: false,
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

    /// An owned playlist is deleted; a saved one is only taken out of the library.
    async fn delete_remote_playlist(&self, playlist_id: &str) -> Result<(), SourceError> {
        if playlist_id == LIKED_MUSIC_ID || !self.client.is_authenticated() {
            return Ok(());
        }
        let (_, _, header) = self.client.playlist_page(playlist_id, None).await?;
        // Only an owned playlist's header carries a privacy, so no header means
        // no telling, and guessing wrong leaves it on YouTube.
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

    async fn accounts(&self) -> Result<Vec<SourceAccount>, SourceError> {
        let listed = self.client.accounts().await?;
        let chosen = self.account.as_deref();
        Ok(listed
            .into_iter()
            .map(|account| SourceAccount {
                active: account.page_id.as_deref() == chosen,
                id: account.page_id,
                name: account.name,
                handle: account.handle,
            })
            .collect())
    }

    async fn start_radio(&self, seed_ref: &str) -> Result<RadioPage, SourceError> {
        if seed_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("track has no video id".into()));
        }
        // /next works anonymously (empty cookies), so no auth gate here.
        let page = self.client.start_mix(seed_ref).await?;
        Ok(radio_page(page))
    }

    async fn start_playlist_radio(&self, playlist_ref: &str) -> Result<RadioPage, SourceError> {
        if playlist_ref.trim().is_empty() {
            return Err(SourceError::InvalidInput("playlist has no id".into()));
        }
        // Liked Music needs no special case here: YT builds `RDAMPLLM` like any
        // other playlist mix, which is exactly what its own web client asks for.
        let page = self.client.start_playlist_mix(playlist_ref).await?;
        Ok(radio_page(page))
    }

    async fn more_radio(&self, cursor: &str) -> Result<RadioPage, SourceError> {
        let (playlist_id, token) = cursor
            .split_once(RADIO_CURSOR_SEPARATOR)
            .ok_or_else(|| SourceError::InvalidInput("not a YouTube Music radio cursor".into()))?;
        let page = self.client.mix_continuation(playlist_id, token).await?;
        // A continuation page may leave the playlist id out; the mix is the same one.
        Ok(radio_page(crate::ytmusic::mix::MixPage {
            playlist_id: page.playlist_id.clone().or(Some(playlist_id.to_string())),
            ..page
        }))
    }

    fn web_url(&self, track: &reader::Track) -> Option<String> {
        let vid = track.id.key();
        (!vid.trim().is_empty()).then(|| format!("https://music.youtube.com/watch?v={vid}"))
    }

    fn album_web_url(&self, browse_id: &str) -> Option<String> {
        (!browse_id.trim().is_empty())
            .then(|| format!("https://music.youtube.com/browse/{browse_id}"))
    }

    fn artist_web_url(&self, channel_id: &str) -> Option<String> {
        (!channel_id.trim().is_empty())
            .then(|| format!("https://music.youtube.com/channel/{channel_id}"))
    }

    fn playlist_web_url(&self, playlist_id: &str) -> Option<String> {
        let id = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        (!id.trim().is_empty()).then(|| format!("https://music.youtube.com/playlist?list={id}"))
    }

    async fn fetch_track(&self, item_id: &str) -> Result<Option<reader::Track>, SourceError> {
        if item_id.trim().is_empty() {
            return Ok(None);
        }
        self.client
            .fetch_track(item_id)
            .await
            .map_err(SourceError::from)
    }

    async fn account_avatar(&self) -> Result<Option<String>, SourceError> {
        self.client
            .account_avatar()
            .await
            .map_err(SourceError::from)
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

    async fn personal_mixes(&self) -> Result<Vec<discover::DiscoverItem>, SourceError> {
        let mut page = self.browse_page(discover::HOME, None).await?;
        let mut mixes = Vec::new();
        for read in 1..=MIX_PAGES {
            collect_mixes(&page, &mut mixes);
            let Some(token) = page.continuation.take().filter(|_| read < MIX_PAGES) else {
                break;
            };
            page = match self.browse_page(discover::HOME, Some(&token)).await {
                Ok(next) => next,
                Err(error) => {
                    tracing::debug!(%error, "a later home page for mixes failed; keeping what was read");
                    break;
                }
            };
        }
        Ok(mixes)
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
        match continuation {
            Some(token) => self.client.browse_continuation(token).await,
            None => self.client.browse_page(id).await,
        }
        .map_err(session_error)
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
        let filter = match filter {
            search::ALL => None,
            id => Some(
                search::filter(id)
                    .ok_or_else(|| SourceError::InvalidInput(format!("no such filter: {id}")))?,
            ),
        };
        if let Some(token) = continuation {
            return Ok(search::continued(
                filter,
                self.client.browse_continuation(token).await?,
            ));
        }
        if query.trim().is_empty() {
            return Ok(SearchPage::default());
        }
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
        let Some(browse_id) = crate::ytmusic::search::album_browse_id(id) else {
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
        // A credit's text names no one for sure, so only the channel it links has a photo.
        let Some(channel) = artist.id.as_deref() else {
            return Ok(ArtistLookup::default());
        };
        let header = self
            .client
            .artist_header(channel)
            .await
            .map_err(SourceError::from)?;
        Ok(ArtistLookup {
            image: header.avatar,
            name: header.name,
        })
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

    async fn resolve_stream(
        &self,
        item_id: &str,
        quality: config::StreamQuality,
    ) -> Result<StreamInfo, SourceError> {
        let info = self.client.get_stream(item_id, quality).await?;
        Ok(StreamInfo {
            url: info.url,
            format: Some((info.format, info.range_safe)),
            user_agent: Some(info.user_agent),
            duration_secs: info.duration_secs,
            bitrate: info.bitrate,
            content_length: info.content_length,
            loudness_db: info.loudness_db,
        })
    }

    /// Nothing here waits on the network: the tracking URLs and every ping go
    /// out on their own tasks.
    async fn report_playback_start(&self, item_id: &str) -> Result<(), SourceError> {
        let Some(cookies) = self.client.cookies() else {
            return Ok(());
        };
        let watch = Watch::new(0);
        let cpn = watch.cpn().to_owned();
        if let Ok(mut reported) = self.reported.lock() {
            *reported = Some(Reported {
                item_id: item_id.to_owned(),
                watch,
            });
        }
        let (reported, cookies, page_id, video_id) = (
            self.reported.clone(),
            cookies.to_owned(),
            self.client.page_id().map(str::to_owned),
            item_id.to_owned(),
        );
        // A spawned task leaves the account scope, and a play fetched or
        // reported as the Google account lands in its History, not the brand's.
        tokio::spawn(innertube::as_account(page_id.clone(), async move {
            let tracking = match player::playback_tracking(&video_id, &cookies).await {
                Ok(tracking) => tracking,
                Err(error) => {
                    tracing::debug!(%error, "no playback tracking; the play goes unreported");
                    return;
                }
            };
            let url = reported.lock().ok().and_then(|mut reported| {
                reported
                    .as_mut()
                    .filter(|r| r.watch.cpn() == cpn)
                    .map(|r| r.watch.attach(tracking))
            });
            if let Some(url) = url {
                send_report(cookies, page_id, url);
            }
        }));
        Ok(())
    }

    async fn report_playback_progress(
        &self,
        item_id: &str,
        position_ticks: u64,
        _is_paused: bool,
    ) -> Result<(), SourceError> {
        let Some(cookies) = self.client.cookies() else {
            return Ok(());
        };
        let url = self.reported.lock().ok().and_then(|mut reported| {
            reported
                .as_mut()
                .filter(|r| r.item_id == item_id)
                .and_then(|r| r.watch.on_position(position_ticks / 10_000))
        });
        if let Some(url) = url {
            send_report(
                cookies.to_owned(),
                self.client.page_id().map(str::to_owned),
                url,
            );
        }
        Ok(())
    }

    async fn report_playback_stopped(
        &self,
        item_id: &str,
        position_ticks: u64,
    ) -> Result<(), SourceError> {
        let Some(cookies) = self.client.cookies() else {
            return Ok(());
        };
        let taken = self
            .reported
            .lock()
            .ok()
            .and_then(|mut reported| reported.take_if(|r| r.item_id == item_id));
        let Some(Reported { mut watch, .. }) = taken else {
            return Ok(());
        };
        let due = watch.on_position(position_ticks / 10_000);
        for url in due.into_iter().chain(watch.finish()) {
            send_report(
                cookies.to_owned(),
                self.client.page_id().map(str::to_owned),
                url,
            );
        }
        Ok(())
    }

    async fn counterpart(
        &self,
        track: &reader::Track,
    ) -> Result<Option<reader::Counterpart>, SourceError> {
        let video_id = track.id.key();
        if video_id.trim().is_empty() {
            return Ok(None);
        }
        Ok(self.client.counterpart(&video_id).await?)
    }

    async fn video_stream(&self, item_id: &str) -> Result<VideoStream, SourceError> {
        let stream = self.client.get_video_stream(item_id).await?;
        Ok(VideoStream {
            url: stream.url,
            content_type: stream.mime,
            user_agent: Some(stream.user_agent),
            content_length: stream.content_length,
        })
    }

    async fn validate(&self) -> AuthOutcome {
        match self.client.check_session().await {
            SessionCheck::Valid => AuthOutcome::Valid,
            SessionCheck::SignedOut => AuthOutcome::Expired,
            SessionCheck::Unknown(error) => {
                tracing::warn!(%error, "YouTube Music gave no verdict on the session; keeping it");
                AuthOutcome::Unreachable
            }
        }
    }

    async fn diagnose(&self) -> Vec<Check> {
        crate::ytmusic::diagnose::run(&self.client).await
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
            });
        }
        // True per-page InnerTube walk so a long playlist streams into the cache
        // (and the UI) instead of blocking on a full fetch every visit.
        let (tracks, next, header) = self
            .client
            .playlist_page(playlist_id, cursor.as_deref())
            .await?;
        Ok(PlaylistPage {
            tracks,
            next,
            header,
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

/// A signed-out answer is an expired session, which the daemon recovers and retries.
fn session_error(error: String) -> SourceError {
    if error == innertube::SIGNED_OUT {
        SourceError::Auth
    } else {
        SourceError::Backend(error)
    }
}

fn send_report(cookies: String, page_id: Option<String>, url: String) {
    tokio::spawn(innertube::as_account(page_id, async move {
        let visitor = player::signed_in_visitor(&cookies).await;
        let endpoint = url.split('?').next().unwrap_or_default();
        match tracking::ping(&url, &cookies, visitor).await {
            Ok(status) => tracing::debug!(endpoint, status, "reported playback"),
            Err(error) => tracing::debug!(endpoint, %error, "playback report failed"),
        }
    }));
}

/// A continuation only answers alongside the mix's playlist id, so the cursor
/// carries both. Neither an `RD…` id nor a token contains a space.
const RADIO_CURSOR_SEPARATOR: char = ' ';

fn radio_page(page: crate::ytmusic::mix::MixPage) -> RadioPage {
    let more = page
        .playlist_id
        .zip(page.continuation)
        .map(|(playlist_id, token)| format!("{playlist_id}{RADIO_CURSOR_SEPARATOR}{token}"));
    RadioPage {
        tracks: page.tracks,
        more,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::MusicService;

    async fn source(token: &str) -> (YtSource, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = db::init(&dir.path().join("kopuz.db")).await.unwrap();
        let conn = ServerConn {
            service: MusicService::YtMusic,
            url: String::new(),
            token: token.to_string(),
            user_id: String::new(),
            device_id: "test".to_string(),
            apple_music_storefront: String::new(),
            apple_music_language: String::new(),
            folders: Vec::new(),
            account: None,
        };
        let src = YtSource::new(db, Source::Server("test".to_string()), &conn);
        (src, dir)
    }

    /// A ping as the anonymous identity would link it to whoever signs in next.
    #[tokio::test]
    async fn a_signed_out_source_reports_nothing() {
        let (src, _dir) = source("").await;
        src.report_playback_start("v").await.unwrap();
        src.report_playback_progress("v", 15 * 10_000_000, false)
            .await
            .unwrap();
        src.report_playback_stopped("v", 20 * 10_000_000)
            .await
            .unwrap();
        assert!(src.reported.lock().unwrap().is_none());
    }

    fn playlist(id: &str, title: &str) -> discover::DiscoverItem {
        discover::DiscoverItem::Playlist {
            playlist_id: id.to_string(),
            title: title.to_string(),
            subtitle: String::new(),
            thumbnail: None,
            actions: discover::ItemActions::default(),
        }
    }

    fn feed(items: Vec<discover::DiscoverItem>) -> BrowsePage {
        BrowsePage {
            shelves: vec![discover::DiscoverShelf {
                title: "Mixed for you".into(),
                strapline: None,
                more: None,
                items,
                layout: discover::ShelfLayout::Carousel,
                continuation: None,
                search_filter: None,
            }],
            ..BrowsePage::default()
        }
    }

    fn mix_ids(page: &BrowsePage) -> Vec<String> {
        let mut mixes = Vec::new();
        collect_mixes(page, &mut mixes);
        mixes
            .into_iter()
            .map(|item| match item {
                discover::DiscoverItem::Playlist { playlist_id, .. } => playlist_id,
                other => panic!("not a playlist: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_mix_is_known_by_its_id_prefix_and_never_by_its_title() {
        let page = feed(vec![
            playlist("RDTMAK5uy_supermix", "Something else entirely"),
            playlist("PLnotamix", "My Supermix"),
            playlist("RDCLAK5uy_curated", "Chill Mix"),
            playlist("OLAK5uy_album", "Discover Mix"),
            playlist("RDTMAK5uy_supermix", "the same mix again"),
            playlist("RDTMAK5uy_mix1", "My Mix 1"),
        ]);

        assert_eq!(mix_ids(&page), ["RDTMAK5uy_supermix", "RDTMAK5uy_mix1"]);
    }

    #[test]
    fn the_recorded_home_feed_yields_only_its_mix_tiles() {
        let first: serde_json::Value =
            serde_json::from_str(include_str!("../ytmusic/testdata/home.json")).unwrap();
        let page = browse::parse_page(&first).expect("a browse page");
        assert!(page.shelves.iter().flat_map(|s| &s.items).any(
            |item| matches!(item, discover::DiscoverItem::Playlist { playlist_id, .. } if playlist_id.starts_with("RDCLAK5uy_"))
        ));
        assert!(mix_ids(&page).is_empty(), "curated playlists are no mixes");

        let more: serde_json::Value =
            serde_json::from_str(include_str!("../ytmusic/testdata/home_continuation.json"))
                .unwrap();
        let page = browse::parse_continuation(&more);
        assert_eq!(
            mix_ids(&page),
            ["RDTMAK5uy_lr0LWzGrq6FU9GIxWvFHTRPQD2LHMqlFA"]
        );
    }

    #[tokio::test]
    async fn a_stop_for_another_track_leaves_the_play_running() {
        let (src, _dir) = source("SAPISID=abc").await;
        src.report_playback_start("v").await.unwrap();
        src.report_playback_stopped("w", 0).await.unwrap();
        assert!(src.reported.lock().unwrap().is_some());
        src.report_playback_stopped("v", 0).await.unwrap();
        assert!(src.reported.lock().unwrap().is_none());
    }
}
