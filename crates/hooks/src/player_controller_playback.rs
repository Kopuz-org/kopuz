//! Playback classification, stream resolution, and engine transitions.

use std::time::Duration;

use config::MusicService;
use dioxus::logger::tracing::Instrument;
use dioxus::{logger::tracing, prelude::*};
use player::decoder;
use player::engine::{SourceFactory, Transition};
use player::player::{LoadArgs, NowPlayingMeta};

use crate::playback_ref::{PlaybackItemRef, ResolvedStreamRef};
use crate::scrobble_scheduler::{self, ScrobbleOptions};
use crate::use_player_controller::{PlaybackIntent, PlayerController, network_factory};

impl PlayerController {
    /// Remap an index after moving one queue item from `from` to `to`.
    pub(crate) fn remap_queue_index(index: usize, from: usize, to: usize) -> usize {
        if index == from {
            to
        } else if from < to && index > from && index <= to {
            index - 1
        } else if to < from && index >= to && index < from {
            index + 1
        } else {
            index
        }
    }

    pub fn should_crossfade(&self) -> bool {
        self.config.peek().crossfade_seconds > 0
            && *self.is_playing.peek()
            && self.player.peek().can_resume()
    }

    pub fn has_next_track(&self) -> bool {
        // Delegates to the same predicate the advance uses, so the crossfade
        // arm can't fire when the advance would instead end the queue.
        Self::has_following_track(
            *self.current_queue_index.peek(),
            self.queue.peek().len(),
            *self.loop_mode.peek(),
        )
    }

    pub fn play_track(&mut self, idx: usize) {
        let current_idx = *self.current_queue_index.peek();
        self.history.with_mut(|h| {
            if h.last() != Some(&current_idx) {
                h.push(current_idx);
            }
        });

        if *self.shuffle.peek() {
            // workaround: shuffle enable/disable needed to play the selected track when shuffle is enabled
            self.shuffle.set(false);
            self.play_track_no_history_without_crossfade(idx);
            self.shuffle.set(true);
            self.rebuild_shuffle_order();
        } else {
            self.play_track_no_history_without_crossfade(idx);
        }
    }

    pub fn play_track_no_history(&mut self, idx: usize) {
        self.play_track_no_history_with_transition(idx, false);
    }

    pub fn play_track_no_history_without_crossfade(&mut self, idx: usize) {
        self.play_track_no_history_with_transition(idx, false);
    }

    #[tracing::instrument(name = "player.transition", skip(self), fields(idx, crossfade = allow_crossfade))]
    pub(crate) fn play_track_no_history_with_transition(
        &mut self,
        idx: usize,
        allow_crossfade: bool,
    ) {
        // ── Phase 1: classify — no mutation (bar stale-cache eviction), so
        // every early bail below leaves no half-set loading state behind. ──
        let Some(track) = self.get_track_at(idx) else {
            return;
        };

        let path_str = track.id.uid().to_string();
        let (restore_seek_secs, clear_pending_resume_on_success) = self.pending_resume_seek(&track);
        let use_crossfade = allow_crossfade
            && self.should_crossfade()
            && restore_seek_secs.is_none_or(|secs| secs == 0);
        let crossfade_duration = Duration::from_secs(self.config.peek().crossfade_seconds as u64);
        let item_ref = PlaybackItemRef::parse(&path_str);
        let is_radio_item = item_ref.is_radio();
        let is_server_item = item_ref.is_server();
        let id = item_ref.primary_id().unwrap_or_default().to_string();
        let stream_id = item_ref.stream_id().unwrap_or_default().to_string();

        let is_spotify_item = track.id.service() == Some(MusicService::Spotify);
        if !is_spotify_item {
            self.stop_external_playback();
        } else if self.spotify_access().is_some() {
            self.playback_error.set(None);
            self.cancel_radio_task();
            self.cancel_load_task();
            self.clear_pending_crossfade_ui();
            self.player.peek().stop_for_transition();
            let token = self.allocate_token();
            self.set_intent(PlaybackIntent::Committed { token });
            self.external_active.set(true);
            self.spotify_progress_anchor
                .set(Some((0, std::time::Instant::now())));
            self.hydrate_current_track_metadata(idx, 0);
            self.is_playing.set(true);
            self.spotify_play(&id, &track);
            scrobble_scheduler::schedule(
                track.clone(),
                Some(id.clone()),
                self.config,
                self.current_token,
                token,
                self.is_playing,
                Some(self.active_source),
                ScrobbleOptions::REMOTE_NATIVE,
                self.db.peek().clone(),
            );
            return;
        }

        // ── classify the source ─────────────────────────────────────────

        // Offline cache first (server items only).
        let offline_path: Option<std::path::PathBuf> = if is_server_item {
            let raw = self
                .config
                .read()
                .offline_tracks
                .get(&id)
                .map(std::path::PathBuf::from)
                .filter(|p| p.exists());
            // Evict stale entries saved with the wrong ".audio"/".bin" fallback
            if let Some(ref p) = raw {
                let bad_ext = matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("audio") | Some("bin")
                );
                if bad_ext {
                    // The persisted path is untrusted configuration. Dropping
                    // the stale mapping is safe; deleting it here could remove
                    // an arbitrary user file from an imported config.
                    self.config.write().offline_tracks.remove(&id);
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

        // Remote stream reference + synchronous cover URL for server/radio
        // items that aren't cached offline. Streams resolve in the load task;
        // only the cover is built here so artwork shows immediately on click.
        // ICY titles only for stations without a live metadata provider.
        let mut use_icy = false;
        let remote_ref: Option<(String, String)> = if offline_path.is_some() {
            None
        } else if is_radio_item {
            let registry = self.station_registry.read();
            let station = registry.get(&id);
            use_icy = station.is_some_and(|s| !s.has_live_metadata());
            // Static cover/favicon so artwork shows from the first frame.
            let cover = station
                .and_then(|s| match &s.metadata {
                    Some(radio::manifest::MetadataSourceDef::Static(st)) => {
                        st.resolve(&stream_id).2.map(str::to_string)
                    }
                    _ => None,
                })
                .unwrap_or_default();
            station
                .and_then(|s| s.streams.iter().find(|str| str.id == stream_id))
                .map(|s| s.url.clone())
                .map(|stream_url| (stream_url, cover))
        } else if is_server_item {
            // Every server source resolves its stream async in the load task, so
            // the ref is a pending marker; only the cover is built now, through
            // the cover seam. No creds no longer bails silently — resolve_stream
            // surfaces a real error instead.
            if self.config.read().server.is_some() {
                let cover_url = ::server::cover::track(&self.config.read(), &track, 800)
                    .map(|cover| cover.as_ref().to_string())
                    .unwrap_or_default();
                Some((ResolvedStreamRef::pending_marker(&id), cover_url))
            } else {
                None
            }
        } else {
            None
        };

        let local_path: Option<std::path::PathBuf> = if is_server_item || is_radio_item {
            None
        } else {
            track.id.local_path().map(|p| p.to_path_buf())
        };

        // A server item with no server configured has nothing to resolve — stop
        // silently, as the old sync path did.
        if offline_path.is_none() && local_path.is_none() && remote_ref.is_none() {
            return;
        }

        // ── Phase 2: commit — mutates state, so only past every bail above ──
        // (Cancelling the prior load is just a resource optimization; the token
        // is what guards correctness.)
        self.playback_error.set(None);
        self.cancel_radio_task();
        self.cancel_load_task();
        if !use_crossfade {
            self.clear_pending_crossfade_ui();
        }
        let from_token = self.intent.peek().token();
        let token = self.allocate_token();
        self.set_intent(PlaybackIntent::Loading {
            token,
            idx,
            crossfade: use_crossfade,
            from_token,
        });
        self.buffered_ranges.set(Vec::new());

        let cover_url: String = if offline_path.is_some() {
            self.cover_url_for_track(&track)
        } else if let Some((_, cover)) = &remote_ref {
            cover.clone()
        } else {
            String::new()
        };
        let artwork = if is_server_item || is_radio_item {
            Some(cover_url.clone())
        } else {
            // For a local track `track.cover` is its album-art file path
            // (projected from the album by the DB read layer).
            track.cover.clone()
        };

        // ── UI transition ───────────────────────────────────────────────
        if !use_crossfade {
            if is_server_item || is_radio_item {
                // Deliberate UX: silence while a (possibly slow) load resolves.
                // Pure local files switch seamlessly inside the engine instead.
                self.player.peek().stop_for_transition();
                self.is_playing.set(false);
            }
            self.hydrate_current_track_metadata(idx, restore_seek_secs.unwrap_or(0));
            if is_server_item || is_radio_item {
                self.current_song_cover_url.set(cover_url.clone());
            }
        }

        // ── the load pipeline ───────────────────────────────────────────
        // One cancellable task for every source kind: resolve the stream URL
        // if needed, hand the engine a source factory (executed on its decode
        // worker thread, so network buffering never blocks the UI), then apply
        // the post-load bookkeeping once the engine confirms playback.
        let mut ctrl = *self;
        let phys_idx = self.get_queue_index(idx);
        let station_id = id.clone();
        let cached_item_id = id;

        let task = spawn(
            async move {
                // ICY channel: tx goes to the stream download, rx to the
                // metadata follower after load commits.
                let (icy_tx, icy_rx) = if is_radio_item && use_icy {
                    let (tx, rx) = tokio::sync::watch::channel(utils::icy::IcyMeta::default());
                    (Some(tx), Some(rx))
                } else {
                    (None, None)
                };
                let buffer_progress = (!is_radio_item)
                    .then(|| ctrl.buffer_progress_callback(token));

                let factory: SourceFactory = if let Some(path) = local_path {
                    Box::new(move || decoder::open_file(&path).map_err(|e| e.to_string()))
                } else if let Some(path) = offline_path {
                    // Cached server file: on open failure fall back to the live
                    // stream instead of failing. The resolve blocks on this
                    // task's runtime handle — legal on the decode worker.
                    let source = ctrl.active_source.peek().clone();
                    let rt_handle = tokio::runtime::Handle::current();
                    Box::new(move || match decoder::open_file(&path) {
                        Ok(parts) => Ok(parts),
                        Err(e) => {
                            tracing::warn!(error = %e, "cached file failed to open; falling back to the server stream");
                            let info = rt_handle
                                .block_on(source.resolve_stream(&cached_item_id))
                                .map_err(|e| e.to_string())?;
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
                } else {
                    let (stream_ref, _) = remote_ref.expect("classified as remote above");
                    let (stream_url, yt_format, yt_user_agent) =
                        match ResolvedStreamRef::parse(&stream_ref) {
                            ResolvedStreamRef::Pending(item_id) => {
                                // The one genuinely per-source op: resolve the
                                // playable stream through the active source
                                // (a URL for Jellyfin/Subsonic, a deciphered
                                // stream for YT).
                                let source = ctrl.active_source.peek().clone();
                                match source.resolve_stream(item_id).await {
                                    Ok(info) => {
                                        ctrl.stamp_probed_stream_info(
                                            phys_idx,
                                            idx,
                                            info.duration_secs,
                                            info.bitrate,
                                        );
                                        (info.url, info.format, info.user_agent)
                                    }
                                    Err(e) => {
                                        tracing::error!(error = %e, "stream URL resolve failed");
                                        ctrl.fail_load(token, &e);
                                        return;
                                    }
                                }
                            }
                            ResolvedStreamRef::SoundCloudHls(_)
                            | ResolvedStreamRef::AppleMusicFmp4(_)
                            | ResolvedStreamRef::Direct(_) => (stream_ref, None, None),
                        };

                    // The factory runs on the decode worker (no runtime), so
                    // hand StreamBuffer's download a handle from this task.
                    let rt_handle = tokio::runtime::Handle::current();
                    network_factory(
                        stream_url,
                        yt_format,
                        yt_user_agent,
                        is_radio_item,
                        icy_tx,
                        rt_handle,
                        buffer_progress,
                    )
                };

                let meta = NowPlayingMeta {
                    title: track.title.clone(),
                    artist: track.artist.clone(),
                    album: track.album.clone(),
                    duration: std::time::Duration::from_secs(track.duration),
                    artwork,
                };
                let transition = if use_crossfade {
                    Transition::Crossfade(crossfade_duration)
                } else {
                    Transition::Immediate
                };
                let start_at = restore_seek_secs
                    .filter(|secs| *secs > 0)
                    .map(Duration::from_secs);
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                ctrl.player.write().load(LoadArgs {
                    token,
                    factory,
                    meta,
                    transition,
                    start_at,
                    reply: Some(reply_tx),
                });

                match reply_rx.await {
                    Ok(Ok(outcome)) => {
                        ctrl.set_intent(PlaybackIntent::Committed { token });
                        if clear_pending_resume_on_success {
                            ctrl.pending_resume.set(None);
                        }
                        if use_crossfade {
                            if outcome.crossfaded {
                                // Defer the UI until TrackSwitched confirms the fade.
                                ctrl.schedule_pending_crossfade_ui(idx, token, from_token);
                            } else {
                                // Crossfade fell back to an immediate switch —
                                // commit now; no fade midpoint is coming.
                                ctrl.hydrate_current_track_metadata(idx, 0);
                            }
                        }

                        if is_radio_item {
                            ctrl.start_radio_metadata(station_id, stream_id, icy_rx);
                        } else {
                            let (item_id, source, options) = if is_server_item {
                                (
                                    Some(station_id.clone()),
                                    Some(ctrl.active_source),
                                    ScrobbleOptions::REMOTE_NATIVE,
                                )
                            } else {
                                (None, None, ScrobbleOptions::LOCAL)
                            };
                            scrobble_scheduler::schedule(
                                track.clone(),
                                item_id,
                                ctrl.config,
                                ctrl.current_token,
                                token,
                                ctrl.is_playing,
                                source,
                                options,
                                ctrl.db.peek().clone(),
                            );

                            if is_server_item && !cover_url.is_empty() {
                                ctrl.spawn_server_artwork_fetch(
                                    cover_url.clone(),
                                    track.clone(),
                                    token,
                                );
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::error!(error = %e, "playback failed");
                        ctrl.fail_load(token, &e);
                    }
                    Err(_) => {
                        // Cancelled engine-side (superseded or stopped) — the
                        // token no longer matches, so any stray write-back is
                        // ignored; whichever flow cancelled owns the intent.
                    }
                }
            }
            .instrument(tracing::info_span!("player.load_pipeline", idx)),
        );

        self.load_task.set(Some(task));
    }
}
