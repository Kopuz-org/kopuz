//! BotGuard mint runtime: a dedicated thread owning a
//! [`rustypipe_botguard::Botguard`] (a `deno_core` V8 isolate + jsdom shim),
//! reachable only via [`super::MintRequest`] since V8 is `!Send`.
//!
//! Uses the crate over a hand-rolled runtime because BotGuard fingerprints a
//! browser: bare `deno_core` with hand-written globals mints a degraded
//! snapshot that never emits the mint signal. Kept warm and re-initialized near
//! the integrity token's expiry; a snapshot file makes (re-)init fast.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustypipe_botguard::Botguard;
use tokio::sync::mpsc;

use super::MintRequest;

/// A desktop-Chrome UA for the BotGuard WAA requests.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// A warm minter and when to refresh its integrity token (80% of TTL).
struct Warm {
    bg: Botguard,
    refresh_at: Instant,
}

/// Cache the negotiated V8 snapshot so cold start / re-init is fast.
fn snapshot_path() -> Option<PathBuf> {
    let dir = directories::ProjectDirs::from("com", "temidaradev", "kopuz")?
        .cache_dir()
        .to_path_buf();
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("botguard_snapshot.bin"))
}

async fn init() -> Result<Warm, String> {
    let snap = snapshot_path();
    let bg = Botguard::builder()
        .user_agent(UA)
        .snapshot_path_opt(snap.as_deref())
        .init()
        .await
        .map_err(|e| e.to_string())?;
    // lifetime() is the integrity token's TTL in seconds; refresh at 80%.
    let ttl = bg.lifetime() as u64;
    let refresh_at = Instant::now() + Duration::from_secs(ttl.saturating_mul(4) / 5);
    Ok(Warm { bg, refresh_at })
}

/// Thread entrypoint: serve mint requests from one warm minter, re-initializing
/// when the token nears expiry.
pub fn run(mut rx: mpsc::UnboundedReceiver<MintRequest>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "botguard: tokio runtime build failed");
            return;
        }
    };

    rt.block_on(async move {
        let mut warm: Option<Warm> = None;

        while let Some(req) = rx.recv().await {
            // (Re)negotiate the integrity token on first use or near expiry.
            let needs_init = warm.as_ref().is_none_or(|w| Instant::now() >= w.refresh_at);
            if needs_init {
                match init().await {
                    Ok(w) => {
                        tracing::info!(
                            from_snapshot = w.bg.is_from_snapshot(),
                            "botguard: integrity token negotiated"
                        );
                        warm = Some(w);
                    }
                    Err(e) => {
                        let _ = req.reply.send(Err(format!("BotGuard init failed: {e}")));
                        continue;
                    }
                }
            }

            let w = warm.as_mut().expect("warm set above");
            let result =
                w.bg.mint_token(&req.video_id)
                    .await
                    .map_err(|e| format!("mint: {e}"));
            // A mint failure often means the token went stale early — drop the
            // instance so the next request re-negotiates.
            if result.is_err() {
                warm = None;
            }
            let _ = req.reply.send(result);
        }
    });
}
