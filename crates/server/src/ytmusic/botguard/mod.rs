//! Content PoToken minting for anonymous YouTube streaming.
//!
//! Anonymous googlevideo URLs 403 on deep/seek ranges without a content-bound
//! PO token (premium sessions are exempt — see `player::resolve`). The token is
//! minted by [`runtime`], a dedicated thread that runs YouTube's BotGuard VM
//! (the vendored `bgutils.js`) on an in-process `deno_core` V8 runtime. This
//! module is the typed channel to it plus the lazy bootstrap.
//!
//! The minter starts itself on the first [`mint_content_pot`] call, so there is
//! no UI wiring — any code path that needs a pot just asks for one. On Android
//! (no V8) the minter is absent and callers get the "not running" error, the
//! same anonymous-degraded behaviour as before the native minter existed.

use std::sync::OnceLock;

use tokio::sync::{mpsc, oneshot};

#[cfg(not(target_os = "android"))]
mod runtime;

/// One mint job: a `video_id` to bind the content pot to, and a one-shot for
/// the result (the base64url pot, or an error string).
pub struct MintRequest {
    pub video_id: String,
    pub reply: oneshot::Sender<Result<String, String>>,
}

static MINTER: OnceLock<mpsc::UnboundedSender<MintRequest>> = OnceLock::new();

/// Register the minter channel. Called once by [`ensure_started`] when the
/// runtime thread is spawned. A second call is ignored (returns the sender
/// back).
pub fn set_minter(
    tx: mpsc::UnboundedSender<MintRequest>,
) -> Result<(), mpsc::UnboundedSender<MintRequest>> {
    MINTER.set(tx)
}

/// True once the minter is registered.
pub fn is_available() -> bool {
    MINTER.get().is_some()
}

/// Spawn the BotGuard runtime thread exactly once, registering its channel. The
/// channel sender is set *before* the thread starts its (slow) V8 boot, so a
/// caller that races in can enqueue immediately and the reply lands once the
/// runtime is warm. No-op on Android (no V8).
#[cfg(not(target_os = "android"))]
pub fn ensure_started() {
    use std::sync::Once;
    static START: Once = Once::new();
    START.call_once(|| {
        let (tx, rx) = mpsc::unbounded_channel::<MintRequest>();
        // Register synchronously so `mint_content_pot` sends succeed the instant
        // this returns; the thread drains `rx` once V8 is up.
        let _ = set_minter(tx);
        if let Err(e) = std::thread::Builder::new()
            .name("botguard".into())
            .spawn(move || runtime::run(rx))
        {
            tracing::error!(error = %e, "failed to spawn BotGuard runtime thread");
        }
    });
}

#[cfg(target_os = "android")]
pub fn ensure_started() {}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    // Live: boots V8, runs the BotGuard VM, hits jnn-pa. Run explicitly with
    // `cargo test -p kopuz-server mints -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "hits live YouTube BotGuard"]
    async fn mints_a_content_pot() {
        let pot = super::mint_content_pot("dQw4w9WgXcQ")
            .await
            .expect("mint should succeed");
        assert!(!pot.is_empty(), "pot must be non-empty");
        eprintln!(
            "minted pot: len={} head={}",
            pot.len(),
            &pot[..pot.len().min(24)]
        );
        // A second mint within TTL should reuse the cached WebPoMinter.
        let pot2 = super::mint_content_pot("9bZkp7q19f0")
            .await
            .expect("second mint should succeed");
        assert!(!pot2.is_empty());
    }
}

/// Mint a content-bound PO token for `video_id`. Boots the runtime on first use.
/// Sub-ms in steady state: the runtime negotiates the BotGuard integrity token
/// once (refreshed near its TTL) and mints each content pot from it locally.
/// Errors if the minter isn't available (Android) or the runtime failed.
#[tracing::instrument(name = "yt.mint_pot", fields(video_id = %video_id))]
pub async fn mint_content_pot(video_id: &str) -> Result<String, String> {
    ensure_started();
    let tx = MINTER
        .get()
        .ok_or_else(|| "PO token minter unavailable on this platform".to_string())?;
    let (reply, rx) = oneshot::channel();
    tx.send(MintRequest {
        video_id: video_id.to_string(),
        reply,
    })
    .map_err(|_| "PO token minter channel closed".to_string())?;
    // Bound the wait: the very first mint pays the V8 boot + integrity-token
    // negotiation (a few seconds); a hung runtime must not hang the caller.
    match tokio::time::timeout(std::time::Duration::from_secs(20), rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("PO token minter dropped the reply".to_string()),
        Err(_) => Err("PO token mint timed out".to_string()),
    }
}
