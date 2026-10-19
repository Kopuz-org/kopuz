//! The load pipeline: classification, source construction on a worker
//! task, and the engine handoff. Extracted from the session actor.

use super::*;

impl Session {
    /// Two-phase load pipeline. Classification is mutation-free except stale
    /// offline-cache eviction; only after every early bail do we cancel the
    /// old load, allocate a token, and publish Loading intent.
    /// `idx` is a logical position in the live queue; `allow_crossfade` marks it
    /// as a transition candidate, which means the pointer stays on the outgoing
    /// track until the engine reports the switch.
    pub(super) fn start_load(&mut self, idx: usize, allow_crossfade: bool) -> bool {
        let Some(track) = self.model.track_at(idx).cloned() else {
            return false;
        };
        // A track only an integration can play is handed to it, and one it
        // cannot hands playback back. Both directions run through here, so
        // every way of reaching a track -- next, previous, a jump, a new queue
        // -- switches sides the same way.
        match self.sink_for(&track) {
            Some(sink) => return self.start_external_load(track, sink),
            None if self.external.is_some() => self.release_external(),
            None => {}
        }
        let track_key = track.id.uid();
        let (restore_seek, clear_pending_resume) = self.pending_resume_seek(&track);
        let transition =
            if !allow_crossfade || restore_seek.is_some_and(|position| !position.is_zero()) {
                Transition::Immediate
            } else if self.should_crossfade() {
                Transition::Crossfade(Duration::from_secs(
                    self.settings.playback.crossfade_seconds as u64,
                ))
            } else if self.should_play_gapless() {
                Transition::Gapless
            } else {
                Transition::Immediate
            };
        let deferred = transition != Transition::Immediate;
        let item_ref = PlaybackItemRef::parse(&track_key);
        let is_radio = item_ref.is_radio();
        let is_server = item_ref.is_server();
        let item_id = item_ref.primary_id().unwrap_or_default().to_string();
        let stream_id = item_ref.stream_id().unwrap_or_default().to_string();

        let factory_override = self
            .factory_override
            .as_ref()
            .and_then(|provider| provider(&track));

        let offline_path = if factory_override.is_none() && is_server {
            // Imported paths are untrusted: one with a placeholder extension streams instead.
            self.offline
                .present(&item_id)
                .map(PathBuf::from)
                .filter(|path| {
                    !matches!(
                        path.extension().and_then(|extension| extension.to_str()),
                        Some("audio") | Some("bin")
                    )
                })
        } else {
            None
        };

        let mut use_icy = false;
        let remote_ref = if factory_override.is_some() || offline_path.is_some() {
            None
        } else if is_radio {
            let station = self.station(&item_id);
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
        } else if is_server && self.sources.server.is_some() && self.active_source.is_some() {
            let cover =
                now_playing_artwork(self.sources.server.as_ref(), &track).unwrap_or_default();
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
            if allow_crossfade {
                return false;
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
                reason: None,
            });
            return false;
        }

        self.error = None;
        if is_server {
            self.ask_counterpart(&track);
        }
        self.cancel_load_task();
        self.cancel_radio_task();
        if !deferred {
            self.pending_transition = None;
        }
        let from_token = self.intent.token();
        let token = self.allocate_token();
        self.set_intent(PlaybackIntent::Loading {
            token,
            idx,
            deferred,
            from_token,
        });
        self.buffered.clear();

        if is_radio
            && let Some(station) = self
                .station(&item_id)
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

        if deferred {
            self.pending_transition = Some(PendingTransition {
                to_position: idx,
                to_key: track_key.clone(),
                to_token: token,
                from_token,
                stage: TransitionStage::Loading,
                gapless: transition == Transition::Gapless,
            });
        }

        let cover_url = if offline_path.is_some() {
            now_playing_artwork(self.sources.server.as_ref(), &track).unwrap_or_default()
        } else {
            remote_ref
                .as_ref()
                .map(|(_, cover)| cover.clone())
                .unwrap_or_default()
        };
        let artwork = if is_server || is_radio {
            Some(cover_url).filter(|cover| !cover.is_empty())
        } else {
            track.cover.clone()
        };

        if !deferred {
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
            item_id,
            use_icy,
            factory_override,
            offline_path,
            local_path,
            remote_ref,
            active_source: self.active_source.clone(),
            stream_quality: self.settings.playback.stream_quality,
            artwork,
            transition,
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
        true
    }

    pub(super) fn handle_prepared_load(
        &mut self,
        result: Result<PreparedLoad, LoadFailure>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        let mut prepared = match result {
            Ok(prepared) => prepared,
            Err(failure) => {
                if self.fail_load(failure.token, failure.code, failure.message) {
                    self.publish(state_tx, false);
                }
                return;
            }
        };
        if self.intent.token() != prepared.token {
            return;
        }
        if let Some(artwork) = self
            .model
            .track_at(prepared.idx)
            .filter(|track| track.id == prepared.track.id)
            .and_then(|track| now_playing_artwork(self.sources.server.as_ref(), track))
        {
            prepared.artwork = Some(artwork);
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
                    temp_cover_key(&prepared.track, url),
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
        let service_replay_gain = prepared.track.replay_gain;
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
            album_context: self.album_context_for_token(prepared.token),
            service_replay_gain,
            reply: Some(reply_tx),
        });
        let token = prepared.token;
        #[cfg(target_os = "macos")]
        if let Some((url, key, mut meta)) = artwork_fetch {
            let artwork_tx = self.cmd_tx.clone();
            tokio::spawn(async move {
                let Some(path) = fetch_cover_to_temp(&url, &key).await else {
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
                // A queued gapless track is not heard yet; it scrobbles from the switch.
                if !outcome.gapless {
                    let committed_track = self
                        .pending_transition
                        .as_ref()
                        .filter(|pending| pending.to_token == finished.token)
                        .map(|pending| pending.to_position)
                        .and_then(|position| self.model.track_at(position).cloned())
                        .or_else(|| self.model.current_track().cloned());
                    if let Some(track) = committed_track {
                        self.scrobble_committed(track, finished.token);
                    }
                }
                let matching_transition = self
                    .pending_transition
                    .as_ref()
                    .is_some_and(|pending| pending.to_token == finished.token);
                let mut fading_from = None;
                if matching_transition {
                    if outcome.crossfaded || outcome.gapless {
                        if let Some(pending) = self.pending_transition.as_mut() {
                            // Keep the visible queue/track outgoing until the
                            // authoritative TrackSwitched event.
                            pending.stage = if outcome.crossfaded {
                                fading_from = Some(pending.from_token);
                                TransitionStage::Fading
                            } else {
                                TransitionStage::Queued
                            };
                        }
                    } else {
                        self.commit_transition(finished.token);
                    }
                } else if !outcome.crossfaded && !outcome.gapless {
                    self.player.commit_now_playing(finished.token);
                }
                self.maybe_record_recent();
                self.publish(state_tx, false);
                // The outgoing track stays on screen through the fade, so clients get a live anchor for it to keep counting.
                if let Some(from_token) = fading_from {
                    self.publish_position_anchor(state_tx, Some(from_token), None, true);
                }
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
                if self.arm_gapless_now() {
                    self.publish(state_tx, false);
                }
            }
            Some(Err(error)) => {
                tracing::error!(error = %error, "playback failed");
                if self.fail_load(finished.token, engine_failure_code(error.kind), error) {
                    self.publish(state_tx, false);
                }
            }
            None => {
                // Engine-side cancellation is owned by the command that
                // cancelled it; token guards reject any late completion.
            }
        }
    }

    /// The integration that plays this track, if one does. A track the engine
    /// can decode is never handed over, even when the integration could also
    /// play it: a local file plays locally.
    pub(super) fn sink_for(&self, track: &Track) -> Option<crate::external::SharedExternalPlayer> {
        let sink = self.sink.as_ref()?;
        (track.id.service() == Some(sink.service())).then(|| sink.clone())
    }

    /// Hand a track to the integration. The engine stops, and what is playing
    /// comes back on the report stream rather than from a load reply -- the
    /// integration decides when it actually starts.
    fn start_external_load(
        &mut self,
        track: Track,
        sink: crate::external::SharedExternalPlayer,
    ) -> bool {
        self.cancel_load_task();
        self.pending_transition = None;
        if self.external.is_none() {
            self.attach_external_now(sink.clone());
        }
        let artwork = server::cover::track(self.sources.server.as_ref(), &track, 800)
            .map(|cover| cover.as_ref().to_string());
        tokio::spawn(async move {
            if let Err(error) = sink.load(&track, artwork).await {
                tracing::warn!(%error, "external playback could not start");
            }
        });
        true
    }

    pub(super) fn apply_album_covers(
        &mut self,
        source: config::Source,
        covers: std::collections::HashMap<String, String>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        if source != self.sources.active {
            return;
        }
        let loaded_position = self
            .pending_transition
            .as_ref()
            .map_or(self.model.current_position(), |transition| {
                transition.to_position
            });
        let mut changed = false;
        let mut loaded_changed = false;
        for position in 0..self.model.len() {
            let Some(track) = self.model.track_at_mut(position) else {
                continue;
            };
            if let Some(cover) = covers.get(&track.album_id)
                && track.cover.as_ref() != Some(cover)
            {
                track.cover = Some(cover.clone());
                changed = true;
                loaded_changed |= position == loaded_position;
            }
        }
        if loaded_changed && self.external.is_none() {
            let artwork = self
                .model
                .track_at(loaded_position)
                .and_then(|track| now_playing_artwork(self.sources.server.as_ref(), track));
            if self.player.set_artwork(self.intent.token(), artwork)
                && self.pending_transition.is_none()
            {
                self.player.commit_now_playing(self.intent.token());
            }
        }
        if changed {
            self.publish(state_tx, true);
        }
    }
}

pub(super) fn now_playing_artwork(
    server: Option<&config::MusicServer>,
    track: &Track,
) -> Option<String> {
    match server::cover::locate(server, reader::CoverRef::for_track(track), 800)? {
        server::cover::Located::File(path) => Some(path.to_string_lossy().into_owned()),
        server::cover::Located::Url(url) => Some(url),
    }
}

/// What a temp cover file is named by: a Subsonic cover's id, whose URL is signed afresh on every play, else its URL.
#[cfg(target_os = "macos")]
pub(super) fn temp_cover_key(track: &Track, url: &str) -> String {
    match reader::CoverRef::for_track(track) {
        reader::CoverRef::SubsonicItem { item_id } => format!("subsonic|{item_id}"),
        _ => url.to_string(),
    }
}

/// Download a cover to a temp file named by a hash of `key`, so repeated plays
/// of the same album reuse the file instead of growing the temp dir. MPRIS
/// `mpris:artUrl` wants a file too, not just macOS Now Playing.
pub(super) async fn fetch_cover_to_temp(url: &str, key: &str) -> Option<String> {
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
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
        let response = utils::http::client()
            .get(url)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
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
    stream_quality: config::StreamQuality,
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
                code: api::ErrorCode::Internal,
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
            }) as server::stream::stream_buffer::BufferProgressCallback
        });

        let icy_tx = if self.is_radio && self.use_icy {
            let (tx, mut rx) = watch::channel(server::stream::icy::IcyMeta::default());
            let cmd_tx = self.cmd_tx.clone();
            let token = self.token;
            tokio::spawn(async move {
                while rx.changed().await.is_ok() {
                    let meta = rx.borrow_and_update().clone();
                    if meta.title.trim().is_empty() {
                        continue;
                    }
                    let (artist, title) = server::stream::icy::split_artist_title(&meta.title);
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
        let native_stream = match &source {
            ClassifiedSource::Remote {
                stream_ref,
                source: Some(source),
            } => {
                if let ResolvedStreamRef::Pending(item_id) = ResolvedStreamRef::parse(stream_ref) {
                    source
                        .open_stream(item_id, buffer_progress.clone())
                        .await
                        .map_err(|error| LoadFailure::from_source(self.token, &error))?
                } else {
                    None
                }
            }
            _ => None,
        };
        let source = match native_stream {
            Some(stream) => ClassifiedSource::Factory(Box::new(move || {
                Ok(player::decoder::from_stream(stream))
            })),
            None => source,
        };
        let mut loudness_db = None;
        let factory: SourceFactory = match source {
            ClassifiedSource::Factory(factory) => factory,
            ClassifiedSource::Local(path) => Box::new(move || player::decoder::open_file(&path)),
            ClassifiedSource::Cached {
                path,
                source,
                item_id,
            } => {
                // The fallback resolve blocks on the runtime captured here;
                // this closure executes on the runtime-less decode worker.
                let rt_handle = tokio::runtime::Handle::current();
                let quality = self.stream_quality;
                Box::new(move || match player::decoder::open_file(&path) {
                    Ok(parts) => Ok(parts),
                    Err(error) => {
                        tracing::warn!(error = %error, "cached file failed to open; falling back to the server stream");
                        let source = source.as_ref().ok_or_else(|| {
                            std::io::Error::other("no active source for cache fallback")
                        })?;
                        if let Some(stream) = rt_handle
                            .block_on(source.open_stream(&item_id, buffer_progress.clone()))
                            .map_err(source_io_error)?
                        {
                            return Ok(player::decoder::from_stream(stream));
                        }
                        let info = rt_handle
                            .block_on(source.resolve_stream(&item_id, quality))
                            .map_err(source_io_error)?;
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
                            code: api::ErrorCode::Internal,
                            message: "no active source for remote track".to_string(),
                        })?;
                        let info = source
                            .resolve_stream(item_id, self.stream_quality)
                            .await
                            .map_err(|error| {
                                tracing::error!(error = %error, "stream URL resolve failed");
                                LoadFailure::from_source(self.token, &error)
                            })?;
                        duration_secs = info.duration_secs;
                        bitrate = info.bitrate;
                        loudness_db = info.loudness_db;
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
        self.track.replay_gain.loudness_db = loudness_db;

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
    code: api::ErrorCode,
    message: String,
}

impl LoadFailure {
    fn from_source(token: u64, error: &server::source::SourceError) -> Self {
        Self {
            token,
            code: crate::sources::source_error_code(error),
            message: error.to_string(),
        }
    }
}

/// A source's failure inside a stream factory, as the I/O error the engine tells an unreachable host by.
fn source_io_error(error: server::source::SourceError) -> std::io::Error {
    use server::source::{Refusal, SourceError};
    match error {
        SourceError::Connectivity | SourceError::Refused(Refusal::Network, _) => {
            std::io::Error::new(std::io::ErrorKind::NotConnected, error.to_string())
        }
        other => std::io::Error::other(other.to_string()),
    }
}

/// The error code a failure from the engine is shown under.
pub(super) fn engine_failure_code(kind: player::engine::FailureKind) -> api::ErrorCode {
    match kind {
        player::engine::FailureKind::Unreachable => api::ErrorCode::SourceUnreachable,
        player::engine::FailureKind::NotFound => api::ErrorCode::NotFound,
        player::engine::FailureKind::Other => api::ErrorCode::Internal,
    }
}

pub(super) struct LoadFinished {
    token: u64,
    result: Option<Result<player::engine::LoadOutcome, player::engine::Failure>>,
    clear_pending_resume: bool,
}
