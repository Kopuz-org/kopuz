//! Content PoToken minting for anonymous YouTube streaming.
//!
//! Anon googlevideo URLs 403 on deep/seek ranges without a content-bound PO
//! token (Premium sessions are exempt). Minted headlessly by [`runtime`]; this
//! module is the typed channel + lazy bootstrap, self-starting on the first
//! [`mint_content_pot`]. Absent on Android (no V8) — callers get the same
//! anonymous-degraded path as before.

use std::sync::OnceLock;

use tokio::sync::{mpsc, oneshot};

#[cfg(not(target_os = "android"))]
mod runtime;

/// One mint job: the `video_id` to bind the pot to, and a one-shot for the result.
pub struct MintRequest {
    pub video_id: String,
    pub reply: oneshot::Sender<Result<String, String>>,
}

static MINTER: OnceLock<mpsc::UnboundedSender<MintRequest>> = OnceLock::new();

/// Register the minter channel. A second call is ignored (returns the sender back).
pub fn set_minter(
    tx: mpsc::UnboundedSender<MintRequest>,
) -> Result<(), mpsc::UnboundedSender<MintRequest>> {
    MINTER.set(tx)
}

/// True once the minter is registered.
pub fn is_available() -> bool {
    MINTER.get().is_some()
}

/// Spawn the BotGuard runtime thread once. The channel is registered before the
/// (slow) V8 boot so racing callers can enqueue immediately. No-op on Android.
#[cfg(not(target_os = "android"))]
pub fn ensure_started() {
    use std::sync::Once;
    static START: Once = Once::new();
    START.call_once(|| {
        // Before spawning, so it can't race the decipher isolate (see
        // `ytmusic::ensure_v8_platform`).
        crate::ytmusic::ensure_v8_platform();
        let (tx, rx) = mpsc::unbounded_channel::<MintRequest>();
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
        // A second mint within TTL should reuse the cached WebPoMinter.
        let pot2 = super::mint_content_pot("9bZkp7q19f0")
            .await
            .expect("second mint should succeed");
        assert!(!pot2.is_empty());
    }
}

/// Mint a content-bound PO token for `video_id`, booting the runtime on first
/// use. Errors if the minter is unavailable (Android) or the runtime failed.
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
    // The first mint pays V8 boot + token negotiation; a hung runtime must not
    // hang the caller.
    match tokio::time::timeout(std::time::Duration::from_secs(20), rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("PO token minter dropped the reply".to_string()),
        Err(_) => Err("PO token mint timed out".to_string()),
    }
}
