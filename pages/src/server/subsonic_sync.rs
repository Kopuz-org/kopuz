use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use reader::Library;
use reader::models::{Album, Track};
use std::collections::HashSet;
use std::path::PathBuf;

pub struct SubsonicLibraryData {
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub genres: Vec<(String, String)>,
}

pub async fn sync_server_library(
    mut library: Signal<Library>,
    config: Signal<AppConfig>,
    clear_first: bool,
) -> Result<(), String> {
    let snapshot = {
        let conf = config.read();
        let Some(server) = &conf.server else {
            return Ok(());
        };
        let Some(token) = &server.access_token else {
            return Ok(());
        };
        let Some(user_id) = &server.user_id else {
            return Ok(());
        };
        (
            server.service,
            server.url.clone(),
            token.clone(),
            user_id.clone(),
            conf.device_id.clone(),
        )
    };

    let (service, server_url, token, user_id, device_id) = snapshot;

    match service {
        MusicService::Jellyfin => {
            let remote = JellyfinClient::new(&server_url, Some(&token), &device_id, Some(&user_id));
            let mut out_albums = Vec::new();
            let mut out_tracks = Vec::new();
            let mut out_genres = Vec::new();
            let libs = remote.get_music_libraries().await?;
            for lib in libs {
                let mut album_start_index = 0;
                let album_limit = 500;
                loop {
                    let (albums, _total) = remote
                        .get_albums_paginated(&lib.id, album_start_index, album_limit)
                        .await?;

                    if albums.is_empty() {
                        break;
                    }
                    let count = albums.len();

                    for album_item in albums {
                        let image_tag = album_item
                            .image_tags
                            .as_ref()
                            .and_then(|t| t.get("Primary").cloned());

                        let cover_path = if let Some(tag) = image_tag {
                            Some(PathBuf::from(format!("jellyfin:{}:{}", album_item.id, tag)))
                        } else {
                            Some(PathBuf::from(format!("jellyfin:{}", album_item.id)))
                        };

                        out_albums.push(Album {
                            id: format!("jellyfin:{}", album_item.id),
                            title: album_item.name,
                            artist: album_item
                                .album_artist
                                .or_else(|| album_item.artists.as_ref().map(|a| a.join(", ")))
                                .unwrap_or_default(),
                            genre: album_item
                                .genres
                                .as_ref()
                                .map(|g| g.join(", "))
                                .unwrap_or_default(),
                            year: album_item.production_year.unwrap_or(0),
                            cover_path,
                        });
                    }

                    album_start_index += count;
                    if count < album_limit {
                        break;
                    }
                }

                let mut start_index = 0;
                let limit = 500;
                loop {
                    let items = remote
                        .get_music_library_items_paginated(&lib.id, start_index, limit)
                        .await?;

                    if items.is_empty() {
                        break;
                    }
                    let count = items.len();

                    for item in items {
                        let mut path_str = format!("jellyfin:{}", item.id);
                        if let Some(tags) = &item.image_tags {
                            if let Some(tag) = tags.get("Primary") {
                                path_str.push_str(&format!(":{}", tag));
                            }
                        }

                        let bitrate_kbps = item.bitrate.unwrap_or(0) / 1000;
                        let bitrate_u8 = bitrate_kbps.min(255) as u8;

                        out_tracks.push(Track {
                            path: PathBuf::from(path_str),
                            album_id: item
                                .album_id
                                .map(|id| format!("jellyfin:{}", id))
                                .unwrap_or_default(),
                            title: item.name,
                            artist: item
                                .album_artist
                                .or_else(|| item.artists.map(|a| a.join(", ")))
                                .unwrap_or_default(),
                            album: item.album.unwrap_or_default(),
                            duration: item.run_time_ticks.unwrap_or(0) / 10_000_000,
                            khz: item.sample_rate.unwrap_or(0),
                            bitrate: bitrate_u8,
                            track_number: item.index_number,
                            disc_number: item.parent_index_number,
                            musicbrainz_release_id: None,
                            playlist_item_id: None,
                        });
                    }

                    start_index += count;
                    if count < limit {
                        break;
                    }
                }

                let genres = remote.get_genres().await?;
                out_genres = genres.into_iter().map(|g| (g.name, g.id)).collect();
            }

            let mut lib_write = library.write();
            if clear_first {
                lib_write.jellyfin_tracks.clear();
                lib_write.jellyfin_albums.clear();
                lib_write.jellyfin_genres.clear();
            }
            for album in out_albums {
                if !lib_write.jellyfin_albums.iter().any(|a| a.id == album.id) {
                    lib_write.jellyfin_albums.push(album);
                }
            }
            for track in out_tracks {
                if !lib_write
                    .jellyfin_tracks
                    .iter()
                    .any(|t| t.path == track.path)
                {
                    lib_write.jellyfin_tracks.push(track);
                }
            }
            if !out_genres.is_empty() {
                lib_write.jellyfin_genres = out_genres;
            }
        }
        MusicService::Subsonic | MusicService::Custom => {
            let data = fetch_subsonic_library(service, &server_url, &user_id, &token).await?;
            let mut lib_write = library.write();
            lib_write.jellyfin_albums = data.albums;
            lib_write.jellyfin_tracks = data.tracks;
            lib_write.jellyfin_genres = data.genres;
        }
    }

    Ok(())
}

pub async fn fetch_subsonic_library(
    service: MusicService,
    server_url: &str,
    username: &str,
    password: &str,
) -> Result<SubsonicLibraryData, String> {
    let remote = SubsonicClient::new(server_url, username, password);
    let provider_prefix = match service {
        MusicService::Subsonic => "subsonic",
        MusicService::Custom => "custom",
        MusicService::Jellyfin => "jellyfin",
    };

    let mut albums_out = Vec::new();
    let mut tracks_out = Vec::new();
    let mut seen_track_ids = HashSet::new();
    let mut genres = HashSet::new();

    let mut offset = 0usize;
    let batch = 250usize;

    loop {
        let albums = remote.get_album_list(offset, batch).await?;
        if albums.is_empty() {
            break;
        }

        let count = albums.len();

        for album in albums {
            let album_cover_tag = album
                .cover_art
                .as_ref()
                .and_then(|cover_art_id| remote.cover_art_url(cover_art_id, Some(512)).ok())
                .map(|url| encode_cover_url_tag(&url));

            let album_id_prefixed = if let Some(tag) = &album_cover_tag {
                format!("{}:{}:{}", provider_prefix, album.id, tag)
            } else {
                format!("{}:{}:none", provider_prefix, album.id)
            };
            let album_name = album.name.clone();
            let album_artist = album.artist.clone().unwrap_or_default();
            let album_genre = album.genre.clone().unwrap_or_default();

            if !album_genre.is_empty() {
                genres.insert(album_genre.clone());
            }

            albums_out.push(Album {
                id: album_id_prefixed.clone(),
                title: album_name.clone(),
                artist: album_artist.clone(),
                genre: album_genre,
                year: album.year.unwrap_or(0),
                cover_path: Some(PathBuf::from(album_id_prefixed.clone())),
            });

            let songs = remote.get_album_songs(&album.id).await.map_err(|e| {
                format!("Failed to fetch songs for album '{}': {}", album.id, e)
            })?;

            for song in songs {
                if !seen_track_ids.insert(song.id.clone()) {
                    continue;
                }

                if let Some(genre) = &song.genre {
                    if !genre.is_empty() {
                        genres.insert(genre.clone());
                    }
                }

                let bitrate_u8 = song.bit_rate.unwrap_or(0).min(255) as u8;

                let song_cover_tag = song
                    .cover_art
                    .as_ref()
                    .and_then(|cover_art_id| remote.cover_art_url(cover_art_id, Some(512)).ok())
                    .map(|url| encode_cover_url_tag(&url));

                let song_path = if let Some(tag) = &song_cover_tag {
                    format!("{}:{}:{}", provider_prefix, song.id, tag)
                } else {
                    format!("{}:{}:none", provider_prefix, song.id)
                };

                tracks_out.push(Track {
                    path: PathBuf::from(song_path),
                    album_id: album_id_prefixed.clone(),
                    title: song.title,
                    artist: song.artist.unwrap_or_else(|| album_artist.clone()),
                    album: song.album.unwrap_or_else(|| album_name.clone()),
                    duration: song.duration.unwrap_or(0),
                    khz: song.sampling_rate.unwrap_or(0),
                    bitrate: bitrate_u8,
                    track_number: song.track,
                    disc_number: song.disc_number,
                    musicbrainz_release_id: None,
                    playlist_item_id: None,
                });
            }
        }

        offset += count;
        if count < batch {
            break;
        }
    }

    let mut genres_out: Vec<(String, String)> = genres
        .into_iter()
        .map(|genre| (genre.clone(), genre))
        .collect();
    genres_out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    Ok(SubsonicLibraryData {
        albums: albums_out,
        tracks: tracks_out,
        genres: genres_out,
    })
}

fn encode_cover_url_tag(url: &str) -> String {
    let mut hex = String::with_capacity(url.len() * 2);
    for b in url.as_bytes() {
        hex.push_str(&format!("{:02x}", b));
    }
    format!("urlhex_{}", hex)
}
