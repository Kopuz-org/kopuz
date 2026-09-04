use std::path::Path;

use super::*;

impl FrontendService {
    pub async fn playlists(&self) -> Result<api::PlaylistCatalog, ApiError> {
        let config = self.current().await;
        let store = self
            .db
            .load_playlists(&config.active_source)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::playlist_catalog(&store))
    }

    pub async fn playlist_tracks(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let playlist = self.playlist(&config, &request.id).await?;
        let total = playlist.tracks.len().min(u32::MAX as usize) as u32;
        let keys: Vec<String> = playlist
            .tracks
            .iter()
            .skip(request.page.offset as usize)
            .take(request.page.limit as usize)
            .cloned()
            .collect();
        let tracks = self
            .db
            .tracks_by_keys(&config.active_source, &keys)
            .await
            .map_err(Self::db_error)?;
        Ok(api::TrackPage {
            total,
            offset: request.page.offset,
            items: tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn refresh_playlist(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<api::TrackPage, ApiError> {
        if request.id.trim().is_empty() {
            return Err(ApiError::invalid_input("playlist id is required"));
        }
        let config = self.current().await;
        let source = self.source().await;
        if source.capabilities().sync {
            let mut cursor = None;
            let mut tracks = Vec::new();
            let mut seen = std::collections::HashSet::new();
            loop {
                let page = source
                    .fetch_playlist_entries_page(&request.id, cursor)
                    .await
                    .map_err(Self::source_error)?;
                let next = page.next;
                tracks.extend(page.tracks.into_iter().filter(|track| {
                    let key = track.id.key().into_owned();
                    !key.is_empty() && seen.insert(key)
                }));
                match next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            for chunk in tracks.chunks(100) {
                self.db
                    .upsert_tracks(&config.active_source, chunk)
                    .await
                    .map_err(Self::db_error)?;
            }
            let keys: Vec<String> = tracks
                .iter()
                .map(|track| track.id.key().into_owned())
                .collect();
            self.db
                .set_playlist_tracks(&config.active_source, &request.id, &keys)
                .await
                .map_err(Self::db_error)?;
            self.library.invalidate(api::Table::Tracks);
            self.library.invalidate(api::Table::Playlists);
        }
        self.playlist_tracks(request).await
    }

    pub async fn create_playlist(&self, name: &str, keys: &[String]) -> Result<String, ApiError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ApiError::invalid_input("playlist name is required"));
        }
        let id = self
            .source()
            .await
            .create_playlist(name, keys)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Playlists);
        Ok(id)
    }

    pub async fn rename_playlist(&self, id: &str, name: &str) -> Result<(), ApiError> {
        let name = name.trim();
        if id.is_empty() || name.is_empty() {
            return Err(ApiError::invalid_input("playlist id and name are required"));
        }
        let config = self.current().await;
        let playlist = self.playlist(&config, id).await?;
        self.db
            .upsert_playlist_meta(
                &config.active_source,
                id,
                name,
                playlist.cover_path.as_deref().and_then(Path::to_str),
                playlist.image_tag.as_deref(),
            )
            .await
            .map_err(Self::db_error)?;
        self.library.invalidate(api::Table::Playlists);
        Ok(())
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<(), ApiError> {
        if id.is_empty() {
            return Err(ApiError::invalid_input("playlist id is required"));
        }
        self.source()
            .await
            .delete_playlist(id)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Playlists);
        self.library.invalidate(api::Table::Folders);
        Ok(())
    }

    pub async fn add_playlist_tracks(&self, id: &str, keys: &[String]) -> Result<(), ApiError> {
        self.source()
            .await
            .add_to_playlist(id, keys)
            .await
            .map(|_| ())
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Playlists);
        Ok(())
    }

    pub async fn remove_playlist_tracks(&self, id: &str, keys: &[String]) -> Result<(), ApiError> {
        let config = self.current().await;
        let playlist = self.playlist(&config, id).await?;
        let source = self.source().await;
        let mut remaining = playlist.tracks.clone();
        for key in keys {
            let Some(position) = remaining.iter().position(|item| item == key) else {
                continue;
            };
            let Some(track) = self
                .db
                .tracks_by_keys(&config.active_source, std::slice::from_ref(key))
                .await
                .map_err(Self::db_error)?
                .into_iter()
                .next()
            else {
                continue;
            };
            source
                .remove_from_playlist(id, &track, position)
                .await
                .map_err(Self::source_error)?;
            remaining.remove(position);
        }
        self.library.invalidate(api::Table::Playlists);
        Ok(())
    }

    pub async fn reorder_playlist_tracks(&self, id: &str, keys: &[String]) -> Result<(), ApiError> {
        let config = self.current().await;
        let playlist = self.playlist(&config, id).await?;
        let mut current_keys = playlist.tracks.clone();
        let mut requested_keys = keys.to_vec();
        current_keys.sort_unstable();
        requested_keys.sort_unstable();
        if current_keys != requested_keys {
            return Err(ApiError::invalid_input(
                "a reorder must contain every playlist track",
            ));
        }
        let Some(first) = keys
            .iter()
            .enumerate()
            .find(|(index, key)| playlist.tracks.get(*index) != Some(*key))
            .map(|(index, _)| index)
        else {
            return Ok(());
        };
        let last = keys
            .iter()
            .enumerate()
            .rfind(|(index, key)| playlist.tracks.get(*index) != Some(*key))
            .map(|(index, _)| index)
            .unwrap_or(first);
        let moved_later = playlist.tracks[first] == keys[last]
            && playlist.tracks[first + 1..=last] == keys[first..last];
        let moved_earlier = playlist.tracks[last] == keys[first]
            && playlist.tracks[first..last] == keys[first + 1..=last];
        let (moved_key, new_index) = if moved_later {
            (&keys[last], last)
        } else if moved_earlier {
            (&keys[first], first)
        } else {
            return Err(ApiError::invalid_input(
                "a reorder must describe one moved playlist track",
            ));
        };
        let track = self
            .db
            .tracks_by_keys(&config.active_source, std::slice::from_ref(moved_key))
            .await
            .map_err(Self::db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("moved track not found"))?;
        self.source()
            .await
            .reorder_playlist(id, keys, &track, new_index)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Playlists);
        Ok(())
    }

    pub async fn create_folder(&self, name: &str) -> Result<String, ApiError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ApiError::invalid_input("folder name is required"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.source()
            .await
            .create_folder(&id, name)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Folders);
        Ok(id)
    }

    pub async fn rename_folder(&self, id: &str, name: &str) -> Result<(), ApiError> {
        let name = name.trim();
        if id.is_empty() || name.is_empty() {
            return Err(ApiError::invalid_input("folder id and name are required"));
        }
        self.source()
            .await
            .rename_folder(id, name)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Folders);
        Ok(())
    }

    pub async fn delete_folder(&self, id: &str) -> Result<(), ApiError> {
        if id.is_empty() {
            return Err(ApiError::invalid_input("folder id is required"));
        }
        self.source()
            .await
            .delete_folder(id)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Folders);
        Ok(())
    }

    pub async fn move_playlist(&self, id: &str, folder_id: Option<&str>) -> Result<(), ApiError> {
        self.source()
            .await
            .set_playlist_folder(id, folder_id)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Folders);
        Ok(())
    }
}
