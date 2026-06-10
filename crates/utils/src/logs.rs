//! On-disk log file management: per-session rotation, retention, crash
//! reports, and the user-facing "export logs" bundle.
//!
//! Lives in `utils` (not `kopuz`) so the settings UI can trigger an export
//! without depending on the binary crate. The tracing *subscriber* still
//! lives in `kopuz::logging`; this module only owns the files it writes to.
//!
//! Layout under `<cache>/logs/`:
//!   - `latest.log`        — the current session (the appender writes here).
//!   - `kopuz-<ts>.log`    — previous sessions, archived on startup so a
//!                           restart never erases a crashing run. `<ts>` is
//!                           UTC `YYYY-MM-DD_HH-MM-SS`, which sorts
//!                           alphabetically == chronologically.
//!   - `crash-<ts>.txt`    — written only on a panic (message + backtrace +
//!                           recent log tail + version/OS).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

static LOG_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

const LATEST: &str = "latest.log";
const SESSION_PREFIX: &str = "kopuz-";
const SESSION_SUFFIX: &str = ".log";
const CRASH_PREFIX: &str = "crash-";
/// How many archived session logs to keep before pruning the oldest.
const KEEP_SESSIONS: usize = 10;
/// Bytes of `latest.log` to include as context in a crash report / export.
const TAIL_BYTES: u64 = 64 * 1024;

/// Record the active log directory so [`export_logs`] (called from the UI)
/// and [`write_crash_report`] (called from the panic hook) can find it.
pub fn set_log_dir(dir: PathBuf) {
    if let Ok(mut g) = LOG_DIR.lock() {
        *g = Some(dir);
    }
}

/// The active log directory, if logging has been initialized.
pub fn log_dir() -> Option<PathBuf> {
    LOG_DIR.lock().ok().and_then(|g| g.clone())
}

/// UTC `YYYY-MM-DD_HH-MM-SS` — filename-safe (no colons) and sorts
/// lexicographically in chronological order.
pub fn timestamp() -> String {
    format_time(SystemTime::now())
}

fn format_time(t: SystemTime) -> String {
    use time::OffsetDateTime;
    use time::macros::format_description;
    let fmt = format_description!("[year]-[month]-[day]_[hour]-[minute]-[second]");
    OffsetDateTime::from(t)
        .format(&fmt)
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Archive the previous session's `latest.log` to a timestamped file, then
/// prune old archives. Call this BEFORE the appender opens a fresh
/// `latest.log`, so the crashing session survives a restart instead of
/// being overwritten.
pub fn rotate_session_log(dir: &Path) {
    let latest = dir.join(LATEST);
    if latest.exists() {
        // Name by the previous file's last-modified time (when that session
        // ran) when available, falling back to now. Both sort chronologically.
        let ts = std::fs::metadata(&latest)
            .and_then(|m| m.modified())
            .map(format_time)
            .unwrap_or_else(|_| timestamp());
        let archive = dir.join(format!("{SESSION_PREFIX}{ts}{SESSION_SUFFIX}"));
        let _ = std::fs::rename(&latest, &archive);
    }
    prune_old_sessions(dir, KEEP_SESSIONS);
}

fn prune_old_sessions(dir: &Path, keep: usize) {
    let mut sessions: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(SESSION_PREFIX) && n.ends_with(SESSION_SUFFIX))
            })
            .collect(),
        Err(_) => return,
    };
    if sessions.len() <= keep {
        return;
    }
    sessions.sort(); // timestamped names sort oldest-first
    for old in &sessions[..sessions.len() - keep] {
        let _ = std::fs::remove_file(old);
    }
}

/// Read up to the last `TAIL_BYTES` of a file as a UTF-8 (lossy) string,
/// trimming a partial first line. Used so crash reports/exports carry recent
/// context without copying a potentially large debug log in full.
fn read_tail(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    // Read bytes and lossily decode: seeking to a fixed offset can land
    // mid-UTF-8, which would make read_to_string fail and drop the tail
    // entirely despite the "lossy" intent.
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let mut buf = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        // Drop the (likely partial) first line.
        if let Some(nl) = buf.find('\n') {
            buf.drain(..=nl);
        }
    }
    Some(buf)
}

/// Write a crash report from the panic hook. Returns the path written.
/// `version` is passed in by the caller because `env!("CARGO_PKG_VERSION")`
/// here would resolve to `utils`, not the app.
pub fn write_crash_report(
    dir: &Path,
    version: &str,
    message: &str,
    location: &str,
    backtrace: &str,
) -> Option<PathBuf> {
    let ts = timestamp();
    let path = dir.join(format!("{CRASH_PREFIX}{ts}.txt"));
    let mut f = std::fs::File::create(&path).ok()?;
    let _ = writeln!(f, "kopuz crash report — {ts} UTC");
    let _ = writeln!(f, "version: {version}");
    let _ = writeln!(f, "os: {} / {}", std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(f, "\npanic: {message}");
    let _ = writeln!(f, "location: {location}");
    let _ = writeln!(f, "\n--- backtrace ---\n{backtrace}");
    if let Some(tail) = read_tail(&dir.join(LATEST)) {
        let _ = writeln!(f, "\n--- recent log ({LATEST} tail) ---\n{tail}");
    }
    Some(path)
}

fn newest_crash(dir: &Path) -> Option<PathBuf> {
    let mut crashes: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(CRASH_PREFIX))
        })
        .collect();
    crashes.sort();
    crashes.pop()
}

/// Reveal the logs directory in the OS file manager so the user can grab
/// `latest.log`, the archived sessions, and any crash reports directly.
/// Fire-and-forget — we don't wait on the spawned file manager.
pub fn open_log_dir() -> io::Result<()> {
    let dir =
        log_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "log dir not set"))?;
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(&dir).spawn()?;
    Ok(())
}

/// Bundle the current session log and the most recent crash report (plus a
/// version/OS header) into a single file at `dest`, for the user to attach to
/// a bug report. Triggered by the settings "Export logs" button.
pub fn export_logs(dest: &Path) -> io::Result<()> {
    let dir =
        log_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "log dir not set"))?;

    // Read sources BEFORE creating (truncating) dest — the user could pick an
    // existing log file (even latest.log itself) as the destination, which
    // File::create would erase before we read it.
    let latest = std::fs::read_to_string(dir.join(LATEST));
    let crash = newest_crash(&dir).map(|p| {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("crash report")
            .to_string();
        (name, std::fs::read_to_string(&p))
    });

    let mut out = std::fs::File::create(dest)?;
    writeln!(out, "=== kopuz log export — {} UTC ===", timestamp())?;
    writeln!(out, "os: {} / {}", std::env::consts::OS, std::env::consts::ARCH)?;

    writeln!(out, "\n=== {LATEST} ===")?;
    match latest {
        Ok(s) => out.write_all(s.as_bytes())?,
        Err(e) => writeln!(out, "(unavailable: {e})")?,
    }

    if let Some((name, body)) = crash {
        writeln!(out, "\n=== {name} ===")?;
        if let Ok(s) = body {
            out.write_all(s.as_bytes())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("kopuz-logtest-{}-{name}", std::process::id()))
    }

    #[test]
    fn read_tail_small_file_returns_all() {
        let p = tmp("small");
        std::fs::write(&p, b"line1\nline2\nline3\n").unwrap();
        assert_eq!(read_tail(&p).unwrap(), "line1\nline2\nline3\n");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn read_tail_caps_at_64k_and_trims_partial_first_line() {
        let p = tmp("large");
        // One giant line (> 64 KiB, no newline) then a complete trailing line.
        // The 64 KiB window starts mid-giant-line, which must be dropped.
        let mut content = "x".repeat(70_000);
        content.push('\n');
        content.push_str("TAIL LINE");
        std::fs::write(&p, content.as_bytes()).unwrap();
        let out = read_tail(&p).unwrap();
        assert!(out.len() <= TAIL_BYTES as usize, "tail {} exceeds cap", out.len());
        assert_eq!(out, "TAIL LINE", "partial first line should be trimmed");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn read_tail_lossy_when_offset_splits_utf8() {
        let p = tmp("utf8");
        // 75_000 bytes of 3-byte chars: 64 KiB from the end lands mid-codepoint
        // (75000-65536 = 9464, 9464 % 3 = 2). read_to_string would error here
        // and drop the tail; the lossy read must still return it.
        std::fs::write(&p, "€".repeat(25_000).as_bytes()).unwrap();
        assert!(
            read_tail(&p).is_some(),
            "mid-UTF-8 window boundary should not drop the tail"
        );
        let _ = std::fs::remove_file(&p);
    }
}
