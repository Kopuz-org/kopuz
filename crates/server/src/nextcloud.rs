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
use nextcloud::{Depth, FileEntry, Nextcloud};

/// Fallback when the server reports no usable content type.
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "aac", "wav", "wma", "aiff", "alac",
];

const COVER_NAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.png",
];

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

pub(crate) struct NextcloudTrack {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_path: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
}

pub(crate) struct NextcloudAlbum {
    pub path: String,
    pub title: String,
    pub artist: String,
    /// Remote path, before the sync caches it.
    pub cover_path: Option<String>,
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

// Extension fallback: some storage backends label everything octet-stream.
fn is_audio(entry: &FileEntry) -> bool {
    if entry.is_directory {
        return false;
    }
    if let Some(mime) = entry.content_type.as_deref()
        && mime.starts_with("audio/")
    {
        return true;
    }
    extension(entry.name()).is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
}

fn is_cover_file(entry: &FileEntry) -> bool {
    !entry.is_directory && COVER_NAMES.contains(&entry.name().to_ascii_lowercase().as_str())
}

fn cover_rank(name: &str) -> usize {
    let lower = name.to_ascii_lowercase();
    COVER_NAMES
        .iter()
        .position(|candidate| *candidate == lower)
        .unwrap_or(usize::MAX)
}

fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
}

/// Title and leading track number of a file stem.
fn split_leading_number(stem: &str) -> (String, Option<u32>) {
    let digits: String = stem.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 3 {
        // four or more digits is a year
        return (stem.trim().to_string(), None);
    }

    let rest = stem[digits.len()..].trim_start();
    let rest = rest
        .strip_prefix('-')
        .or_else(|| rest.strip_prefix('.'))
        .or_else(|| rest.strip_prefix('_'))
        .unwrap_or(rest);
    let title = rest.trim();

    // A bare "01.mp3" keeps the digits rather than ending up titleless.
    if title.is_empty() {
        return (stem.trim().to_string(), digits.parse().ok());
    }
    (title.to_string(), digits.parse().ok())
}

/// Disc number from a "Disc 2" or "CD2" directory name.
fn disc_of(dir_name: &str) -> Option<u32> {
    let lower = dir_name.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("disc")
        .or_else(|| lower.strip_prefix("disk"))
        .or_else(|| lower.strip_prefix("cd"))?;
    rest.trim_start_matches([' ', '-', '_'])
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
}

/// Group a flat listing by directory: album is the container (or grandparent,
/// for a disc folder), artist the level above, empty one level from the root.
fn group(root: &str, entries: &[FileEntry]) -> (Vec<NextcloudAlbum>, Vec<NextcloudTrack>) {
    let mut covers: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for entry in entries.iter().filter(|e| is_cover_file(e)) {
        let dir = dav_path::parent(&entry.path);
        // Earliest COVER_NAMES entry wins, so cover.jpg beats front.png.
        match covers.get(&dir) {
            Some(held) if cover_rank(dav_path::name(held)) <= cover_rank(entry.name()) => {}
            _ => {
                covers.insert(dir, entry.path.clone());
            }
        }
    }

    let mut albums: std::collections::HashMap<String, NextcloudAlbum> =
        std::collections::HashMap::new();
    let mut tracks = Vec::new();

    for entry in entries.iter().filter(|e| is_audio(e)) {
        let container = dav_path::parent(&entry.path);
        let container_name = dav_path::name(&container);
        let disc = disc_of(container_name);

        let album_path = if disc.is_some() {
            dav_path::parent(&container)
        } else {
            container.clone()
        };
        let album_name = dav_path::name(&album_path);
        let artist_path = dav_path::parent(&album_path);
        let artist_name = dav_path::name(&artist_path);

        let artist = if artist_path == dav_path::normalise(root) || artist_name.is_empty() {
            String::new()
        } else {
            artist_name.to_string()
        };

        let stem = Path::new(entry.name())
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.name().to_string());
        let (title, track_number) = split_leading_number(&stem);

        let album_title = if album_name.is_empty() {
            "Unknown Album".to_string()
        } else {
            album_name.to_string()
        };

        albums
            .entry(album_path.clone())
            .or_insert_with(|| NextcloudAlbum {
                path: album_path.clone(),
                title: album_title.clone(),
                artist: artist.clone(),
                cover_path: covers
                    .get(&album_path)
                    .or_else(|| covers.get(&container))
                    .cloned(),
            });

        tracks.push(NextcloudTrack {
            path: entry.path.clone(),
            title,
            artist: artist.clone(),
            album: album_title,
            album_path,
            track_number,
            disc_number: disc,
        });
    }

    let mut albums: Vec<NextcloudAlbum> = albums.into_values().collect();
    albums.sort_by(|a, b| a.path.cmp(&b.path));
    (albums, tracks)
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

    fn entry(path: &str, is_dir: bool, mime: Option<&str>) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            is_directory: is_dir,
            content_type: mime.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn is_audio_checks_mime_then_extension() {
        assert!(is_audio(&entry("/Music/a/b/1.mp3", false, None)));
        assert!(is_audio(&entry(
            "/Music/a/b/x.dat",
            false,
            Some("audio/mpeg")
        )));
        assert!(is_audio(&entry(
            "/Music/a/b/2.flac",
            false,
            Some("application/octet-stream")
        )));
        assert!(!is_audio(&entry("/Music/a/b/notes.txt", false, None)));
        assert!(!is_audio(&entry("/Music/a/b", true, None)));
    }

    #[test]
    fn split_leading_number_strips_track_prefix() {
        assert_eq!(
            split_leading_number("01 - Bloom"),
            ("Bloom".to_string(), Some(1))
        );
        assert_eq!(
            split_leading_number("02. Codex"),
            ("Codex".to_string(), Some(2))
        );
        assert_eq!(
            split_leading_number("003_Separator"),
            ("Separator".to_string(), Some(3))
        );
        assert_eq!(
            split_leading_number("Lotus Flower"),
            ("Lotus Flower".to_string(), None)
        );
        assert_eq!(
            split_leading_number("1999 remaster"),
            ("1999 remaster".to_string(), None)
        );
        assert_eq!(split_leading_number("01"), ("01".to_string(), Some(1)));
    }

    #[test]
    fn disc_of_parses_disc_folder_names() {
        assert_eq!(disc_of("Disc 2"), Some(2));
        assert_eq!(disc_of("disk3"), Some(3));
        assert_eq!(disc_of("CD-1"), Some(1));
        assert_eq!(disc_of("Live in Tokyo"), None);
        assert_eq!(disc_of("Disc"), None);
    }

    #[test]
    fn group_builds_albums_and_tracks() {
        let entries = vec![
            entry("/Music/Radiohead", true, None),
            entry("/Music/Radiohead/Kid A", true, None),
            entry(
                "/Music/Radiohead/Kid A/cover.jpg",
                false,
                Some("image/jpeg"),
            ),
            entry("/Music/Radiohead/Kid A/01 - Everything.flac", false, None),
            entry("/Music/Radiohead/Kid A/02 - Kid A.flac", false, None),
        ];
        let (albums, tracks) = group("/Music", &entries);

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].title, "Kid A");
        assert_eq!(albums[0].artist, "Radiohead");
        assert_eq!(
            albums[0].cover_path.as_deref(),
            Some("/Music/Radiohead/Kid A/cover.jpg")
        );

        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "Everything");
        assert_eq!(tracks[0].artist, "Radiohead");
        assert_eq!(tracks[0].album, "Kid A");
        assert_eq!(tracks[0].track_number, Some(1));
        assert!(tracks[0].disc_number.is_none());
    }

    #[test]
    fn group_folds_disc_folders() {
        let entries = vec![
            entry("/Music/Artist/Album/Disc 1/01 - A.mp3", false, None),
            entry("/Music/Artist/Album/Disc 2/01 - B.mp3", false, None),
        ];
        let (albums, tracks) = group("/Music", &entries);

        assert_eq!(albums.len(), 1, "both discs belong to one album");
        assert_eq!(albums[0].path, "/Music/Artist/Album");
        assert_eq!(tracks[0].disc_number, Some(1));
        assert_eq!(tracks[1].disc_number, Some(2));
        assert_eq!(tracks[1].album, "Album");
    }

    #[test]
    fn group_leaves_root_albums_artistless() {
        let entries = vec![entry("/Music/Loose Album/01 - Track.mp3", false, None)];
        let (albums, tracks) = group("/Music", &entries);

        assert_eq!(albums[0].title, "Loose Album");
        assert!(albums[0].artist.is_empty());
        assert!(tracks[0].artist.is_empty());
    }

    #[test]
    fn group_ranks_cover_names() {
        let entries = vec![
            entry("/Music/A/B/front.png", false, None),
            entry("/Music/A/B/cover.jpg", false, None),
            entry("/Music/A/B/01 - T.mp3", false, None),
        ];
        let (albums, _) = group("/Music", &entries);
        assert_eq!(
            albums[0].cover_path.as_deref(),
            Some("/Music/A/B/cover.jpg")
        );
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
