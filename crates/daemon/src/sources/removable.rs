use super::*;

impl SourceService {
    pub(crate) fn watch_removable(self: &Arc<Self>) {
        if !server::audio_cd::supported() {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            let mut monitor =
                match tokio::task::spawn_blocking(server::audio_cd::Monitor::new).await {
                    Ok(Ok(monitor)) => monitor,
                    Ok(Err(error)) => {
                        tracing::debug!(%error, "optional audio CD monitor unavailable");
                        return;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "audio CD monitor failed to start");
                        return;
                    }
                };
            loop {
                let polled = tokio::task::spawn_blocking(move || {
                    let discs = monitor.poll();
                    (monitor, discs)
                })
                .await;
                let Ok((returned, discs)) = polled else {
                    return;
                };
                monitor = returned;
                match service.reconcile_discs(discs).await {
                    Ok(inserted) => {
                        for (id, disc) in inserted {
                            let service = service.clone();
                            tokio::spawn(async move {
                                if let Some(metadata) =
                                    server::audio_cd::metadata::lookup(&disc).await
                                    && let Err(error) =
                                        service.apply_disc_metadata(&id, disc, metadata).await
                                {
                                    tracing::warn!(%error, "applying CD identification failed");
                                }
                            });
                        }
                    }
                    Err(error) => tracing::warn!(%error, "updating inserted audio CDs failed"),
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }

    async fn reconcile_discs(
        &self,
        discs: Vec<(String, server::audio_cd::Disc)>,
    ) -> Result<Vec<(String, server::audio_cd::Disc)>, ApiError> {
        let mut inserted = Vec::new();
        let mut removable = self.removable.lock().await;
        let mut present = Vec::new();
        let mut changed = false;
        for (device, disc) in discs {
            // Device plus TOC keeps identical discs in different drives distinct.
            let prefix = format!("removable:cd:{device}:{}:", disc.id);
            if let Some(id) = removable.keys().find(|id| id.starts_with(&prefix)) {
                present.push(id.clone());
                continue;
            }
            // Each insertion gets a fresh id, even if a client missed the removal event.
            let id = format!("{prefix}{}", uuid::Uuid::new_v4());
            present.push(id.clone());
            let snapshot = server::source::audio_cd_snapshot(disc.clone());
            let mut server = config::MusicServer::new_with_service(
                snapshot.albums[0].title.clone(),
                device,
                config::MusicService::AudioCd,
            );
            server.id = Some(id.clone());
            let source = config::Source::Server(id.clone());
            self.db
                .upsert_tracks(&source, &snapshot.tracks)
                .await
                .map_err(db_error)?;
            self.db
                .upsert_albums(&source, &snapshot.albums)
                .await
                .map_err(db_error)?;
            inserted.push((id.clone(), disc));
            removable.insert(id, server);
            changed = true;
        }
        let removed: Vec<_> = removable
            .keys()
            .filter(|id| !present.contains(id))
            .cloned()
            .collect();
        for id in removed {
            let was_active = self.config.remove_temporary_source(&id).await;
            if was_active {
                // Wait for the source switch to stop the engine and settle queue writes.
                self.session.queue_mirror().await;
            }
            self.db
                .purge_source(&config::Source::Server(id.clone()))
                .await
                .map_err(db_error)?;
            removable.remove(&id);
            if was_active {
                self.finish_source_change();
            }
            if let Ok(mut status) = self.status.lock() {
                status.remove(&id);
            }
            changed = true;
        }
        drop(removable);
        if changed {
            self.session.invalidate(Table::Servers);
        }
        Ok(inserted)
    }

    async fn apply_disc_metadata(
        &self,
        id: &str,
        disc: server::audio_cd::Disc,
        metadata: server::audio_cd::metadata::DiscMetadata,
    ) -> Result<(), ApiError> {
        let mut removable = self.removable.lock().await;
        let Some(server) = removable.get_mut(id) else {
            return Ok(());
        };
        // The insertion id prevents a late network answer resurrecting an ejected disc.
        let source = config::Source::Server(id.to_string());
        let mut snapshot = server::source::audio_cd_snapshot(disc);
        metadata.apply(&mut snapshot);
        self.db
            .upsert_tracks(&source, &snapshot.tracks)
            .await
            .map_err(db_error)?;
        self.db
            .upsert_albums(&source, &snapshot.albums)
            .await
            .map_err(db_error)?;
        server.name = if metadata.artist.is_empty() {
            metadata.title
        } else {
            format!("{} — {}", metadata.artist, metadata.title)
        };
        self.session.update_track_metadata(&source, snapshot.tracks);
        for table in [Table::Servers, Table::Tracks, Table::Albums] {
            self.session.invalidate(table);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::PlayerApi;

    fn disc(end: i32) -> server::audio_cd::Disc {
        server::audio_cd::Disc {
            id: format!("{end:064x}"),
            tracks: vec![server::audio_cd::DiscTrack {
                number: 1,
                start: 0,
                end,
                audio: true,
            }],
        }
    }

    fn metadata() -> server::audio_cd::metadata::DiscMetadata {
        server::audio_cd::metadata::DiscMetadata {
            release_id: "release-id".into(),
            cover_url: Some("https://coverartarchive.org/release/11111111-1111-1111-1111-111111111111/front-1200".into()),
            title: "Recognized Album".into(),
            artist: "Artist".into(),
            year: 2001,
            disc_number: 1,
            tracks: vec![server::audio_cd::metadata::TrackMetadata {
                title: "Recognized Song".into(),
                artist: "Artist".into(),
                artists: vec!["Artist".into()],
                recording_id: Some("recording".into()),
                track_id: Some("track".into()),
            }],
        }
    }

    #[tokio::test]
    async fn audio_cd_insertion_requires_acceptance_and_removal_restores_saved_source() {
        let root = tempfile::tempdir().unwrap();
        let db = db::init(&root.path().join("library.db")).await.unwrap();
        let saved = config::AppConfig::default();
        let config = Arc::new(ConfigService::new(
            db.clone(),
            root.path().join("settings.toml"),
            saved.clone(),
        ));
        let library = Arc::new(crate::LibraryService::new(
            db.clone(),
            saved.active_source.clone(),
            Arc::new(radio::registry::StationRegistry::default()),
            root.path().join("covers"),
        ));
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .unwrap();
        let session = SessionHandle::spawn_with_player(
            library.clone(),
            player,
            crate::PlaybackServices {
                config: saved.clone(),
                queue_store: Some(Arc::new(crate::DbQueueStore::new(db.clone()))),
                ..Default::default()
            },
        );
        library.attach_session(session.clone());
        config.attach_session(session.clone());
        let sources = SourceService::new(db.clone(), session.clone(), config.clone());
        let api = crate::LocalApi::new(session.clone());
        sources
            .reconcile_discs(vec![("drive-a".into(), disc(750))])
            .await
            .unwrap();
        assert_eq!(config.snapshot().await.active_source, saved.active_source);
        let cd = sources
            .sources()
            .await
            .unwrap()
            .into_iter()
            .find(|source| source.temporary)
            .unwrap();
        assert!(!cd.active);
        assert!(cd.settings.is_empty());
        assert!(cd.capabilities.rip_audio);
        assert_eq!(
            db.albums(&config::Source::Server(cd.id.clone()))
                .await
                .unwrap()
                .len(),
            1
        );
        sources.switch_source(&cd.id).await.unwrap();
        api.queue_snapshot().await.unwrap();
        assert_eq!(
            session.config_watch().borrow().active_source.as_str(),
            cd.id
        );
        session
            .restore_queue(db::QueueSnapshot {
                version: 1,
                queue: server::source::audio_cd_snapshot(disc(750)).tracks,
                ..Default::default()
            })
            .await
            .unwrap();
        sources
            .apply_disc_metadata(&cd.id, disc(750), metadata())
            .await
            .unwrap();
        let queue = api.queue_snapshot().await.unwrap();
        assert!(queue.items[0].artwork.is_some());
        assert_eq!(queue.items[0].title, "Recognized Song");
        assert_eq!(
            sources.source_info(&cd.id).await.unwrap().name,
            "Artist — Recognized Album"
        );
        assert_eq!(session.state().intent, api::Intent::Stopped);
        config.set_volume(0.37).await.unwrap();
        assert_eq!(
            db.load_config().await.unwrap().unwrap().active_source,
            saved.active_source
        );
        sources.reconcile_discs(vec![]).await.unwrap();
        api.queue_snapshot().await.unwrap();
        assert_eq!(
            session.config_watch().borrow().active_source,
            saved.active_source
        );
        assert!(
            sources
                .sources()
                .await
                .unwrap()
                .iter()
                .all(|source| !source.temporary)
        );
        assert!(
            db.albums(&config::Source::Server(cd.id.clone()))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(sources.switch_source(&cd.id).await.is_err());
        sources
            .apply_disc_metadata(&cd.id, disc(750), metadata())
            .await
            .unwrap();
        assert!(
            db.albums(&config::Source::Server(cd.id.clone()))
                .await
                .unwrap()
                .is_empty()
        );

        // Swapping discs is a fresh temporary identity, without switching sources automatically.
        sources
            .reconcile_discs(vec![("drive-a".into(), disc(900))])
            .await
            .unwrap();
        let next = sources
            .sources()
            .await
            .unwrap()
            .into_iter()
            .find(|source| source.temporary)
            .unwrap();
        assert_ne!(cd.id, next.id);
        assert!(!next.active);
        // Switching away ourselves must win over subsequent physical removal.
        sources.switch_source(&next.id).await.unwrap();
        sources
            .switch_source(saved.active_source.as_str())
            .await
            .unwrap();
        sources.reconcile_discs(vec![]).await.unwrap();
        assert_eq!(config.snapshot().await.active_source, saved.active_source);
    }
    #[tokio::test]
    async fn audio_cd_cache_is_removed_after_restart_without_touching_saved_libraries() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("library.db");
        let db = db::init(&path).await.unwrap();
        let rows = server::source::audio_cd_snapshot(disc(750));
        let temporary = config::Source::Server("removable:cd:old-session".into());
        let saved = config::Source::default();
        for source in [&temporary, &saved] {
            db.upsert_tracks(source, &rows.tracks).await.unwrap();
            db.upsert_albums(source, &rows.albums).await.unwrap();
        }
        let reopened = db::init(&path).await.unwrap();
        assert!(reopened.albums(&temporary).await.unwrap().is_empty());
        assert!(
            reopened
                .tracks_by_keys(&temporary, &[rows.tracks[0].id.key().into_owned()])
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(reopened.albums(&saved).await.unwrap().len(), 1);
        assert_eq!(
            reopened
                .tracks_by_keys(&saved, &[rows.tracks[0].id.key().into_owned()])
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
