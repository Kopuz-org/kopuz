//! Source-agnostic cover resolution (issue #347 / #35).
//!
//! The UI calls these instead of branching on local-file-vs-remote-URL or
//! `match service` per row: the source layer owns where a cover *lives* and how
//! to turn it into a renderable URL. Local resolves the on-disk file to an
//! `artwork://` asset; a server resolves its remote image URL (per service).
//!
//! These are sync free functions, not [`MediaSource`](crate::source::MediaSource)
//! methods, because they run per-row in long lists — they must not allocate a
//! `Box<dyn>` per cover. Capabilities are a trait method (resolved once); cover
//! resolution is a hot, allocation-light function keyed on the config + service.

use std::path::{Path, PathBuf};

use config::{AppConfig, MusicService};
use reader::{CoverRef, Track};
use utils::CoverUrl;

pub(crate) fn jellyfin_item_url(
    server_url: &str,
    item_id: &str,
    image_tag: Option<&str>,
    access_token: Option<&str>,
    max_width: u32,
    quality: u32,
) -> String {
    let mut params = vec![
        format!("maxWidth={max_width}"),
        format!("quality={quality}"),
    ];
    if let Some(tag) = image_tag {
        params.push(format!("tag={tag}"));
    }
    if let Some(token) = access_token {
        params.push(format!("api_key={token}"));
    }
    format!(
        "{server_url}/Items/{item_id}/Images/Primary?{}",
        params.join("&")
    )
}

fn subsonic_item_url(
    server_url: &str,
    item_id: &str,
    access_token: Option<&str>,
    max_width: u32,
    quality: u32,
) -> Option<String> {
    if server_url.is_empty() || item_id.is_empty() {
        return None;
    }
    let mut url = reqwest::Url::parse(&format!(
        "{}/rest/getCoverArt.view",
        server_url.trim_end_matches('/')
    ))
    .ok()?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("id", item_id);
        pairs.append_pair("size", &max_width.to_string());
        pairs.append_pair("quality", &quality.to_string());
        if let Some(token) = access_token {
            pairs.append_pair("access_token", token);
        }
    }
    Some(url.to_string())
}

pub fn remote_artwork_url_at_size(url: String, max_width: u32) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(&url) else {
        return url;
    };
    if !parsed.path().ends_with("/rest/getCoverArt.view") {
        return url;
    }

    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| key != "size")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    {
        let mut query = parsed.query_pairs_mut();
        query.clear();
        query.extend_pairs(
            pairs
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
        query.append_pair("size", &max_width.to_string());
    }
    parsed.to_string()
}

/// Where a cover's bytes actually are.
///
/// [`resolve`] answers the same question as a URL, which forces whoever wants
/// the bytes to parse the URL back apart. Anything that reads a cover rather
/// than rendering it -- the daemon's artwork service, palette extraction --
/// asks for this instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    File(PathBuf),
    /// Already signed where the service needs it, so it must not be logged.
    Url(String),
}

/// Locate a cover's bytes: an on-disk file, or a URL to fetch.
pub fn locate(config: &AppConfig, cover: CoverRef, max_width: u32) -> Option<Located> {
    if let CoverRef::Local(path) = cover {
        return Some(Located::File(path));
    }
    match resolve(config, cover, max_width)? {
        // Neither is fetchable: a data URL carries its own bytes, and
        // `artwork://` is the app's scheme for asking the daemon -- which is
        // whoever is calling this.
        url if url.starts_with("data:") || url.starts_with("artwork://") => None,
        url => Some(Located::Url(url.as_ref().to_string())),
    }
}

/// Resolve a typed cover reference to a renderable URL.
pub fn resolve(config: &AppConfig, cover: CoverRef, max_width: u32) -> Option<CoverUrl> {
    let server = config.server.as_ref();
    let url = match cover {
        CoverRef::Local(path) => return utils::format_artwork_url(Some(&path)),
        CoverRef::EmbeddedUrl(url) => remote_artwork_url_at_size(url, max_width),
        CoverRef::JellyfinItem { item_id, tag } => {
            let server = server.filter(|server| server.service == MusicService::Jellyfin)?;
            jellyfin_item_url(
                &server.url,
                &item_id,
                tag.as_deref(),
                server.access_token.as_deref(),
                max_width,
                80,
            )
        }
        CoverRef::SubsonicItem { item_id, signed } => {
            let server = server.filter(|server| {
                matches!(
                    server.service,
                    MusicService::Subsonic | MusicService::Custom
                )
            })?;
            if signed {
                let (Some(password), Some(username)) =
                    (server.access_token.as_deref(), server.user_id.as_deref())
                else {
                    return None;
                };
                crate::subsonic::cover_art_url(
                    &server.url,
                    username,
                    password,
                    &item_id,
                    Some(max_width),
                )
                .ok()?
            } else {
                subsonic_item_url(
                    &server.url,
                    &item_id,
                    server.access_token.as_deref(),
                    max_width,
                    80,
                )?
            }
        }
        CoverRef::None => return None,
    };
    Some(utils::cover_url_from_string(url))
}

/// Resolve a cover from a stored cover-path ref — album covers and artist-grid
/// images, where the ref is a filesystem path (local) or a remote image path /
/// `directurl:` form (a server). `max_width` sizes the request.
///
/// Dispatches on the ref's own shape, NOT the active source: a local cover is an
/// absolute filesystem path; a remote cover is a service-encoded ref
/// (`ytmusic:_:urlhex_…`, `jellyfin:id:tag`, `directurl:…`) — never absolute. A
/// frame of stale content from a just-switched-away source must not resolve a
/// remote ref against the wrong arm — feeding a remote ref to the local
/// `artwork://` path makes the artwork server `open()` it as a filename (→
/// `ENAMETOOLONG`), and a stale local path would hit the remote resolver.
pub fn from_path(
    config: &AppConfig,
    cover_path: Option<&Path>,
    max_width: u32,
) -> Option<CoverUrl> {
    let stored = cover_path?.to_string_lossy();
    resolve(config, CoverRef::parse(&stored), max_width)
}

/// Resolve a track's cover, dispatching on the **track's own source** (not the
/// active source) so a mixed list — e.g. a server track in the now-playing queue
/// while Local is active — still resolves correctly. Every track self-describes
/// its cover via `track.cover`: a local row's `cover_path` is projected from its
/// album by the DB read layer (so it's a filesystem path), a server row carries
/// the per-service remote ref. No caller-side album lookup.
pub fn track(config: &AppConfig, track: &Track, max_width: u32) -> Option<CoverUrl> {
    resolve(config, CoverRef::for_track(track), max_width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn local_active() -> AppConfig {
        AppConfig {
            active_source: config::Source::default(),
            server: None,
            ..Default::default()
        }
    }

    fn subsonic_track(item_id: &str, cover: Option<&str>) -> Track {
        Track {
            id: reader::TrackId::Server {
                service: MusicService::Subsonic,
                item_id: item_id.to_string(),
            },
            cover: cover.map(str::to_string),
            album_id: String::new(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            credits: Vec::new(),
            artists: Vec::new(),
            replay_gain: config::ReplayGainInfo::default(),
            explicit: false,
            plays: None,
        }
    }

    fn subsonic_config(with_creds: bool) -> AppConfig {
        AppConfig {
            active_source: config::Source::default(),
            server: Some(config::MusicServer {
                url: "https://sub.example.com".into(),
                service: MusicService::Subsonic,
                access_token: with_creds.then(|| "pw".to_string()),
                user_id: with_creds.then(|| "alice".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn jellyfin_config() -> AppConfig {
        AppConfig {
            active_source: config::Source::default(),
            server: Some(config::MusicServer {
                url: "https://jelly.example.com".into(),
                service: MusicService::Jellyfin,
                access_token: Some("token".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn subsonic_track_without_cover_path_falls_back_to_getcoverart() {
        // `cover == "none"` is the no-embedded-cover sentinel; the typed
        // resolver falls back to a signed getCoverArt URL keyed by the track id.
        let track = subsonic_track("TR-42", Some("none"));
        let got = super::track(&subsonic_config(true), &track, 800).expect("fallback cover url");
        let s: &str = &got;
        assert!(s.contains("getCoverArt"), "got: {s}");
        assert!(s.contains("TR-42"), "keyed by the track id: {s}");
        assert!(s.contains("alice"), "signed with the username: {s}");

        // Without credentials the fallback can't sign a request → no cover.
        assert!(super::track(&subsonic_config(false), &track, 800).is_none());
    }

    #[test]
    fn subsonic_item_without_the_sentinel_uses_the_token_lookup() {
        let got = resolve(
            &subsonic_config(true),
            CoverRef::SubsonicItem {
                item_id: "AL-7".to_string(),
                signed: false,
            },
            512,
        )
        .expect("cover url");
        assert!(got.contains("getCoverArt"), "got: {got}");
        assert!(got.contains("id=AL-7"), "keyed by the item: {got}");
        assert!(got.contains("size=512"), "sized by the view: {got}");
        assert!(
            got.contains("access_token=pw"),
            "token-authenticated: {got}"
        );

        // A Jellyfin ref must never resolve against a Subsonic server.
        assert!(
            resolve(
                &subsonic_config(true),
                CoverRef::JellyfinItem {
                    item_id: "AL-7".to_string(),
                    tag: None
                },
                512
            )
            .is_none()
        );
    }

    /// SoundCloud stores the artwork URL itself, so its covers resolve with no
    /// server configured at all.
    #[test]
    fn soundcloud_track_resolves_without_a_server() {
        let url = "https://i1.sndcdn.com/artworks-1:2-large.jpg";
        let mut track = subsonic_track("SC-1", Some(url));
        track.id = reader::TrackId::Server {
            service: MusicService::SoundCloud,
            item_id: "SC-1".to_string(),
        };
        let got = super::track(&local_active(), &track, 500).expect("artwork url");
        assert_eq!(&*got, url);
    }

    #[test]
    fn from_path_resolves_a_remote_ref_while_local_is_active() {
        // The regression: one frame after switching away from YT, its album covers
        // (`ytmusic:_:urlhex_<url>`) are still rendered. With Local active they must
        // resolve to the embedded URL — NOT get fed to the local artwork:// path as
        // a filename (the artwork server would open() it → ENAMETOOLONG).
        let url = "https://example.com/cover.jpg";
        let reff = format!("ytmusic:_:{}", CoverRef::encode_url(url));
        let got = from_path(&local_active(), Some(Path::new(&reff)), 200).expect("resolves");
        assert_eq!(
            &*got, url,
            "self-contained remote ref → its URL, not artwork://"
        );
    }

    #[test]
    fn typed_jellyfin_ref_resolves_the_referenced_item_and_tag() {
        let got = resolve(
            &jellyfin_config(),
            CoverRef::JellyfinItem {
                item_id: "album-42".to_string(),
                tag: Some("primary-tag".to_string()),
            },
            640,
        )
        .expect("jellyfin cover");
        assert!(got.contains("/Items/album-42/Images/Primary"));
        assert!(got.contains("tag=primary-tag"));
        assert!(got.contains("maxWidth=640"));
    }

    #[test]
    fn embedded_url_resolves_without_an_active_server() {
        let url = "https://images.example/cover.jpg";
        let got = resolve(&local_active(), CoverRef::EmbeddedUrl(url.to_string()), 320)
            .expect("embedded cover");
        assert_eq!(&*got, url);
    }

    #[test]
    fn persisted_subsonic_urls_follow_the_view_size() {
        let url =
            "https://music.example/rest/getCoverArt.view?u=user&t=token&s=salt&id=cover-1&size=512";
        let got = resolve(
            &local_active(),
            CoverRef::EmbeddedUrl(url.to_string()),
            1920,
        )
        .expect("embedded cover");
        let parsed = reqwest::Url::parse(&got).expect("valid cover URL");

        assert_eq!(
            parsed
                .query_pairs()
                .find_map(|(key, value)| (key == "size").then(|| value.into_owned())),
            Some("1920".to_string())
        );
        assert!(
            parsed
                .query_pairs()
                .any(|(key, value)| key == "t" && value == "token")
        );
        assert!(
            parsed
                .query_pairs()
                .any(|(key, value)| key == "id" && value == "cover-1")
        );
    }

    #[test]
    fn local_artwork_uses_the_shared_cached_protocol() {
        let path = Path::new("/music/album/cover.png");
        for max_width in [80, 1400] {
            let cover = from_path(&local_active(), Some(path), max_width).expect("local cover");
            assert!(
                cover.starts_with("artwork://")
                    || cover.starts_with("http://artwork.dioxus.localhost/"),
                "local cover must use the artwork protocol: {cover}"
            );
        }
    }
}
