//! Blocking source factories used by the session load pipeline.

use player::decoder;
use player::engine::SourceFactory;
use server::playback_ref::ResolvedStreamRef;
use server::stream::stream_buffer::{Captured, CompleteSink};

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
    icy_tx: Option<tokio::sync::watch::Sender<server::stream::icy::IcyMeta>>,
    rt_handle: tokio::runtime::Handle,
    buffer_progress: Option<server::stream::stream_buffer::BufferProgressCallback>,
    complete: Option<CompleteSink>,
) -> SourceFactory {
    Box::new(move || {
        let mut complete = complete;
        let build = || -> std::io::Result<_> {
            if is_radio {
                let stream = server::stream::stream_buffer::StreamBuffer::new(
                    stream_url,
                    true,
                    server::stream::stream_buffer::radio_client(yt_user_agent),
                    icy_tx,
                    rt_handle,
                );
                stream.wait_for_response_headers();
                if let Some(error) = stream.response_error() {
                    return Err(error);
                }
                Ok(decoder::from_stream_with_hint(stream, "ogg"))
            } else if let Some((fmt, range_safe)) = yt_format {
                // YT: HTTP Range-backed source. Symphonia can seek freely
                // (Matroska Cues at the end, scrub anywhere). A URL the
                // resolver called range-safe can still have its deep ranges
                // refused (issue #731), so the tail is checked first.
                let open_ranged = || {
                    let range = server::stream::range_source::RangeStreamSource::new_with_progress(
                        stream_url.clone(),
                        yt_user_agent.clone(),
                        buffer_progress.clone(),
                    )?;
                    range.check_tail()?;
                    Ok::<_, std::io::Error>(range)
                };
                let range = match range_safe {
                    false => None,
                    true => match open_ranged() {
                        Ok(range) => Some(range),
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "no range support; streaming sequentially without seeking"
                            );
                            None
                        }
                    },
                };
                if let Some(mut range) = range {
                    if let Some(sink) = complete.take() {
                        range.capture(sink);
                    }
                    let len = Some(range.total_size());
                    let (source, mut hint) = decoder::from_stream_with_len(range, len);
                    hint.with_extension(fmt.extension());
                    Ok((source, hint))
                } else {
                    // No-pot fallback: googlevideo 403s deep ranges, and the
                    // probe reads the webm tail — stream sequentially instead
                    // of failing outright (issue #386). No scrubbing.
                    let stream = server::stream::stream_buffer::StreamBuffer::capturing(
                        stream_url,
                        server::stream::stream_buffer::stream_client(yt_user_agent),
                        rt_handle,
                        buffer_progress,
                        complete.take(),
                    );
                    stream.wait_for_response_headers();
                    if let Some(error) = stream.response_error() {
                        return Err(error);
                    }
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
                let bytes =
                    server::stream::hls_source::assemble(hls_url, yt_user_agent.as_deref())?;
                if let Some(sink) = complete.take() {
                    sink(Captured {
                        bytes: bytes.clone(),
                        content_type: Some("audio/mp4".into()),
                    });
                }
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
                let sink = complete.take();
                let track = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    rt_handle.block_on(server::applemusic::stream::resolve_and_decrypt(
                        &adam_id,
                        &token,
                        &storefront,
                        &language,
                        buffer_progress.clone(),
                        sink,
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
                match server::stream::range_source::RangeStreamSource::new_with_progress(
                    stream_url.clone(),
                    yt_user_agent.clone(),
                    buffer_progress.clone(),
                ) {
                    Ok(mut range) => {
                        if let Some(sink) = complete.take() {
                            range.capture(sink);
                        }
                        let len = Some(range.total_size());
                        Ok(decoder::from_stream_with_len(range, len))
                    }
                    Err(error) => {
                        tracing::debug!(%error, "HTTP ranges unavailable; using progressive stream");
                        let stream = server::stream::stream_buffer::StreamBuffer::capturing(
                            stream_url,
                            server::stream::stream_buffer::stream_client(yt_user_agent),
                            rt_handle,
                            buffer_progress,
                            complete.take(),
                        );
                        stream.wait_for_response_headers();
                        if let Some(error) = stream.response_error() {
                            return Err(error);
                        }
                        let len = stream.known_total_size();
                        Ok(decoder::from_stream_with_len(stream, len))
                    }
                }
            }
        };
        build()
    })
}
