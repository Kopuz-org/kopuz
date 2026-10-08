//! Cookie reader for a Gecko sign-in profile.
//!
//! Firefox and its forks keep `cookies.sqlite` in the clear, so there is no
//! per-platform key material to chase here — one reader serves Linux, macOS
//! and Windows. What it does have to handle is the write-ahead log: the
//! cookies a sign-in just wrote live there, not in the main database, until
//! Gecko checkpoints. Copying both and reading the copy gets them without
//! waiting and without disturbing the running browser.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use config::Browser;

use super::store::{Cookie, host_matches_domain};

pub(crate) async fn read_cookies(
    browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    let src = pick_cookies_path(profile_root).ok_or_else(|| {
        format!(
            "no cookies.sqlite under {} — is `{}` installed?",
            profile_root.display(),
            browser.label()
        )
    })?;
    let snapshot = Snapshot::of(&src).await?;
    let cookies = read_store(snapshot.db(), domain).await?;
    tracing::trace!(
        browser = browser.id(),
        domain,
        count = cookies.len(),
        "read cookies from isolated profile"
    );
    Ok(cookies)
}

/// `--profile <dir>` puts the store straight in the directory, but a Gecko
/// that decided to make a profile of its own nests it one level down.
fn pick_cookies_path(profile_root: &Path) -> Option<PathBuf> {
    let direct = profile_root.join("cookies.sqlite");
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(profile_root).ok()?.find_map(|entry| {
        let nested = entry.ok()?.path().join("cookies.sqlite");
        nested.exists().then_some(nested)
    })
}

/// A private copy of the store, removed when it goes out of scope.
struct Snapshot {
    dir: PathBuf,
}

impl Snapshot {
    async fn of(src: &Path) -> Result<Self, String> {
        // Unique per call (pid + counter) so concurrent reads don't clobber
        // each other.
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kopuz-moz-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("snapshot dir: {e}"))?;
        let snapshot = Self { dir };
        // The -shm is deliberately left behind: SQLite rebuilds it from the
        // log, and a copy taken a moment apart from the log is worse than none.
        for suffix in ["", "-wal", "-journal"] {
            let from = src.with_file_name(format!("cookies.sqlite{suffix}"));
            if tokio::fs::try_exists(&from).await.unwrap_or(false) {
                tokio::fs::copy(&from, snapshot.dir.join(format!("cookies.sqlite{suffix}")))
                    .await
                    .map_err(|e| format!("copy cookies.sqlite{suffix}: {e}"))?;
            }
        }
        Ok(snapshot)
    }

    fn db(&self) -> PathBuf {
        self.dir.join("cookies.sqlite")
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Read-write (not read_only) so opening the copy replays its write-ahead log;
/// a read-only open would see only the cookies Gecko has already checkpointed.
async fn read_store(db: PathBuf, domain: &str) -> Result<Vec<Cookie>, String> {
    use sqlx::{ConnectOptions, Row};

    let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db)
        .create_if_missing(false)
        .connect()
        .await
        .map_err(|e| format!("open cookies.sqlite: {e}"))?;
    let rows = sqlx::query("SELECT host, name, value FROM moz_cookies")
        .fetch_all(&mut conn)
        .await
        .map_err(|e| format!("query moz_cookies: {e}"))?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let host: String = row.try_get("host").unwrap_or_default();
            if !host_matches_domain(&host, domain) {
                return None;
            }
            Some(Cookie {
                name: row.try_get("name").unwrap_or_default(),
                value: row.try_get("value").unwrap_or_default(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    use sqlx::{ConnectOptions, Executor};

    async fn profile_with_cookies(rows: &[(&str, &str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("cookies.sqlite"))
            .create_if_missing(true)
            .connect()
            .await
            .expect("create store");
        conn.execute("CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT)")
            .await
            .expect("create table");
        for (host, name, value) in rows {
            sqlx::query("INSERT INTO moz_cookies (host, name, value) VALUES (?1, ?2, ?3)")
                .bind(host)
                .bind(name)
                .bind(value)
                .execute(&mut conn)
                .await
                .expect("insert");
        }
        dir
    }

    #[tokio::test]
    async fn only_the_asked_for_domain_comes_back() {
        let profile = profile_with_cookies(&[
            (".youtube.com", "SAPISID", "yes"),
            ("music.youtube.com", "SID", "also"),
            ("notyoutube.com", "SAPISID", "no"),
            ("soundcloud.com", "oauth_token", "no"),
        ])
        .await;
        let cookies = read_cookies(Browser::Firefox, profile.path(), "youtube.com")
            .await
            .expect("read");
        let mut names: Vec<&str> = cookies.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["SAPISID", "SID"]);
        assert!(cookies.iter().all(|c| c.value != "no"));
    }

    #[tokio::test]
    async fn a_profile_without_a_store_says_so() {
        let empty = tempfile::tempdir().expect("tempdir");
        let error = read_cookies(Browser::Firefox, empty.path(), "youtube.com")
            .await
            .expect_err("no store");
        assert!(error.contains("cookies.sqlite"), "{error}");
    }
}
