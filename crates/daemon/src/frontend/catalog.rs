use super::*;

impl FrontendService {
    fn catalog_page(
        &self,
        home: server::ytmusic::discover::DiscoverHome,
        config: &config::AppConfig,
    ) -> api::CatalogPage {
        use server::ytmusic::discover::DiscoverItem;
        let transient: Vec<reader::Track> = home
            .shelves
            .iter()
            .flat_map(|shelf| shelf.items.iter())
            .filter_map(|item| match item {
                DiscoverItem::Song(track) => Some((**track).clone()),
                _ => None,
            })
            .collect();
        self.library.register_transient(&transient);
        api::CatalogPage {
            shelves: home
                .shelves
                .into_iter()
                .map(|shelf| api::CatalogShelf {
                    title: shelf.title,
                    strapline: shelf.strapline,
                    more_ref: shelf.more_browse_id,
                    list: shelf.is_song_list,
                    items: shelf
                        .items
                        .into_iter()
                        .map(|item| match item {
                            DiscoverItem::Song(track) => api::CatalogItem {
                                kind: api::CatalogItemKind::Track,
                                id: track.id.key().into_owned(),
                                title: track.title.clone(),
                                subtitle: Some(track.artist.clone()),
                                artwork: track.cover.clone(),
                                track: Some(crate::wire::track_info(&track, config)),
                            },
                            DiscoverItem::Playlist {
                                playlist_id,
                                title,
                                subtitle,
                                thumbnail,
                            } => api::CatalogItem {
                                kind: api::CatalogItemKind::Playlist,
                                id: playlist_id,
                                title,
                                subtitle: Some(subtitle),
                                artwork: thumbnail,
                                track: None,
                            },
                            DiscoverItem::Album {
                                browse_id,
                                title,
                                subtitle,
                                thumbnail,
                            } => api::CatalogItem {
                                kind: api::CatalogItemKind::Album,
                                id: browse_id,
                                title,
                                subtitle: Some(subtitle),
                                artwork: thumbnail,
                                track: None,
                            },
                            DiscoverItem::Artist {
                                channel_id,
                                name,
                                thumbnail,
                            } => api::CatalogItem {
                                kind: api::CatalogItemKind::Artist,
                                id: channel_id,
                                title: name,
                                subtitle: None,
                                artwork: thumbnail,
                                track: None,
                            },
                            DiscoverItem::Mood {
                                browse_id,
                                title,
                                thumbnail,
                            } => api::CatalogItem {
                                kind: api::CatalogItemKind::Mood,
                                id: browse_id,
                                title,
                                subtitle: None,
                                artwork: thumbnail,
                                track: None,
                            },
                        })
                        .collect(),
                })
                .collect(),
            continuation: home.continuation,
        }
    }

    pub async fn catalog(&self, continuation: Option<&str>) -> Result<api::CatalogPage, ApiError> {
        let config = self.current().await;
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), &config));
        let home = match continuation {
            Some(token) => source.discover_continuation(token).await,
            None => source.discover_home().await,
        }
        .map_err(Self::source_error)?;
        Ok(self.catalog_page(home, &config))
    }

    pub async fn catalog_detail(
        &self,
        request: api::CatalogDetailRequest,
    ) -> Result<api::CatalogDetail, ApiError> {
        if request.id.trim().is_empty() {
            return Err(ApiError::invalid_input("catalog id is required"));
        }
        let config = self.current().await;
        let source = self.source().await;
        match request.kind {
            api::CatalogItemKind::Album => {
                let album = match source
                    .fetch_album_by_ref(&request.id)
                    .await
                    .map_err(Self::source_error)?
                {
                    Some(album) => album,
                    None => source
                        .fetch_album(&request.id)
                        .await
                        .map_err(Self::source_error)?,
                };
                self.library.register_transient(&album.tracks);
                Ok(api::CatalogDetail {
                    kind: api::CatalogItemKind::Album,
                    id: album.browse_id,
                    title: album.title,
                    subtitle: album.artist,
                    description: None,
                    artwork: album.thumbnail,
                    playback_id: album.audio_playlist_id,
                    year: album.year,
                    tracks: album
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, &config))
                        .collect(),
                    shelves: Vec::new(),
                    continuation: None,
                })
            }
            api::CatalogItemKind::Playlist => {
                let page = source
                    .fetch_playlist_entries_page(&request.id, request.continuation)
                    .await
                    .map_err(Self::source_error)?;
                self.library.register_transient(&page.tracks);
                Ok(api::CatalogDetail {
                    kind: api::CatalogItemKind::Playlist,
                    id: request.id.clone(),
                    title: request.id,
                    tracks: page
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, &config))
                        .collect(),
                    continuation: page.next,
                    ..Default::default()
                })
            }
            api::CatalogItemKind::Artist => {
                let channel_id = if request.id.starts_with("UC") {
                    request.id
                } else {
                    source
                        .resolve_artist_channel_id(request.id.trim())
                        .await
                        .map_err(Self::source_error)?
                        .ok_or_else(|| ApiError::not_found("catalog artist not found"))?
                };
                let artist = source
                    .fetch_artist(&channel_id)
                    .await
                    .map_err(Self::source_error)?;
                let page = self.catalog_page(
                    server::ytmusic::discover::DiscoverHome {
                        shelves: artist.sections,
                        continuation: None,
                    },
                    &config,
                );
                Ok(api::CatalogDetail {
                    kind: api::CatalogItemKind::Artist,
                    id: artist.channel_id,
                    title: artist.name,
                    subtitle: artist.subscribers,
                    description: artist.description,
                    artwork: artist.banner_thumbnail,
                    playback_id: artist.shuffle_playlist_id,
                    tracks: Vec::new(),
                    shelves: page.shelves,
                    continuation: page.continuation,
                    year: None,
                })
            }
            api::CatalogItemKind::Track
            | api::CatalogItemKind::Mood
            | api::CatalogItemKind::Unknown => {
                Err(ApiError::unsupported("catalog detail kind is unsupported"))
            }
        }
    }
}
