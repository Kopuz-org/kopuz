//! Nextcloud over raw WebDAV, on the nextcloud-rs crate.
//!
//! No music API, so the library comes from the tree's shape rather than from
//! tags, which would mean downloading every file. Instances running the Music
//! app speak Subsonic. Prefer that source, it carries real metadata.
//!
//! WebDAV has only Basic auth and no signed-URL form, so stream URLs carry
//! userinfo and covers cache to disk (an img tag won't send credentials).

use std::path::{Path, PathBuf};

use nextcloud::files::path as dav_path;
use nextcloud::{Depth, Nextcloud};

mod tree;

use reader::probe::{CoverProbe, probe_embedded_cover};
pub(crate) use tree::{ArtTrack, NextcloudAlbum, NextcloudTrack};
use tree::{extension, group, is_audio, within_roots};

/// A first-run guess, tried in order, used only until the user picks folders.
/// No fallback to `/`, that is an infinity PROPFIND over a whole account.
const ROOT_CANDIDATES: &[&str] = &["/Music", "/music", "/Musik", "/Musique"];

/// Enough for a FLAC STREAMINFO, an MP3 Xing frame or a front-loaded MP4 moov.
const PROBE_HEAD_BYTES: u64 = 256 * 1024;

/// Ogg states its length in the last page, so the tail only needs to be long
/// enough to hold one page header.
const PROBE_TAIL_BYTES: u64 = 64 * 1024;

/// Read windows for a duration, tried in order. Only a file whose tags push the
/// audio past the first needs the second.
const DURATION_HEAD_STEPS: &[u64] = &[32 * 1024, PROBE_HEAD_BYTES];

/// Read windows for embedded art. Only a file whose tags run past one is read
/// again, so this is a ceiling rather than a cost per track. Megabyte-sized art
/// pushes the end of the tags well past the first step.
const ART_HEAD_STEPS: &[u64] = &[PROBE_HEAD_BYTES, 2 * 1024 * 1024];

/// Grid size. The cache keeps what the server returns, so it caps every view.
const PREVIEW_SIZE: u32 = 512;

/// Extensions a cached cover can land under, for the cache-hit check that runs
/// before a fetch settles the format.
const ART_EXTENSIONS: &[&str] = &["jpg", "png", "webp", "gif"];

/// What reading a track's own header turned up.
enum EmbeddedArt {
    Found(Vec<u8>, &'static str),
    /// Tags state no picture, so nothing else will find one either.
    NoArt,
    /// Tags unreachable from the file's front; the server may still have art.
    Unreadable,
}

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

/// The containing directory of a remote path, clamped at the root.
pub fn parent_dir(remote_path: &str) -> String {
    dav_path::parent(remote_path)
}

/// The last segment of a remote path, empty at the root.
pub fn folder_name(remote_path: &str) -> &str {
    dav_path::name(remote_path)
}

/// Sub-directories of `remote_path`, for the settings folder browser. Takes raw
/// creds because the picker runs before the source is (re)built.
pub async fn browse_folders(
    server_url: &str,
    user_id: &str,
    password: &str,
    remote_path: &str,
) -> Result<Vec<String>, String> {
    NextcloudClient::new(server_url, user_id, password)?
        .list_dirs(remote_path)
        .await
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

    /// The music tree as albums and tracks, one infinity-depth PROPFIND per
    /// root; empty `roots` falls back to the first-run guess. An unreadable
    /// root is skipped, so this errors only when nothing at all was listed.
    pub(crate) async fn scan(
        &self,
        roots: &[String],
    ) -> Result<(Vec<NextcloudAlbum>, Vec<NextcloudTrack>), String> {
        let roots = if roots.is_empty() {
            vec![self.guess_music_root().await?]
        } else {
            roots.iter().map(|r| dav_path::normalise(r)).collect()
        };

        let mut albums = Vec::new();
        let mut tracks = Vec::new();
        let mut failures = Vec::new();
        for root in &roots {
            let entries = match self
                .nc
                .files()
                .propfind(root, Depth::Infinity, nextcloud::files::DEFAULT_PROPS)
                .await
            {
                Ok(entries) => entries,
                // A renamed or unshared root must not void the rest.
                Err(e) => {
                    tracing::warn!(root, error = %e, "nextcloud folder unreadable");
                    failures.push(format!("{root}: {e}"));
                    continue;
                }
            };
            let (root_albums, root_tracks) = group(root, &entries);
            albums.extend(root_albums);
            tracks.extend(root_tracks);
        }

        if albums.is_empty() && tracks.is_empty() && !failures.is_empty() {
            return Err(format!("could not list {}", failures.join("; ")));
        }

        // Nested roots (a folder and its parent) would list a track twice.
        albums.sort_by(|a, b| a.path.cmp(&b.path));
        albums.dedup_by(|a, b| a.path == b.path);
        tracks.sort_by(|a, b| a.path.cmp(&b.path));
        tracks.dedup_by(|a, b| a.path == b.path);
        Ok((albums, tracks))
    }

    /// Sub-directories of `path`, sorted by name, for the settings folder
    /// browser. Errors when the listing request fails.
    pub(crate) async fn list_dirs(&self, path: &str) -> Result<Vec<String>, String> {
        let path = dav_path::normalise(path);
        let entries = self
            .nc
            .files()
            .propfind(&path, Depth::One, nextcloud::files::DEFAULT_PROPS)
            .await
            .map_err(|e| format!("could not list {path}: {e}"))?;

        let mut dirs: Vec<String> = entries
            .into_iter()
            // PROPFIND depth 1 includes the collection itself.
            .filter(|e| e.is_directory && dav_path::normalise(&e.path) != path)
            .map(|e| e.path)
            .collect();
        dirs.sort_by_key(|p| dav_path::name(p).to_lowercase());
        Ok(dirs)
    }

    /// Paths of every audio file starred in Nextcloud itself, restricted to the
    /// configured roots so a starred file outside the library isn't favourited
    /// against a track the library doesn't hold.
    pub(crate) async fn favorites(&self, roots: &[String]) -> Result<Vec<String>, String> {
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
            .filter(|path| within_roots(path, roots))
            .collect())
    }

    pub(crate) async fn set_favorite(&self, remote_path: &str, on: bool) -> Result<(), String> {
        self.nc
            .files()
            .set_favorite(remote_path, on)
            .await
            .map_err(|e| format!("could not update favourite: {e}"))
    }

    /// Track length in whole seconds, read from the file's header because
    /// WebDAV never reports one. Costs one ranged GET per duration head step,
    /// plus one for Ogg's tail. None when the header states no length (an MP3
    /// with no Xing frame, an MP4 whose moov trails the audio), so the UI shows
    /// no duration rather than a wrong one.
    pub(crate) async fn probe_duration(&self, remote_path: &str) -> Option<u64> {
        let extension = extension(remote_path);

        for window in DURATION_HEAD_STEPS {
            let head = match self
                .nc
                .files()
                .read_range(remote_path, 0, Some(window - 1))
                .await
            {
                Ok(Some(head)) => head,
                Ok(None) => return None,
                Err(e) => {
                    tracing::debug!(path = remote_path, error = %e, "duration probe failed");
                    return None;
                }
            };

            let info = reader::probe::read_head(&head.bytes, extension.as_deref());
            if info.duration_secs.is_some() {
                return info.duration_secs;
            }
            // Ogg states its length in the last page, never in the first.
            if head.bytes.starts_with(b"OggS") {
                return self.ogg_tail_duration(remote_path, &info, head.total).await;
            }
            // Whole file already read, so a wider window changes nothing.
            if head.total <= *window {
                break;
            }
        }
        None
    }

    /// Length of an Ogg file from its final page, given what the head stated
    /// and the file's total size. None if the tail cannot be read.
    async fn ogg_tail_duration(
        &self,
        remote_path: &str,
        head: &reader::probe::HeadInfo,
        total: u64,
    ) -> Option<u64> {
        let start = total.saturating_sub(PROBE_TAIL_BYTES);
        let expected = total.saturating_sub(start);
        let tail = self
            .nc
            .files()
            .download_range(remote_path, start, total.saturating_sub(1))
            .await
            .ok()?;
        if tail.len() as u64 != expected {
            tracing::debug!(path = remote_path, "nextcloud ignored the ogg tail range");
            return None;
        }
        reader::probe::ogg_duration(head, &tail)
    }

    /// Cache a sidecar image on disk. `None` rather than an error: art never
    /// fails a sync.
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
        write_cached(&dir, &target, &bytes).await
    }

    /// Cache the art of an album with no sidecar image, taken from a track: the
    /// picture in its tags, else the server's preview. `None` like
    /// [`cache_cover`](Self::cache_cover).
    pub(crate) async fn cache_track_art(&self, track: &ArtTrack) -> Option<PathBuf> {
        let dir = cover_cache_dir()?;
        let stem = cover_cache_stem(&track.path);
        if let Some(cached) = cached_art(&dir, &stem) {
            return Some(cached);
        }

        let (bytes, ext) = match self.embedded_art(&track.path).await {
            EmbeddedArt::Found(bytes, ext) => (bytes, ext),
            // Previews come out of the same tags, so asking buys nothing.
            EmbeddedArt::NoArt => return None,
            EmbeddedArt::Unreadable => self.preview_art(track).await?,
        };
        if bytes.is_empty() {
            return None;
        }
        write_cached(&dir, &dir.join(format!("{stem}.{ext}")), &bytes).await
    }

    /// The server's rendering of a track's art, addressed by file id because the
    /// by-path preview endpoint is the older, patchier one.
    async fn preview_art(&self, track: &ArtTrack) -> Option<(Vec<u8>, &'static str)> {
        let file_id = track.file_id?;
        let url = self
            .nc
            .previews()
            .url_for_file_id(
                file_id,
                nextcloud::preview::PreviewOptions::square(PREVIEW_SIZE),
            )
            .ok()?;

        match self.nc.previews().fetch(url).await {
            // No preview provider for the format, or previews are off.
            Ok(None) => None,
            Ok(Some(preview)) => {
                let ext = extension_for_mime(preview.content_type);
                Some((preview.bytes.to_vec(), ext))
            }
            Err(e) => {
                tracing::debug!(path = track.path, error = %e, "nextcloud preview fetch failed");
                None
            }
        }
    }

    /// The picture in the track's own tags, read from the file's front in
    /// [`ART_HEAD_STEPS`] windows. Only a file that reports truncated tags is
    /// read again, so a track with no art costs one small request.
    async fn embedded_art(&self, remote_path: &str) -> EmbeddedArt {
        let extension = extension(remote_path);
        let mut cover = None;

        for window in ART_HEAD_STEPS {
            let head = match self
                .nc
                .files()
                .read_range(remote_path, 0, Some(window - 1))
                .await
            {
                Ok(Some(head)) => head,
                Ok(None) => return EmbeddedArt::NoArt,
                Err(e) => {
                    tracing::debug!(path = remote_path, error = %e, "embedded art read failed");
                    return EmbeddedArt::Unreadable;
                }
            };

            match probe_embedded_cover(&head.bytes, extension.as_deref()) {
                CoverProbe::Found(found) => {
                    cover = Some(found);
                    break;
                }
                CoverProbe::None => return EmbeddedArt::NoArt,
                // Whole file already read, so a longer window changes nothing.
                CoverProbe::Truncated if head.total <= *window => {
                    return EmbeddedArt::Unreadable;
                }
                CoverProbe::Truncated => continue,
            }
        }

        let Some(cover) = cover else {
            return EmbeddedArt::Unreadable;
        };
        let ext = cover
            .extension
            .and_then(|ext| ART_EXTENSIONS.iter().find(|known| **known == ext).copied())
            .unwrap_or("jpg");
        EmbeddedArt::Found(cover.bytes, ext)
    }

    async fn guess_music_root(&self) -> Result<String, String> {
        for candidate in ROOT_CANDIDATES {
            if self.nc.files().exists(candidate).await.unwrap_or(false) {
                return Ok((*candidate).to_string());
            }
        }
        Err(format!(
            "no music folder found (looked for {}); pick one in Settings",
            ROOT_CANDIDATES.join(", ")
        ))
    }
}

fn cover_cache_dir() -> Option<PathBuf> {
    Some(
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")?
            .cache_dir()
            .join("nextcloud-covers"),
    )
}

/// Digest of the remote path, so albums sharing a directory name stay apart
/// without the name outgrowing the filesystem's 255-byte limit.
fn cover_cache_stem(remote_path: &str) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(remote_path.as_bytes()))
}

fn cover_cache_name(remote_path: &str) -> String {
    let stem = cover_cache_stem(remote_path);
    match extension(remote_path) {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem,
    }
}

/// A cached cover under any extension art can arrive as, for the hit check that
/// runs before a fetch settles the format.
fn cached_art(dir: &Path, stem: &str) -> Option<PathBuf> {
    ART_EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("{stem}.{ext}")))
        .find(|candidate| candidate.exists())
}

/// Write `bytes` to `target`, returning it, or `None` if the cache is unwritable.
/// The write lands on a temporary name first: readers only test `exists()`, so a
/// half-written file would be served as art for good.
async fn write_cached(dir: &Path, target: &Path, bytes: &[u8]) -> Option<PathBuf> {
    tokio::fs::create_dir_all(dir).await.ok()?;
    let staging = target.with_extension(format!(
        "{}.{}.part",
        target.extension().and_then(|e| e.to_str()).unwrap_or("bin"),
        std::process::id(),
    ));
    if tokio::fs::write(&staging, bytes).await.is_err() {
        let _ = tokio::fs::remove_file(&staging).await;
        return None;
    }
    if tokio::fs::rename(&staging, target).await.is_err() {
        let _ = tokio::fs::remove_file(&staging).await;
        return None;
    }
    Some(target.to_path_buf())
}

/// Anything unrecognised is stored as JPEG, the server's own default.
fn extension_for_mime(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers_clamp_at_the_root() {
        assert_eq!(parent_dir("/Music/Albums"), "/Music");
        assert_eq!(parent_dir("/Music/Albums/"), "/Music");
        assert_eq!(parent_dir("/Music"), "/");
        assert_eq!(parent_dir("/"), "/");

        assert_eq!(folder_name("/Music/Albums"), "Albums");
        assert_eq!(folder_name("/Music/Albums/"), "Albums");
        assert_eq!(folder_name("/"), "");
    }
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
