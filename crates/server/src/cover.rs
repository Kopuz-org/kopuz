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

use std::path::Path;

use config::{AppConfig, MusicService, Source};
use reader::{ArtistImageRef, Track};
use utils::CoverUrl;

/// Resolve a cover from a stored cover-path ref — album covers and artist-grid
/// images, where the ref is a filesystem path (local) or a remote image path /
/// `directurl:` form (a server). `max_width` sizes the request.
pub fn from_path(
    config: &AppConfig,
    cover_path: Option<&Path>,
    max_width: u32,
) -> Option<CoverUrl> {
    match &config.active_source {
        Source::Local => {
            let owned = cover_path.map(Path::to_path_buf);
            utils::format_artwork_thumb_url(owned.as_ref(), max_width)
        }
        Source::Server(_) => {
            let server = config.server.as_ref()?;
            let path = cover_path?;
            utils::map_cover_url(utils::jellyfin_image::jellyfin_image_url_from_path(
                &path.to_string_lossy(),
                &server.url,
                server.access_token.as_deref(),
                max_width,
                80,
            ))
        }
    }
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
/// while Local is active — still resolves correctly. A local track uses its album
/// art (the caller passes the album cover-path it has in hand); a server track
/// uses the per-service remote form (Jellyfin/Subsonic image endpoints, YT's
/// thumbnail URL), built against the configured server's creds.
pub fn track(
    config: &AppConfig,
    track: &Track,
    album_cover_path: Option<&Path>,
    max_width: u32,
) -> Option<CoverUrl> {
    let Some(service) = track.id.service() else {
        // Local track → its album art as a sized asset.
        let owned = album_cover_path.map(Path::to_path_buf);
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
        // SoundCloud stores the artwork URL directly in `cover` — no encoding.
        MusicService::SoundCloud => track.cover.clone(),
    };
    utils::map_cover_url(url)
}
