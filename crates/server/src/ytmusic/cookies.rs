//! Native browser cookie reader for Chromium-family browsers on Linux.
//!
//! Mirrors what `yt-dlp --cookies-from-browser` does internally, but talks to
//! `secret-tool` (libsecret) for the OSCrypt v11 key — Hyprland trips up
//! yt-dlp's keyring auto-detection which is why we can't just shell out.

use std::path::{Path, PathBuf};
use std::process::Command;

use aes::Aes128;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use config::Browser;
use hmac::Hmac;
use rusqlite::{Connection, OpenFlags};
use sha1::Sha1;

type Aes128CbcDec = cbc::Decryptor<Aes128>;

/// Per-browser layout knowledge: where its profile lives under `$HOME`
/// and what application name libsecret stores its OSCrypt key under.
fn profile_subdir(b: Browser) -> &'static str {
    match b {
        Browser::Chrome => ".config/google-chrome",
        Browser::Chromium => ".config/chromium",
        Browser::Brave => ".config/BraveSoftware/Brave-Browser",
        Browser::Edge => ".config/microsoft-edge",
        Browser::Vivaldi => ".config/vivaldi",
    }
}

fn secret_app(b: Browser) -> &'static str {
    match b {
        Browser::Chrome => "chrome",
        Browser::Chromium => "chromium",
        Browser::Brave => "brave",
        Browser::Edge => "microsoft-edge",
        Browser::Vivaldi => "vivaldi",
    }
}

/// Browser candidates that have both a profile directory AND a libsecret
/// OSCrypt key — i.e. browsers the user has actually used at least once.
/// Caller still has to verify each one has YT auth cookies (`find_signed_in`).
pub fn candidate_browsers() -> Vec<Browser> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let home = PathBuf::from(home);
    Browser::ALL
        .iter()
        .copied()
        .filter(|b| home.join(profile_subdir(*b)).is_dir() && secret_tool_has(secret_app(*b)))
        .collect()
}

/// Tries each candidate browser in order, returning the first one whose
/// profile actually contains YouTube auth cookies.
pub async fn find_signed_in() -> Result<(Browser, String), String> {
    let candidates = candidate_browsers();
    if candidates.is_empty() {
        return Err(
            "no Chromium-family browser detected on this system".to_string(),
        );
    }
    let mut last_err: Option<String> = None;
    let mut tried: Vec<Browser> = Vec::new();
    for browser in candidates {
        tried.push(browser);
        match extract(browser).await {
            Ok(cookies) => return Ok((browser, cookies)),
            Err(e) => {
                eprintln!("[yt-cookies] {}: {e}", browser.id());
                last_err = Some(e);
            }
        }
    }
    let tried_labels: Vec<&'static str> = tried.iter().map(|b| b.label()).collect();
    Err(format!(
        "no signed-in YouTube Music browser among {tried_labels:?} — sign in to YouTube Music in one of them, then retry. last error: {}",
        last_err.as_deref().unwrap_or("(none)")
    ))
}

fn secret_tool_has(application: &str) -> bool {
    Command::new("secret-tool")
        .args(["lookup", "application", application])
        .output()
        .ok()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Extracts and decrypts cookies for `.youtube.com` from the browser's
/// default profile. Returns a single `Cookie:` header string.
pub async fn extract(browser: Browser) -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let profile_root = PathBuf::from(home).join(profile_subdir(browser));
    extract_from(browser, &profile_root).await
}

/// Like [`extract`] but with an explicit profile root — used by the
/// keepalive loop which runs against an isolated profile we own.
pub async fn extract_from(browser: Browser, profile_root: &Path) -> Result<String, String> {
    let db_path = pick_cookies_path(profile_root).ok_or_else(|| {
        format!(
            "no Cookies database under {} — is `{}` installed?",
            profile_root.display(),
            browser.label()
        )
    })?;

    // Snapshot to a temp file so we don't fight the running browser for the
    // write lock on its SQLite file.
    let snapshot = std::env::temp_dir().join(format!(
        "kopuz-yt-cookies-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    tokio::fs::copy(&db_path, &snapshot)
        .await
        .map_err(|e| format!("snapshot cookies db: {e}"))?;
    let snapshot_for_cleanup = snapshot.clone();

    let master_key = secret_tool_lookup(secret_app(browser))?.trim().to_string();
    let v10_key = derive_key(b"peanuts");
    let v11_key = derive_key(master_key.as_bytes());

    let header = tokio::task::spawn_blocking(move || {
        let r = read_and_decrypt(&snapshot, &v10_key, &v11_key);
        let _ = std::fs::remove_file(&snapshot);
        r
    })
    .await
    .map_err(|e| format!("decrypt task: {e}"))?;
    let _ = tokio::fs::remove_file(&snapshot_for_cleanup).await;

    let header = header?;
    if !header.contains("SAPISID=") && !header.contains("__Secure-3PAPISID=") {
        return Err(format!(
            "no auth cookies found in {} profile — sign in to YouTube Music there first",
            browser.label()
        ));
    }
    Ok(header)
}

fn pick_cookies_path(profile_root: &Path) -> Option<PathBuf> {
    let candidates = [
        profile_root.join("Default").join("Network").join("Cookies"),
        profile_root.join("Default").join("Cookies"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn secret_tool_lookup(application: &str) -> Result<String, String> {
    let output = Command::new("secret-tool")
        .args(["lookup", "application", application])
        .output()
        .map_err(|e| format!("secret-tool spawn: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "secret-tool exit {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let s = String::from_utf8(output.stdout)
        .map_err(|e| format!("secret-tool stdout utf-8: {e}"))?;
    if s.trim().is_empty() {
        return Err(format!(
            "secret-tool found no `{application}` entry — open the browser at least once"
        ));
    }
    Ok(s)
}

fn derive_key(password: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2::<Hmac<Sha1>>(password, b"saltysalt", 1, &mut key)
        .expect("pbkdf2 derive");
    key
}

fn decrypt_chromium(encrypted: &[u8], key: &[u8; 16]) -> Option<Vec<u8>> {
    if encrypted.len() <= 3 {
        return None;
    }
    let body = &encrypted[3..];
    if body.is_empty() || body.len() % 16 != 0 {
        return None;
    }
    let iv = [0x20u8; 16];
    let mut buf = body.to_vec();
    let pt = Aes128CbcDec::new(key.into(), &iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .ok()?
        .to_vec();
    // Chrome 130+ prefixes ciphertext plaintext with a 32-byte SHA-256 hash
    // of the host_key. Strip when the head looks binary and the tail is
    // valid UTF-8.
    if pt.len() > 32 {
        let head_binary = pt[..32].iter().any(|&b| !(0x20..=0x7e).contains(&b));
        let tail_ok = std::str::from_utf8(&pt[32..]).is_ok();
        if head_binary && tail_ok {
            return Some(pt[32..].to_vec());
        }
    }
    Some(pt)
}

fn read_and_decrypt(
    db_path: &Path,
    v10_key: &[u8; 16],
    v11_key: &[u8; 16],
) -> Result<String, String> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open db: {e}"))?;
    let mut stmt = conn
        .prepare("SELECT name, encrypted_value, value, host_key FROM cookies")
        .map_err(|e| format!("prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| format!("query: {e}"))?;
    let mut parts = Vec::new();
    for row in rows {
        let (name, encrypted, plain, host) = row.map_err(|e| format!("row: {e}"))?;
        // ONLY .youtube.com — the .google.com siblings have different
        // SID/3PSID values for the same name and confuse YT's auth check.
        let host_ok = host == "youtube.com"
            || host == ".youtube.com"
            || host.ends_with(".youtube.com");
        if !host_ok {
            continue;
        }
        let bytes = if encrypted.starts_with(b"v10") {
            decrypt_chromium(&encrypted, v10_key)
        } else if encrypted.starts_with(b"v11") {
            decrypt_chromium(&encrypted, v11_key)
        } else if !plain.is_empty() {
            Some(plain.into_bytes())
        } else {
            None
        };
        let Some(b) = bytes else { continue };
        let Ok(s) = String::from_utf8(b) else { continue };
        parts.push(format!("{name}={s}"));
    }
    Ok(parts.join("; "))
}
