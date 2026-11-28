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
/// What a resolved network stream is fetched as.
pub(crate) struct NetworkStream {
    pub(crate) url: String,
    pub(crate) format: Option<(server::stream::AudioFormat, bool)>,
    pub(crate) user_agent: Option<String>,
    pub(crate) guard: Option<utils::http::Guard>,
}

pub(crate) fn network_factory(
    stream: NetworkStream,
    is_radio: bool,
    icy_tx: Option<tokio::sync::watch::Sender<server::stream::icy::IcyMeta>>,
    rt_handle: tokio::runtime::Handle,
    buffer_progress: Option<server::stream::stream_buffer::BufferProgressCallback>,
    complete: Option<CompleteSink>,
) -> SourceFactory {
    let NetworkStream {
        url: stream_url,
        format: yt_format,
        user_agent: yt_user_agent,
        guard,
    } = stream;
    Box::new(move || {
        let mut complete = complete;
        let build = || -> std::io::Result<_> {
            if is_radio {
                let client = server::stream::stream_buffer::guarded_radio_client(
                    yt_user_agent,
                    guard.clone(),
                )?;
                let stream = server::stream::stream_buffer::StreamBuffer::radio(
                    stream_url, client, guard, icy_tx, rt_handle,
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
                        guard.clone(),
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
                        server::stream::stream_buffer::guarded_stream_client(
                            yt_user_agent,
                            guard.clone(),
                        )?,
                        guard.clone(),
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
                let bytes = server::stream::hls_source::assemble(
                    hls_url,
                    yt_user_agent.as_deref(),
                    guard.clone(),
                )?;
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

                let len = Some(track.total_size());
                let (source, mut hint) = decoder::from_stream_with_len(track, len);
                hint.with_extension("m4a");
                Ok((source, hint))
            } else {
                match server::stream::range_source::RangeStreamSource::new_with_progress(
                    stream_url.clone(),
                    yt_user_agent.clone(),
                    buffer_progress.clone(),
                    guard.clone(),
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
                            server::stream::stream_buffer::guarded_stream_client(
                                yt_user_agent,
                                guard.clone(),
                            )?,
                            guard.clone(),
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

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{NetworkStream, network_factory};

    /// Lets only 127.0.0.1, as a plugin's reach would its own host.
    struct LoopbackOnly;

    impl utils::http::Reach for LoopbackOnly {
        fn allows_host(&self, host: &str) -> bool {
            host == "127.0.0.1"
        }

        fn may_be_local(&self, host: &str) -> bool {
            host == "127.0.0.1"
        }
    }

    /// Answers every request on a fresh port with `reply`; counts them.
    fn serve(reply: String) -> (u16, Arc<AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else {
                    return;
                };
                counted.fetch_add(1, Ordering::SeqCst);
                let mut request = [0u8; 1024];
                let _ = socket.read(&mut request);
                let _ = socket.write_all(reply.as_bytes());
            }
        });
        (port, hits)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_guarded_station_does_not_follow_a_redirect_off_its_hosts() {
        let (elsewhere, elsewhere_hits) =
            serve("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_string());
        let (start, _) = serve(format!(
            "HTTP/1.1 302 Found\r\nLocation: http://localhost:{elsewhere}/live\r\nContent-Length: 0\r\n\r\n"
        ));
        let guard: utils::http::Guard = Arc::new(LoopbackOnly);
        let factory = network_factory(
            NetworkStream {
                url: format!("http://127.0.0.1:{start}/live"),
                format: None,
                user_agent: None,
                guard: Some(guard),
            },
            true,
            None,
            tokio::runtime::Handle::current(),
            None,
            None,
        );

        let opened = tokio::task::spawn_blocking(factory).await.expect("join");
        assert!(opened.is_err(), "the station opened off its hosts");
        assert_eq!(elsewhere_hits.load(Ordering::SeqCst), 0);
    }
}
