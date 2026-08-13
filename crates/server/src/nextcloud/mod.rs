//! Nextcloud over raw WebDAV, on the nextcloud-rs crate.
//!
//! No music API, so the library comes from the tree's shape rather than from
//! tags, which would mean downloading every file. Instances running the Music
//! app speak Subsonic. Prefer that source, it carries real metadata.
//!
//! WebDAV has only Basic auth and no signed-URL form, so stream URLs carry
//! userinfo and covers cache to disk (an img tag won't send credentials).

use std::path::PathBuf;

use nextcloud::files::path as dav_path;
use nextcloud::{Depth, Nextcloud};

mod tree;

pub(crate) use tree::{NextcloudAlbum, NextcloudTrack};
use tree::{extension, group, is_audio};

/// Tried in order. No fallback to `/`: infinity PROPFIND over a whole account.
const ROOT_CANDIDATES: &[&str] = &["/Music", "/music", "/Musik", "/Musique"];

/// Slashes escape too: segments encode separately, so one inside a file name
/// is not a separator.
const SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode_segment(s: &str) -> percent_encoding::PercentEncode<'_> {
    percent_encoding::utf8_percent_encode(s, SEGMENT)
}

pub fn stream_url(
    server_url: &str,
    user_id: &str,
    password: &str,
    remote_path: &str,
) -> Result<String, String> {
    Ok(NextcloudClient::new(server_url, user_id, password)?.stream_url(remote_path))
}

pub(crate) struct NextcloudClient {
    nc: Nextcloud,
    /// Carries userinfo, unlike the one inside `nc`.
    authed_base: String,
    user_id: String,
}

impl NextcloudClient {
    pub(crate) fn new(url: &str, user_id: &str, password: &str) -> Result<Self, String> {
        let nc = Nextcloud::builder(url)
            .basic_auth(user_id, password)
            .user_id(user_id)
            .user_agent(concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
            .timeout(Some(std::time::Duration::from_secs(180))) // scans run long
            .build()
            .map_err(|e| format!("invalid Nextcloud server URL: {e}"))?;

        let mut authed = nc.base_url().clone();
        authed
            .set_username(user_id)
            .and_then(|()| authed.set_password(Some(password)))
            .map_err(|()| "server URL cannot carry credentials".to_string())?;

        // Subpath installs ("https://host/nextcloud") have no trailing slash.
        let mut authed_base = authed.to_string();
        if !authed_base.ends_with('/') {
            authed_base.push('/');
        }

        Ok(Self {
            nc,
            authed_base,
            user_id: user_id.to_string(),
        })
    }

    pub(crate) async fn ping(&self) -> Result<(), nextcloud::Error> {
        self.nc.files().stat("/").await.map(|_| ())
    }

    // Hand-built: nextcloud-rs keeps its DAV URL builder private.
    pub(crate) fn stream_url(&self, remote_path: &str) -> String {
        let encoded = dav_path::normalise(remote_path)
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|segment| encode_segment(segment).to_string())
            .collect::<Vec<_>>()
            .join("/");

        format!(
            "{}remote.php/dav/files/{}/{encoded}",
            self.authed_base,
            encode_segment(&self.user_id),
        )
    }

    /// The music tree as albums and tracks, in one infinity-depth PROPFIND.
    pub(crate) async fn scan(&self) -> Result<(Vec<NextcloudAlbum>, Vec<NextcloudTrack>), String> {
        let root = self.music_root().await?;
        let entries = self
            .nc
            .files()
            .propfind(&root, Depth::Infinity, nextcloud::files::DEFAULT_PROPS)
            .await
            .map_err(|e| format!("could not list {root}: {e}"))?;

        Ok(group(&root, &entries))
    }

    /// Paths of every audio file starred in Nextcloud itself.
    pub(crate) async fn favorites(&self) -> Result<Vec<String>, String> {
        let entries = self
            .nc
            .files()
            .favorites("/")
            .await
            .map_err(|e| format!("could not read favourites: {e}"))?;

        Ok(entries
            .into_iter()
            .filter(is_audio)
            .map(|entry| entry.path)
            .collect())
    }

    pub(crate) async fn set_favorite(&self, remote_path: &str, on: bool) -> Result<(), String> {
        self.nc
            .files()
            .set_favorite(remote_path, on)
            .await
            .map_err(|e| format!("could not update favourite: {e}"))
    }

    /// Cache the art on disk. `None` rather than an error: art never fails a sync.
    pub(crate) async fn cache_cover(&self, remote_path: &str) -> Option<PathBuf> {
        let dir = cover_cache_dir()?;
        let target = dir.join(cover_cache_name(remote_path));
        if target.exists() {
            return Some(target);
        }

        let bytes = match self.nc.files().download(remote_path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::debug!(path = remote_path, error = %e, "nextcloud cover fetch failed");
                return None;
            }
        };

        tokio::fs::create_dir_all(&dir).await.ok()?;
        tokio::fs::write(&target, &bytes).await.ok()?;
        Some(target)
    }

    async fn music_root(&self) -> Result<String, String> {
        for candidate in ROOT_CANDIDATES {
            if self.nc.files().exists(candidate).await.unwrap_or(false) {
                return Ok((*candidate).to_string());
            }
        }
        Err(format!(
            "no music folder found; expected one of {}",
            ROOT_CANDIDATES.join(", ")
        ))
    }
}

fn cover_cache_dir() -> Option<PathBuf> {
    Some(
        directories::ProjectDirs::from("com", "temidaradev", "kopuz")?
            .cache_dir()
            .join("nextcloud-covers"),
    )
}

/// Digest of the remote path, so albums sharing a directory name stay apart
/// without the name outgrowing the filesystem's 255-byte limit.
fn cover_cache_name(remote_path: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = hex::encode(Sha256::digest(remote_path.as_bytes()));
    match extension(remote_path) {
        Some(ext) => format!("{digest}.{ext}"),
        None => digest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_cache_name_hashes_path() {
        let a = cover_cache_name("/Music/A/Album/cover.jpg");
        let b = cover_cache_name("/Music/B/Album/cover.jpg");
        assert_ne!(a, b);
        assert!(a.ends_with(".jpg"));

        let deep = cover_cache_name(&format!("/Music/{}/cover.jpg", "x".repeat(400)));
        assert!(deep.len() < 255, "must stay a writable file name");
    }

    #[test]
    fn stream_url_carries_auth_and_escapes() {
        let client = NextcloudClient::new("https://cloud.example.test", "alice", "app-pw")
            .expect("client builds");
        let url = client.stream_url("/Music/a b/track #1.mp3");

        let parsed = reqwest::Url::parse(&url).expect("valid stream URL");
        assert_eq!(parsed.username(), "alice");
        assert_eq!(parsed.password(), Some("app-pw"));
        assert_eq!(
            parsed.path(),
            "/remote.php/dav/files/alice/Music/a%20b/track%20%231.mp3"
        );
    }

    #[test]
    fn stream_url_handles_subpath_install() {
        let client = NextcloudClient::new("https://host.test/nextcloud", "alice", "app-pw")
            .expect("client builds");
        let url = client.stream_url("/Music/t.mp3");

        let parsed = reqwest::Url::parse(&url).expect("valid stream URL");
        assert_eq!(
            parsed.path(),
            "/nextcloud/remote.php/dav/files/alice/Music/t.mp3"
        );
    }
}
