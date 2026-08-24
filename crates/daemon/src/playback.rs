//! Blocking source factories used by the session load pipeline.

use player::decoder;
use player::engine::SourceFactory;
use utils::playback_ref::ResolvedStreamRef;

/// Factory for a resolved network stream (radio, YT range/sequential,
/// SoundCloud HLS, Apple Music fMP4, or a plain buffered stream).
///
/// The returned closure runs on the decode worker thread, which has no tokio
/// runtime. Keep the captured runtime handle and its `block_on` calls here;
/// the blocking decode I/O is intentional.
pub(crate) fn network_factory(
    stream_url: String,
    yt_format: Option<(server::ytmusic::player::AudioFormat, bool)>,
    yt_user_agent: Option<String>,
    is_radio: bool,
    icy_tx: Option<tokio::sync::watch::Sender<utils::icy::IcyMeta>>,
    rt_handle: tokio::runtime::Handle,
    buffer_progress: Option<utils::stream_buffer::BufferProgressCallback>,
) -> SourceFactory {
    Box::new(move || {
        let build = || -> std::io::Result<_> {
            if is_radio {
                let stream = utils::stream_buffer::StreamBuffer::with_user_agent(
                    stream_url,
                    true,
                    yt_user_agent,
                    icy_tx,
                    rt_handle,
                );
                Ok(decoder::from_stream_with_hint(stream, "ogg"))
            } else if let Some((fmt, range_safe)) = yt_format {
                if range_safe {
                    // YT: HTTP Range-backed source. Symphonia can seek freely
                    // (Matroska Cues at the end, scrub anywhere) and startup
                    // probes only fetch the ~512 KiB they need.
                    let range = utils::range_source::RangeStreamSource::new_with_progress(
                        stream_url,
                        yt_user_agent,
                        buffer_progress,
                    )?;
                    let len = Some(range.total_size());
                    let (source, mut hint) = decoder::from_stream_with_len(range, len);
                    hint.with_extension(fmt.extension());
                    Ok((source, hint))
                } else {
                    // No-pot fallback: googlevideo 403s deep ranges, and the
                    // probe reads the webm tail — stream sequentially instead
                    // of failing outright (issue #386). No scrubbing.
                    let stream = utils::stream_buffer::StreamBuffer::with_user_agent_and_progress(
                        stream_url,
                        false,
                        yt_user_agent,
                        None,
                        rt_handle,
                        buffer_progress,
                    );
                    stream.wait_for_response_headers();
                    let len = stream.known_total_size();
                    let (source, mut hint) = decoder::from_stream_with_len(stream, len);
                    hint.with_extension(fmt.extension());
                    Ok((source, hint))
                }
            } else if let ResolvedStreamRef::SoundCloudHls(hls_url) =
                ResolvedStreamRef::parse(&stream_url)
            {
                // SoundCloud Go+ AAC: assemble the HLS playlist's fMP4 segments
                // into one in-memory buffer Symphonia can decode.
                let bytes = utils::hls_source::assemble(hls_url, yt_user_agent.as_deref())?;
                let len = Some(bytes.len() as u64);
                let cursor = std::io::Cursor::new(bytes);
                let (source, mut hint) = decoder::from_stream_with_len(cursor, len);
                hint.with_extension("m4a");
                Ok((source, hint))
            } else if let ResolvedStreamRef::AppleMusicFmp4(payload) =
                ResolvedStreamRef::parse(&stream_url)
            {
                // The key exchange has panicked on malformed CDM responses, so
                // contain it rather than taking the decode worker down.
                let (adam_id, storefront, language, token_b64) =
                    ResolvedStreamRef::apple_music_parts(payload).ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "malformed Apple Music stream ref",
                        )
                    })?;
                let token =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, token_b64)
                        .map_err(|error| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("malformed Apple Music token: {error}"),
                            )
                        })
                        .and_then(|bytes| {
                            String::from_utf8(bytes).map_err(|error| {
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    format!("malformed Apple Music token: {error}"),
                                )
                            })
                        })?;
                let (adam_id, storefront, language) = (
                    adam_id.to_string(),
                    storefront.to_string(),
                    language.to_string(),
                );
                let track = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    rt_handle.block_on(server::applemusic::stream::resolve_and_decrypt(
                        &adam_id,
                        &token,
                        &storefront,
                        &language,
                        buffer_progress.clone(),
                    ))
                }))
                .unwrap_or_else(|panic| {
                    let msg = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    tracing::error!("am.playback: resolve_and_decrypt panicked: {msg}");
                    Err(format!("Apple Music decrypt panicked: {msg}"))
                })
                .map_err(std::io::Error::other)?;
                // Samples decrypt on demand, so the source remains seekable.
                let len = Some(track.total_size());
                let (source, mut hint) = decoder::from_stream_with_len(track, len);
                hint.with_extension("m4a");
                Ok((source, hint))
            } else {
                // Jellyfin and Subsonic normally support ranges. Fall back to
                // a progressive stream when the endpoint does not.
                match utils::range_source::RangeStreamSource::new_with_progress(
                    stream_url.clone(),
                    yt_user_agent.clone(),
                    buffer_progress.clone(),
                ) {
                    Ok(range) => {
                        let len = Some(range.total_size());
                        Ok(decoder::from_stream_with_len(range, len))
                    }
                    Err(error) => {
                        tracing::debug!(%error, "HTTP ranges unavailable; using progressive stream");
                        let stream =
                            utils::stream_buffer::StreamBuffer::with_user_agent_and_progress(
                                stream_url,
                                false,
                                yt_user_agent,
                                None,
                                rt_handle,
                                buffer_progress,
                            );
                        stream.wait_for_response_headers();
                        let len = stream.known_total_size();
                        Ok(decoder::from_stream_with_len(stream, len))
                    }
                }
            }
        };
        build().map_err(|e| e.to_string())
    })
}
