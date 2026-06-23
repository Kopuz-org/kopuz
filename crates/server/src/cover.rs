//! Source-agnostic cover resolution (issue #347 / #35).
//!
//! The UI calls these instead of branching on local-file-vs-remote-URL or
//! `match service` per row: the source layer owns where a cover *lives* and how
//! to turn it into a renderable URL. Local resolves the on-disk file to a sized
//! `artwork://` asset; a server resolves its remote image URL (per service).
//!
//! These are sync free functions, not [`MediaSource`](crate::source::MediaSource)
//! methods, because they run per-row in long lists — they must not allocate a
//! `Box<dyn>` per cover. Capabilities are a trait method (resolved once); cover
//! resolution is a hot, allocation-light function keyed on the config + service.

use std::path::{Path, PathBuf};

use config::{AppConfig, MusicService};
use reader::{ArtistImageRef, Track};
use utils::CoverUrl;

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
    let path = cover_path?;
    if path.is_absolute() {
        return utils::format_artwork_thumb_url(Some(&path.to_path_buf()), max_width);
    }
    // A `urlhex_`/`directurl:` ref carries the full image URL and resolves with no
    // server; a bare service id needs the active server's base URL + token (absent
    // → `None`, a clean placeholder rather than a broken request).
    let (server_url, token) = config
        .server
        .as_ref()
        .map(|s| (s.url.as_str(), s.access_token.as_deref()))
        .unwrap_or(("", None));
    utils::map_cover_url(utils::jellyfin_image::jellyfin_image_url_from_path(
        &path.to_string_lossy(),
        server_url,
        token,
        max_width,
        80,
    ))
}

/// Resolve one artist's photo, source-agnostic. Priority: a custom `override_path`
/// (always), then — when `use_photo` is set — the synced `photo`, with a server
/// photo outranking a freshly-`fetched_url` outranking a local file, then the
/// album cover. The UI passes the candidates and never branches on
/// local-vs-remote — that resolution lives here.
pub fn artist(
    config: &AppConfig,
    override_path: Option<&Path>,
    photo: Option<&ArtistImageRef>,
    fetched_url: Option<&str>,
    album_cover_path: Option<&Path>,
    use_photo: bool,
    max_width: u32,
) -> Option<CoverUrl> {
    let override_owned = override_path.map(Path::to_path_buf);
    if let Some(cover) = utils::format_artwork_url(override_owned.as_ref()) {
        return Some(cover);
    }
    if use_photo {
        let resolved = match photo {
            Some(ArtistImageRef::Remote(url)) => Some(utils::cover_url_from_string(url.clone())),
            other => fetched_url
                .map(|u| utils::cover_url_from_string(u.to_string()))
                .or_else(|| match other {
                    Some(ArtistImageRef::Local(path)) => utils::format_artwork_url(Some(path)),
                    _ => None,
                }),
        };
        if resolved.is_some() {
            return resolved;
        }
    }
    from_path(config, album_cover_path, max_width)
}

/// Resolve a track's cover, dispatching on the **track's own source** (not the
/// active source) so a mixed list — e.g. a server track in the now-playing queue
/// while Local is active — still resolves correctly. Every track self-describes
/// its cover via `track.cover`: a local row's `cover_path` is projected from its
/// album by the DB read layer (so it's a filesystem path), a server row carries
/// the per-service remote ref. No caller-side album lookup.
pub fn track(config: &AppConfig, track: &Track, max_width: u32) -> Option<CoverUrl> {
    let Some(service) = track.id.service() else {
        // Local track → its (album) art file as a sized asset.
        let owned = track.cover.as_deref().map(PathBuf::from);
        return utils::format_artwork_thumb_url(owned.as_ref(), max_width);
    };
    let server = config.server.as_ref()?;
    let url = match service {
        MusicService::Jellyfin => utils::jellyfin_image::resolve_track_cover(
            track.cover.as_deref(),
            &track.id.key(),
            &track.album_id,
            &server.url,
            server.access_token.as_deref(),
            max_width,
            80,
        ),
        MusicService::Subsonic | MusicService::Custom => {
            let subsonic_path = match track.cover.as_deref() {
                Some(c) => format!("{}:{}", track.id.uid(), c),
                None => track.id.uid(),
            };
            utils::subsonic_image::subsonic_image_url_from_path(
                &subsonic_path,
                &server.url,
                server.access_token.as_deref(),
                max_width,
                80,
            )
        }
        MusicService::YtMusic => utils::jellyfin_image::resolve_track_cover(
            track.cover.as_deref(),
            &track.id.key(),
            &track.album_id,
            "",
            None,
            max_width,
            80,
        ),
        // SoundCloud and Spotify store the artwork URL directly in `cover`.
        MusicService::SoundCloud | MusicService::Spotify => track.cover.clone(),
    };
    utils::map_cover_url(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn local_active() -> AppConfig {
        AppConfig {
            active_source: config::Source::Local,
            server: None,
            ..Default::default()
        }
    }

    #[test]
    fn from_path_resolves_a_remote_ref_while_local_is_active() {
        // The regression: one frame after switching away from YT, its album covers
        // (`ytmusic:_:urlhex_<url>`) are still rendered. With Local active they must
        // resolve to the embedded URL — NOT get fed to the local artwork:// path as
        // a filename (the artwork server would open() it → ENAMETOOLONG).
        let url = "https://example.com/cover.jpg";
        let reff = format!("ytmusic:_:{}", utils::jellyfin_image::encode_cover_url(url));
        let got = from_path(&local_active(), Some(Path::new(&reff)), 200).expect("resolves");
        assert_eq!(
            &*got, url,
            "self-contained remote ref → its URL, not artwork://"
        );
    }
}
