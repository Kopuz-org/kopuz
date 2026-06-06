//! Shell out to `rustypipe-botguard` for Proof-of-Origin tokens.
//!
//! YouTube's /player rejects requests from web-ish clients without a valid
//! content-bound PO token in `serviceIntegrityDimensions.poToken`. Minting
//! requires running YouTube's obfuscated Botguard VM in a real-ish JS
//! environment. We delegate to ThetaDev's `rustypipe-botguard` binary
//! (V8 + jsdom-minimal); it caches a V8 startup snapshot so warm mints are
//! ~20 ms, cold-start ~600 ms once per ~6 hours.

use std::path::PathBuf;

/// Locate the `rustypipe-botguard` binary. Search order:
///   1. `$RUSTYPIPE_BOTGUARD_BIN` env var (escape hatch)
///   2. Next to the running executable (release distribution shape):
///      `<exe_dir>/rustypipe-botguard` or `<exe_dir>/bin/rustypipe-botguard`
///   3. Walking up from cwd looking for `bin/rustypipe-botguard` (dev mode:
///      finds it whether you started Kopuz from the workspace root or from
///      a crate subdir).
///   4. Bare `rustypipe-botguard` (resolved via PATH).
fn binary_path() -> PathBuf {
    if let Some(p) = std::env::var_os("RUSTYPIPE_BOTGUARD_BIN") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let next_to_exe = dir.join("rustypipe-botguard");
        if next_to_exe.is_file() {
            return next_to_exe;
        }
        let in_bin = dir.join("bin").join("rustypipe-botguard");
        if in_bin.is_file() {
            return in_bin;
        }
    }
    if let Ok(mut p) = std::env::current_dir() {
        loop {
            let candidate = p.join("bin").join("rustypipe-botguard");
            if candidate.is_file() {
                return candidate;
            }
            if !p.pop() {
                break;
            }
        }
    }
    PathBuf::from("rustypipe-botguard")
}

/// Returns `Ok(())` if the `rustypipe-botguard` binary can be located and
/// runs (we exec it with `--help` so the check is sub-millisecond and
/// doesn't touch the network). Use this at server-selection / login time
/// so the user sees the install hint up front instead of discovering it
/// silently when their first track 403s mid-stream.
pub async fn check_available() -> Result<(), String> {
    let bin = binary_path();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        match std::process::Command::new(&bin)
            .arg("--help")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .status()
        {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!(
                "`{}` exited with {} on --help",
                bin.display(),
                s
            )),
            Err(_) => Err(
                "rustypipe-botguard not found — install with: cargo install rustypipe-botguard --version 0.1.2".to_string()
            ),
        }
    })
    .await
    .map_err(|e| format!("join check task: {e}"))?
}

/// Mint a content-bound PO token for `video_id`. Returns the base64url
/// token suitable for stuffing into `serviceIntegrityDimensions.poToken`.
pub async fn mint_content_pot(video_id: &str) -> Result<String, String> {
    let bin = binary_path();
    let video_id = video_id.to_string();
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let out = std::process::Command::new(&bin)
            .arg("--")
            .arg(&video_id)
            .output()
            .map_err(|e| {
                format!(
                    "spawn `{}`: {e} — install with `cargo install rustypipe-botguard --version 0.1.2`",
                    bin.display()
                )
            })?;
        if !out.status.success() {
            return Err(format!(
                "rustypipe-botguard exit {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .map(|s| s.to_string())
            .ok_or_else(|| "rustypipe-botguard returned no token".to_string())
    })
    .await
    .map_err(|e| format!("join botguard task: {e}"))?
}
