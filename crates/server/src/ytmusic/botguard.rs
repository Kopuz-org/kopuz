//! Content PoToken minting for anonymous YouTube streaming.
//!
//! Anonymous googlevideo URLs 403 on deep/seek ranges without a content-bound
//! PO token (premium sessions are exempt — see `player::resolve`). The token is
//! minted by an in-app WebView running YouTube's BotGuard VM (BgUtils on the
//! music.youtube.com origin); this module is just the typed channel to it.
//!
//! The UI layer stands up that minter WebView when an anonymous YouTube Music
//! server becomes active and registers its sender via [`set_minter`]. Replaces
//! the old `rustypipe-botguard` subprocess — no external binary, flatpak-safe.

use std::sync::OnceLock;

use tokio::sync::{mpsc, oneshot};

/// One mint job: a `video_id` to bind the content pot to, and a one-shot for
/// the result (the base64url pot, or an error string).
pub struct MintRequest {
    pub video_id: String,
    pub reply: oneshot::Sender<Result<String, String>>,
}

static MINTER: OnceLock<mpsc::UnboundedSender<MintRequest>> = OnceLock::new();

/// Register the minter channel. Called by the UI once the anon YT Music minter
/// WebView is up. Idempotent-ish: a second call is ignored (returns the sender
/// back) so re-selecting the server doesn't panic.
pub fn set_minter(
    tx: mpsc::UnboundedSender<MintRequest>,
) -> Result<(), mpsc::UnboundedSender<MintRequest>> {
    MINTER.set(tx)
}

/// True once a minter has registered (UI uses this to decide whether anon
/// playback is wired up yet).
pub fn is_available() -> bool {
    MINTER.get().is_some()
}

/// Mint a content-bound PO token for `video_id`. Sub-ms in steady state: the
/// WebView negotiates the BotGuard integrity token once (pre-warmed at startup,
/// refreshed near its TTL) and mints each content pot from it locally. Errors if
/// no minter is registered (anon YT Music not selected) or the WebView failed.
pub async fn mint_content_pot(video_id: &str) -> Result<String, String> {
    let tx = MINTER
        .get()
        .ok_or_else(|| "PO token minter not running — select a YouTube Music server".to_string())?;
    let (reply, rx) = oneshot::channel();
    tx.send(MintRequest {
        video_id: video_id.to_string(),
        reply,
    })
    .map_err(|_| "PO token minter channel closed".to_string())?;
    // Bound the wait: if the webview bridge isn't ready (page still loading /
    // navigating, `window.__kopuzMint` not yet defined) the dispatch is a no-op
    // and no reply ever comes — without this the caller would hang forever.
    match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("PO token minter dropped the reply".to_string()),
        Err(_) => Err("PO token mint timed out (webview not ready)".to_string()),
    }
}
