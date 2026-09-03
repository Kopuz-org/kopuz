//! The load pipeline: classification, source construction on a worker
//! task, and the engine handoff. Extracted from the session actor.

use super::*;

impl Session {
    /// Two-phase load pipeline. Classification is mutation-free except stale
    /// offline-cache eviction; only after every early bail do we cancel the
    /// old load, allocate a token, and publish Loading intent.
    pub(super) fn start_load(
        &mut self,
        idx: usize,
        allow_crossfade: bool,
        transition_model: Option<QueueModel>,
    ) {
        let source_model = transition_model.as_ref().unwrap_or(&self.model);
        let Some(track) = source_model.track_at(idx).cloned() else {
            return;
        };
        let (restore_seek, clear_pending_resume) = self.pending_resume_seek(&track);
        let use_crossfade = allow_crossfade
            && self.should_crossfade()
            && restore_seek.is_none_or(|position| position.is_zero());
        let crossfade_duration = Duration::from_secs(self.config.crossfade_seconds as u64);
        let is_radio = track.duration == u64::MAX;
        let (is_server, item_id) = match &track.id {
            reader::TrackId::Server { item_id, .. } => (true, item_id.clone()),
            reader::TrackId::Local(_) => (false, String::new()),
        };
        let (radio_id, stream_id) = if is_radio {
            let uid = track.id.uid();
            let item_ref = PlaybackItemRef::parse(&uid);
            (
                item_ref.primary_id().unwrap_or_default().to_string(),
                item_ref.stream_id().unwrap_or_default().to_string(),
            )
        } else {
            (String::new(), String::new())
        };

        let factory_override = self
            .factory_override
            .as_ref()
            .and_then(|provider| provider(&track));

        let offline_path = if factory_override.is_none() && is_server {
            let raw = self
                .config
                .offline_tracks
                .get(&item_id)
                .map(PathBuf::from)
                .filter(|path| path.exists());
            if let Some(path) = raw.as_ref() {
                let bad_ext = matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("audio") | Some("bin")
                );
                if bad_ext {
                    // Imported config paths are untrusted. Remove only the
                    // stale mapping; deleting the path could remove user data.
                    self.config.offline_tracks.remove(&item_id);
                    None
                } else {
                    raw
                }
            } else {
                raw
            }
        } else {
            None
        };

        let mut use_icy = false;
        let remote_ref = if factory_override.is_some() || offline_path.is_some() {
            None
        } else if is_radio {
            let station = self.station_registry.get(&radio_id);
            use_icy = station.is_some_and(|station| !station.has_live_metadata());
            let cover = station
                .and_then(|station| match &station.metadata {
                    Some(radio::manifest::MetadataSourceDef::Static(static_meta)) => {
                        static_meta.resolve(&stream_id).2.map(str::to_string)
                    }
                    _ => None,
                })
                .unwrap_or_default();
            station
                .and_then(|station| station.streams.iter().find(|stream| stream.id == stream_id))
                .map(|stream| (stream.url.clone(), cover))
        } else if is_server && self.config.server.is_some() && self.active_source.is_some() {
            let cover = server::cover::track(&self.config, &track, 800)
                .map(|cover| cover.as_ref().to_string())
                .unwrap_or_default();
            Some((ResolvedStreamRef::pending_marker(&item_id), cover))
        } else {
            None
        };

        let local_path = if factory_override.is_none() && !is_server && !is_radio {
            track.id.local_path().map(PathBuf::from)
        } else {
            None
        };

        if factory_override.is_none()
            && offline_path.is_none()
            && local_path.is_none()
            && remote_ref.is_none()
        {
            // A crossfade candidate that cannot resolve is dropped whole; the
            // end-of-track advance retries on the committed model and reports
            // through the branch below.
            if transition_model.is_some() {
                return;
            }
            // The caller already moved the queue pointer and will publish, so
            // a silent return would present the new track as playing while the
            // old audio continues. Fail the way a resolve failure would.
            tracing::warn!(
                queue_index = idx,
                title = %track.title,
                radio = is_radio,
                "no source can play this track"
            );
            self.cancel_load_task();
            self.cancel_radio_task();
            self.pending_transition = None;
            self.player.stop_for_transition();
            self.set_intent(PlaybackIntent::Stopped);
            self.phase = ApiPhase::Idle;
            self.buffered.clear();
            self.position = Some(PositionAnchor {
                ms: 0,
                at_ms: self.now_ms(),
                playing: false,
            });
            let message = if is_radio {
                "couldn't load this track: the radio station or stream is unknown"
            } else {
                "couldn't load this track: no connected source can provide it"
            };
            self.error = Some(api::ErrorBody {
                code: api::ErrorCode::SourceUnreachable,
                message: message.to_string(),
                details: None,
            });
            return;
        }

        self.error = None;
        self.cancel_load_task();
        self.cancel_radio_task();
        if !use_crossfade {
            self.pending_transition = None;
        }
        let from_token = self.intent.token();
        let token = self.allocate_token();
        self.set_intent(PlaybackIntent::Loading {
            token,
            idx,
            crossfade: use_crossfade,
            from_token,
        });
        self.buffered.clear();
        let source_kind = if factory_override.is_some() {
            "injected"
        } else if offline_path.is_some() {
            "offline"
        } else if local_path.is_some() {
            "local"
        } else if is_radio {
            "radio"
        } else {
            "remote"
        };
        tracing::info!(
            token,
            queue_index = idx,
            title = %track.title,
            artist = %track.artist,
            source = source_kind,
            crossfade = use_crossfade,
            resume_ms = ?restore_seek.map(|position| position.as_millis()),
            "track load started"
        );

        if is_radio
            && let Some(station) = self
                .station_registry
                .get(&radio_id)
                .filter(|station| station.has_live_metadata())
                .cloned()
        {
            use radio::provider::RadioMetadataProvider;
            let provider = radio::provider::DynamicProvider::new(station);
            let mut metadata_rx = provider.start(&stream_id);
            let cmd_tx = self.cmd_tx.clone();
            let handle = tokio::spawn(async move {
                while let Some(meta) = metadata_rx.recv().await {
                    let _ = cmd_tx.send(SessionCmd::RadioMetadata {
                        token,
                        title: meta.title,
                        artist: Some(meta.artist).filter(|artist| !artist.is_empty()),
                    });
                }
            });
            self.radio_task = Some(handle);
        }

        if use_crossfade {
            let Some(model) = transition_model else {
                self.fail_load(token, "crossfade transition has no queue candidate");
                return;
            };
            self.pending_transition = Some(PendingTransition {
                model,
                to_token: token,
                from_token,
                stage: TransitionStage::Loading,
            });
        }

        let cover_url = if offline_path.is_some() {
            server::cover::track(&self.config, &track, 800)
                .map(|cover| cover.as_ref().to_string())
                .unwrap_or_default()
        } else {
            remote_ref
                .as_ref()
                .map(|(_, cover)| cover.clone())
                .unwrap_or_default()
        };
        let artwork = if is_server || is_radio {
            Some(cover_url)
        } else {
            track.cover.clone()
        };

        if !use_crossfade {
            if is_server || is_radio {
                // Remote resolution deliberately silences the old session;
                // local files switch seamlessly inside the engine.
                self.player.stop_for_transition();
                self.phase = ApiPhase::Idle;
            }
            self.position = Some(PositionAnchor {
                ms: restore_seek.unwrap_or_default().as_millis() as u64,
                at_ms: self.now_ms(),
                playing: false,
            });
        }

        let classified = ClassifiedLoad {
            token,
            idx,
            track,
            is_radio,
            item_id: if is_radio { radio_id } else { item_id },
            use_icy,
            factory_override,
            offline_path,
            local_path,
            remote_ref,
            active_source: self.active_source.clone(),
            artwork,
            transition: if use_crossfade {
                Transition::Crossfade(crossfade_duration)
            } else {
                Transition::Immediate
            },
            start_at: restore_seek.filter(|position| !position.is_zero()),
            clear_pending_resume,
            cmd_tx: self.cmd_tx.clone(),
        };
        let tx = self.cmd_tx.clone();
        let task = tokio::spawn(async move {
            let result = classified.prepare().await;
            let _ = tx.send(SessionCmd::LoadPrepared(Box::new(result)));
        });
        self.load_task = Some((token, task));
    }

    pub(super) fn handle_prepared_load(
        &mut self,
        result: Result<PreparedLoad, LoadFailure>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(failure) => {
                if self.fail_load(failure.token, failure.message) {
                    self.publish(state_tx, false);
                }
                return;
            }
        };
        if self.intent.token() != prepared.token {
            return;
        }
        self.load_task = None;
        self.stamp_probed_stream_info(
            prepared.token,
            prepared.idx,
            prepared.duration_secs,
            prepared.bitrate,
        );

        // macOS Now Playing needs a file path (`NSImage initWithContentsOfFile`
        // cannot load a URL), so a remote cover is fetched to a temp file in
        // the background and re-pushed; the other platforms take URLs as-is.
        #[cfg(target_os = "macos")]
        let artwork_fetch = prepared
            .artwork
            .as_deref()
            .filter(|artwork| artwork.starts_with("http://") || artwork.starts_with("https://"))
            .map(|url| {
                (
                    url.to_string(),
                    NowPlayingMeta {
                        title: prepared.track.title.clone(),
                        artist: prepared.track.artist.clone(),
                        album: prepared.track.album.clone(),
                        duration: Duration::from_secs(prepared.track.duration),
                        artwork: None,
                    },
                )
            });
        let (reply_tx, reply_rx) = oneshot::channel();
        self.player.load(LoadArgs {
            token: prepared.token,
            factory: prepared.factory,
            meta: NowPlayingMeta {
                title: prepared.track.title,
                artist: prepared.track.artist,
                album: prepared.track.album,
                duration: Duration::from_secs(prepared.track.duration),
                artwork: prepared.artwork,
            },
            transition: prepared.transition,
            start_at: prepared.start_at,
            reply: Some(reply_tx),
        });
        let token = prepared.token;
        #[cfg(target_os = "macos")]
        if let Some((url, mut meta)) = artwork_fetch {
            let artwork_tx = self.cmd_tx.clone();
            tokio::spawn(async move {
                let Some(path) = fetch_cover_to_temp(&url).await else {
                    return;
                };
                meta.artwork = Some(path);
                let _ = artwork_tx.send(SessionCmd::ArtworkFetched {
                    token,
                    meta: Box::new(meta),
                });
            });
        }
        let tx = self.cmd_tx.clone();
        let task = tokio::spawn(async move {
            let result = reply_rx.await.ok();
            let _ = tx.send(SessionCmd::LoadFinished(LoadFinished {
                token,
                result,
                clear_pending_resume: prepared.clear_pending_resume,
            }));
        });
        self.load_task = Some((token, task));
        self.publish(state_tx, false);
    }

    pub(super) fn handle_load_finished(
        &mut self,
        finished: LoadFinished,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        if self.intent.token() != finished.token {
            return;
        }
        self.load_task = None;
        match finished.result {
            Some(Ok(outcome)) => {
                self.set_intent(PlaybackIntent::Committed {
                    token: finished.token,
                });
                if finished.clear_pending_resume {
                    self.pending_resume = None;
                }
                self.maybe_record_recent();
                let committed_track = self
                    .pending_transition
                    .as_ref()
                    .filter(|pending| pending.to_token == finished.token)
                    .and_then(|pending| pending.model.current_track().cloned())
                    .or_else(|| self.model.current_track().cloned());
                if let Some(track) = committed_track.as_ref() {
                    tracing::info!(
                        token = finished.token,
                        title = %track.title,
                        artist = %track.artist,
                        crossfaded = outcome.crossfaded,
                        "track load committed"
                    );
                }
                if let (Some(scrobbler), Some(track)) = (self.scrobbler.clone(), committed_track) {
                    scrobbler.track_committed(track, finished.token);
                }
                let matching_transition = self
                    .pending_transition
                    .as_ref()
                    .is_some_and(|pending| pending.to_token == finished.token);
                if matching_transition {
                    if outcome.crossfaded {
                        if let Some(pending) = self.pending_transition.as_mut() {
                            // Keep the visible queue/track outgoing until the
                            // authoritative TrackSwitched event.
                            pending.stage = TransitionStage::Fading;
                        }
                    } else {
                        self.commit_transition_model(finished.token);
                    }
                }
                self.publish(state_tx, false);
                if self.pending_transition.is_none()
                    && self.phase != ApiPhase::Idle
                    && self.position_token != Some(finished.token)
                {
                    self.publish_position_anchor(
                        state_tx,
                        Some(finished.token),
                        None,
                        self.phase == ApiPhase::Playing,
                    );
                }
            }
            Some(Err(error)) => {
                tracing::error!(token = finished.token, error = %error, "playback failed");
                if self.fail_load(finished.token, error) {
                    self.publish(state_tx, false);
                }
            }
            None => {
                // Engine-side cancellation is owned by the command that
                // cancelled it; token guards reject any late completion.
            }
        }
    }
}

/// Download a cover to a temp file named by its URL hash, so repeated plays
/// of the same album reuse the file instead of growing the temp dir.
#[cfg(target_os = "macos")]
async fn fetch_cover_to_temp(url: &str) -> Option<String> {
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let digest = hasher.finalize();
    let mut name = String::from("kopuz_cover_");
    for byte in &digest[..12] {
        use std::fmt::Write as _;
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".jpg");
    let path = std::env::temp_dir().join(name);
    if tokio::fs::metadata(&path).await.is_ok() {
        return Some(path.to_string_lossy().to_string());
    }
    let fetch = async {
        let response = reqwest::get(url).await.ok()?.error_for_status().ok()?;
        response.bytes().await.ok()
    };
    let bytes = tokio::time::timeout(Duration::from_secs(30), fetch)
        .await
        .ok()??;
    tokio::fs::write(&path, &bytes).await.ok()?;
    Some(path.to_string_lossy().to_string())
}

pub(super) enum ClassifiedSource {
    Factory(SourceFactory),
    Local(PathBuf),
    Cached {
        path: PathBuf,
        source: Option<server::source::ActiveSource>,
        item_id: String,
    },
    Remote {
        stream_ref: String,
        source: Option<server::source::ActiveSource>,
    },
}

pub(super) struct ClassifiedLoad {
    token: u64,
    idx: usize,
    track: Track,
    is_radio: bool,
    item_id: String,
    use_icy: bool,
    factory_override: Option<SourceFactory>,
    offline_path: Option<PathBuf>,
    local_path: Option<PathBuf>,
    remote_ref: Option<(String, String)>,
    active_source: Option<server::source::ActiveSource>,
    artwork: Option<String>,
    transition: Transition,
    start_at: Option<Duration>,
    clear_pending_resume: bool,
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
}

impl ClassifiedLoad {
    async fn prepare(mut self) -> Result<PreparedLoad, LoadFailure> {
        let source = if let Some(factory) = self.factory_override.take() {
            ClassifiedSource::Factory(factory)
        } else if let Some(path) = self.local_path.take() {
            ClassifiedSource::Local(path)
        } else if let Some(path) = self.offline_path.take() {
            ClassifiedSource::Cached {
                path,
                source: self.active_source.clone(),
                item_id: self.item_id.clone(),
            }
        } else {
            let (stream_ref, _) = self.remote_ref.take().ok_or_else(|| LoadFailure {
                token: self.token,
                message: "classified load has no source".to_string(),
            })?;
            ClassifiedSource::Remote {
                stream_ref,
                source: self.active_source.clone(),
            }
        };

        let buffer_progress = (!self.is_radio).then(|| {
            let tx = self.cmd_tx.clone();
            let token = self.token;
            Arc::new(move |start, end, total| {
                let _ = tx.send(SessionCmd::BufferProgress(BufferProgressEvent {
                    token,
                    start,
                    end,
                    total,
                }));
            }) as utils::stream_buffer::BufferProgressCallback
        });

        let icy_tx = if self.is_radio && self.use_icy {
            let (tx, mut rx) = watch::channel(utils::icy::IcyMeta::default());
            let cmd_tx = self.cmd_tx.clone();
            let token = self.token;
            tokio::spawn(async move {
                while rx.changed().await.is_ok() {
                    let meta = rx.borrow_and_update().clone();
                    if meta.title.trim().is_empty() {
                        continue;
                    }
                    let (artist, title) = utils::icy::split_artist_title(&meta.title);
                    let _ = cmd_tx.send(SessionCmd::RadioMetadata {
                        token,
                        title,
                        artist,
                    });
                }
            });
            Some(tx)
        } else {
            None
        };

        let mut duration_secs = None;
        let mut bitrate = None;
        let factory: SourceFactory = match source {
            ClassifiedSource::Factory(factory) => factory,
            ClassifiedSource::Local(path) => Box::new(move || {
                player::decoder::open_file(&path).map_err(|error| error.to_string())
            }),
            ClassifiedSource::Cached {
                path,
                source,
                item_id,
            } => {
                // The fallback resolve blocks on the runtime captured here;
                // this closure executes on the runtime-less decode worker.
                let rt_handle = tokio::runtime::Handle::current();
                Box::new(move || match player::decoder::open_file(&path) {
                    Ok(parts) => Ok(parts),
                    Err(error) => {
                        tracing::warn!(error = %error, "cached file failed to open; falling back to the server stream");
                        let source = source
                            .as_ref()
                            .ok_or_else(|| "no active source for cache fallback".to_string())?;
                        let info = rt_handle
                            .block_on(source.resolve_stream(&item_id))
                            .map_err(|error| error.to_string())?;
                        network_factory(
                            info.url,
                            info.format,
                            info.user_agent,
                            false,
                            None,
                            rt_handle.clone(),
                            buffer_progress.clone(),
                        )()
                    }
                })
            }
            ClassifiedSource::Remote { stream_ref, source } => {
                let (stream_url, format, user_agent) = match ResolvedStreamRef::parse(&stream_ref) {
                    ResolvedStreamRef::Pending(item_id) => {
                        let source = source.ok_or_else(|| LoadFailure {
                            token: self.token,
                            message: "no active source for remote track".to_string(),
                        })?;
                        let info = source.resolve_stream(item_id).await.map_err(|error| {
                            tracing::error!(error = %error, "stream URL resolve failed");
                            LoadFailure {
                                token: self.token,
                                message: error.to_string(),
                            }
                        })?;
                        duration_secs = info.duration_secs;
                        bitrate = info.bitrate;
                        (info.url, info.format, info.user_agent)
                    }
                    ResolvedStreamRef::SoundCloudHls(_)
                    | ResolvedStreamRef::AppleMusicFmp4(_)
                    | ResolvedStreamRef::Direct(_) => (stream_ref, None, None),
                };

                // The factory runs on the decode worker (no runtime), so hand
                // every blocking stream/decrypt path this task's handle.
                let rt_handle = tokio::runtime::Handle::current();
                network_factory(
                    stream_url,
                    format,
                    user_agent,
                    self.is_radio,
                    icy_tx,
                    rt_handle,
                    buffer_progress,
                )
            }
        };

        if let Some(duration) = duration_secs.filter(|duration| *duration > 0) {
            self.track.duration = duration;
        }
        if let Some(bits_per_second) = bitrate {
            self.track.bitrate = (bits_per_second / 1000) as u16;
        }

        Ok(PreparedLoad {
            token: self.token,
            idx: self.idx,
            track: self.track,
            factory,
            artwork: self.artwork,
            transition: self.transition,
            start_at: self.start_at,
            clear_pending_resume: self.clear_pending_resume,
            duration_secs,
            bitrate,
        })
    }
}

pub(super) struct PreparedLoad {
    token: u64,
    idx: usize,
    track: Track,
    factory: SourceFactory,
    artwork: Option<String>,
    transition: Transition,
    start_at: Option<Duration>,
    clear_pending_resume: bool,
    duration_secs: Option<u64>,
    bitrate: Option<u32>,
}

pub(super) struct LoadFailure {
    token: u64,
    message: String,
}

pub(super) struct LoadFinished {
    token: u64,
    result: Option<Result<player::engine::LoadOutcome, String>>,
    clear_pending_resume: bool,
}
