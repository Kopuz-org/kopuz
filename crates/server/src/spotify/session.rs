//! Process-global librespot [`Session`] holder + a dedicated multi-thread tokio
//! runtime to drive it.
//!
//! Why a dedicated runtime: librespot's `Session` spawns long-lived background
//! tasks (the access-point connection, mercury, the audio-key manager) onto the
//! runtime it is connected from, and its blocking audio reader (`AudioFile`)
//! drives an async download by `block_on`-ing internally. Pinning all of that to
//! our own runtime [`rt`] keeps it off the Dioxus/app runtime and lets the
//! player's `spawn_blocking` decode thread pull bytes without a nested-runtime
//! panic — work is handed to [`rt`] and the result comes back over a channel
//! ([`on_rt`] for async callers, [`block_on_rt`] for the sync decode path).
//!
//! Rate limit: everything here speaks Spotify's internal access-point protocol
//! over this one persistent `Session` (the same channel the official client
//! uses), never the public Web API — so metadata/library hydration doesn't hit
//! the public `429` limits.

use std::future::Future;
use std::sync::OnceLock;

use librespot_core::{authentication::Credentials, config::SessionConfig, session::Session};
use tokio::sync::Mutex;

/// librespot's well-known "keymaster" desktop client id — the one its own OAuth
/// flow uses, so the access tokens we mint are valid for the session.
pub(crate) const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";

/// The dedicated runtime all librespot work runs on.
fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .thread_name("spotify-rt")
            .build()
            .expect("build spotify runtime")
    })
}

struct Entry {
    token: String,
    session: Session,
}

fn cache() -> &'static Mutex<Option<Entry>> {
    static C: OnceLock<Mutex<Option<Entry>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// Run `fut` on the spotify runtime and await its result from any other runtime
/// (the app/Dioxus runtime). Used by the async metadata fetchers.
pub async fn on_rt<F, T>(fut: F) -> Result<T, String>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt().spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.await.map_err(|_| "spotify task dropped".to_string())
}

/// Run `fut` on the spotify runtime and block the *calling* thread for the
/// result. Safe to call from a `spawn_blocking` thread (uses a std channel, so
/// it never `block_on`s within a runtime). Used by the audio decode path.
pub fn block_on_rt<F, T>(fut: F) -> Result<T, String>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    rt().spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.recv().map_err(|_| "spotify task dropped".to_string())
}

/// Get a live, connected `Session` for `access_token`, reusing the cached one
/// when the token is unchanged and the connection is still valid. MUST be
/// awaited inside [`on_rt`]/[`block_on_rt`] so it runs on the spotify runtime.
pub async fn ensure_session(access_token: &str) -> Result<Session, String> {
    let mut guard = cache().lock().await;
    if let Some(e) = guard.as_ref()
        && e.token == access_token
        && !e.session.is_invalid()
    {
        return Ok(e.session.clone());
    }

    let cfg = SessionConfig {
        client_id: CLIENT_ID.to_string(),
        ..SessionConfig::default()
    };
    let session = Session::new(cfg, None);
    tracing::info!("spotify: connecting session");
    session
        .connect(Credentials::with_access_token(access_token), false)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "spotify session connect failed");
            format!("spotify session connect failed: {e}")
        })?;
    tracing::info!(user = %session.username(), country = %session.country(), "spotify: session connected");

    *guard = Some(Entry {
        token: access_token.to_string(),
        session: session.clone(),
    });
    Ok(session)
}

/// Drop the cached session (on sign-out / token rotation).
pub async fn clear_session() {
    *cache().lock().await = None;
}
