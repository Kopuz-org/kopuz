//! Shared utility crate for Kopuz: color helpers, artwork URLs, lyrics fetching,
//! and terminal logging.

pub mod artist;
pub mod build_info;
pub mod color;
pub mod db_cache;
pub mod hls_source;
pub mod icy;
pub mod live_theme;
pub mod logs;
pub mod lyrics;
pub mod musicbrainz;
pub mod playlist;
pub mod range_source;
pub mod stream_buffer;
pub mod themes;
use std::path::Path;
use std::sync::Arc;

pub type CoverUrl = Arc<str>;

pub fn cover_url_from_string(url: String) -> CoverUrl {
    Arc::from(url)
}

pub fn map_cover_url(url: Option<String>) -> Option<CoverUrl> {
    url.map(cover_url_from_string)
}

/// Cross-platform async sleep backed by tokio.
pub async fn sleep(duration: std::time::Duration) {
    tokio::time::sleep(duration).await;
}

/// Run a future on tokio's worker pool instead of the calling thread.
///
/// Dioxus polls its tasks (`use_resource`, `spawn`) on the UI thread, so any
/// CPU spent inside them — sqlx row decoding, response JSON parsing — stalls
/// rendering for that long. Wrapping the future here moves the work to a
/// worker thread; the UI-side task only awaits the join handle. The `Send`
/// bound is the guardrail: a future that touches a `Signal` won't compile.
///
/// Dropping the returned future aborts the spawned task, so cancellation
/// passes through: when dioxus drops a superseded `use_resource` rerun, the
/// offloaded query stops instead of running to completion in the background —
/// the same semantics the un-offloaded future had.
///
/// Panics inside the future propagate to the caller unchanged.
pub async fn offload<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    let handle = tokio::spawn(fut);
    // Aborting an already-finished task is a no-op, so the guard can simply
    // live for the whole function — it only bites on mid-await drop.
    let _guard = AbortOnDrop(handle.abort_handle());
    match handle.await {
        Ok(out) => out,
        Err(err) => match err.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            // Unreachable via our own abort (the awaiter was dropped with the
            // guard); a cancellation seen here means runtime shutdown, where
            // the app is exiting anyway.
            Err(err) => panic!("offloaded task cancelled: {err}"),
        },
    }
}

pub fn format_artwork_url<P: AsRef<Path> + ?Sized>(path: Option<&P>) -> Option<CoverUrl> {
    let p = path?;
    let p = p.as_ref();
    let p_str = p.to_string_lossy();

    let abs_path = if let Some(stripped) = p_str.strip_prefix("./") {
        std::env::current_dir().unwrap_or_default().join(stripped)
    } else {
        p.to_path_buf()
    };

    let abs_str = abs_path.to_string_lossy();
    let abs_str = if abs_str.starts_with('~') {
        if let Ok(home) = std::env::var("HOME") {
            std::borrow::Cow::Owned(abs_str.replacen('~', &home, 1))
        } else {
            abs_str
        }
    } else {
        abs_str
    };

    // Android WebView is unreliable with custom URL schemes (artwork://) and the
    // http localhost shim, so inline the cover as a base64 data URL instead.
    #[cfg(target_os = "android")]
    {
        use base64::{Engine as _, engine::general_purpose};
        return match std::fs::read(&*abs_str) {
            Ok(bytes) => {
                let mime = if abs_str.ends_with(".png") {
                    "image/png"
                } else if abs_str.ends_with(".gif") {
                    "image/gif"
                } else if abs_str.ends_with(".webp") {
                    "image/webp"
                } else {
                    "image/jpeg"
                };
                let b64 = general_purpose::STANDARD.encode(&bytes);
                Some(cover_url_from_string(format!("data:{mime};base64,{b64}")))
            }
            Err(_) => None,
        };
    }

    const QUERY_VAL: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'&')
        .add(b'+')
        .add(b'=')
        .add(b'?')
        .add(b'<')
        .add(b'>')
        .add(b'`')
        .add(b'\\')
        .add(b':');

    // Version the URL because the WebView caches protocol responses for a year;
    // the token prevents an old full-resolution response from surviving a
    // change back to the thumbnail/HQ split.
    if cfg!(target_os = "windows") {
        let url = format!(
            "http://artwork.dioxus.localhost/local?p={}&v=thumb400-hq1920",
            percent_encoding::utf8_percent_encode(&abs_str, QUERY_VAL)
        );
        Some(cover_url_from_string(url))
    } else {
        let url = format!(
            "artwork://local?p={}&v=thumb400-hq1920",
            percent_encoding::utf8_percent_encode(&abs_str, QUERY_VAL)
        );
        Some(cover_url_from_string(url))
    }
}

pub const DEFAULT_COVER_SVG: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='400' height='400' viewBox='0 0 400 400'%3E%3Crect width='400' height='400' fill='%231e1b2e'/%3E%3Ccircle cx='200' cy='180' r='70' fill='none' stroke='%233d3466' stroke-width='6'/%3E%3Cpath d='M155 280 Q200 240 245 280' fill='none' stroke='%233d3466' stroke-width='6' stroke-linecap='round'/%3E%3C/svg%3E";

pub fn default_cover_url() -> CoverUrl {
    cover_url_from_string(DEFAULT_COVER_SVG.to_string())
}

#[cfg(test)]
mod artwork_url_tests {
    #[test]
    fn local_artwork_url_versions_the_webview_cache() {
        let url = super::format_artwork_url(Some(std::path::Path::new("/music/cover.jpg")))
            .expect("artwork URL");
        assert!(url.contains("&v=thumb400-hq1920"));
    }
}

#[cfg(test)]
mod offload_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Dropping the `offload` future must abort the spawned task — a
    /// superseded `use_resource` rerun may not leave its query running.
    #[tokio::test]
    async fn dropping_offload_aborts_the_task() {
        struct SetOnDrop(Arc<AtomicBool>);
        impl Drop for SetOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = SetOnDrop(dropped.clone());
        let fut = super::offload(async move {
            let _guard = guard;
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        });
        // Poll the offload future long enough to spawn, then drop it (select
        // drops the loser when the timer wins).
        tokio::select! {
            _ = fut => panic!("offloaded sleep cannot have completed"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }
        for _ in 0..100 {
            if dropped.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("offloaded task kept running after its caller was dropped");
    }
}
