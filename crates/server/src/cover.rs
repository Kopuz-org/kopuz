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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use config::{AppConfig, MusicService};
use reader::{ArtistImageRef, CoverRef, Track};
use utils::CoverUrl;

use crate::source::ArtistView;

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

/// Session-scoped artist-photo fetch outcomes, keyed by DISPLAY name (the DB
/// caches are keyed by the normalized name — [`ArtistArt::from_caches`] bridges
/// the two). A newtype so every state is constructed through a method — the old
/// `HashMap<String, String>` encoded "resolved, no photo" as an `""` sentinel
/// that leaked into `img src` at one call site.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FetchedArtistImages(HashMap<String, FetchEntry>);

#[derive(Debug, Clone, PartialEq)]
enum FetchEntry {
    Pending,
    Hit(String),
    Miss,
}

/// One artist's position in the photo-fetch pipeline, as [`artist`] consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistFetchState<'a> {
    /// No fetch is planned for this name (a source with no photo fetcher, or a
    /// name the fetch loop skips) — resolution proceeds to the last resort.
    NotFetching,
    /// Queued or in flight — show a placeholder rather than guess and swap.
    Pending,
    /// The fetch concluded: a photo URL, or a definitive miss.
    Resolved(Option<&'a str>),
}

impl FetchedArtistImages {
    pub fn state(&self, display: &str) -> ArtistFetchState<'_> {
        match self.0.get(display) {
            None => ArtistFetchState::NotFetching,
            Some(FetchEntry::Pending) => ArtistFetchState::Pending,
            Some(FetchEntry::Hit(url)) => ArtistFetchState::Resolved(Some(url)),
            Some(FetchEntry::Miss) => ArtistFetchState::Resolved(None),
        }
    }

    /// Whether `display` has an entry at all (pending or resolved) — the fetch
    /// loop's skip-set check.
    pub fn contains(&self, display: &str) -> bool {
        self.0.contains_key(display)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Mark names as queued before the workers start, so their tiles show a
    /// placeholder instead of the last resort they'd otherwise get.
    pub fn mark_pending(&mut self, names: impl IntoIterator<Item = String>) {
        for name in names {
            self.0.entry(name).or_insert(FetchEntry::Pending);
        }
    }

    pub fn insert_hit(&mut self, display: String, url: String) {
        self.0.insert(display, FetchEntry::Hit(url));
    }

    /// Record a definitive "no photo exists" outcome.
    pub fn insert_miss(&mut self, display: String) {
        self.0.insert(display, FetchEntry::Miss);
    }

    /// Replace the whole map with a bulk fetch's findings (Jellyfin/Subsonic).
    pub fn replace_all(&mut self, found: impl IntoIterator<Item = (String, String)>) {
        self.0 = found
            .into_iter()
            .map(|(k, v)| (k, FetchEntry::Hit(v)))
            .collect();
    }
}

/// One artist's image candidates, resolved by [`artist`]. Build via
/// [`from_caches`](Self::from_caches) — the ONE place the normalized-key DB
/// caches and the display-keyed session map meet.
pub struct ArtistArt<'a> {
    /// User-set custom image — wins in every state.
    pub override_path: Option<&'a Path>,
    /// The synced DB photo (server-synced remote URL, or a local folder image).
    pub photo: Option<&'a ArtistImageRef>,
    /// This session's fetch pipeline state for the artist.
    pub fetched: ArtistFetchState<'a>,
    /// The caller's best album-art candidate for the artist (their first album,
    /// else their track's album). Only [`ArtistView::Library`] tiles ever render
    /// it — see [`artist`].
    pub album_cover: Option<&'a Path>,
    /// How the source presents artists — decides whether the album-art last
    /// resort applies.
    pub view: ArtistView,
}

impl<'a> ArtistArt<'a> {
    /// Assemble the candidates for one artist from the app's caches: the DB
    /// image store (keyed by normalized name) and the session fetch map (keyed
    /// by display name).
    pub fn from_caches(
        images: &'a db::ArtistImages,
        fetched: &'a FetchedArtistImages,
        norm: &str,
        display: &str,
        album_cover: Option<&'a Path>,
        view: ArtistView,
    ) -> Self {
        let (overrides, photos) = images;
        Self {
            override_path: overrides.get(norm).map(PathBuf::as_path),
            photo: photos.get(norm),
            fetched: fetched.state(display),
            album_cover,
            view,
        }
    }
}

/// Resolve one artist's image — the SINGLE policy:
///
/// 1. custom override;
/// 2. the source's own photo — DB server photo, then a session-fetched hit,
///    then a local folder image;
/// 3. fetch still pending → placeholder (don't guess and visibly swap);
/// 4. [`Library`](ArtistView::Library) sources: the album-art candidate;
/// 5. placeholder.
///
/// The album-art last resort is what keeps a library grid (whose artists mostly
/// have no photo anywhere) from being a wall of placeholders. A
/// [`Remote`](ArtistView::Remote) catalog never renders it: every artist there
/// resolves to a real photo or a placeholder — its liked-track album covers
/// aren't the artist, and a shared track's cover on every credited artist's
/// tile was the duped-grid bug. Call sites pass candidates and the source's
/// declared view; none of them branches on the service.
pub fn artist(config: &AppConfig, art: ArtistArt<'_>, max_width: u32) -> Option<CoverUrl> {
    let override_owned = art.override_path.map(Path::to_path_buf);
    if let Some(cover) = utils::format_artwork_url(override_owned.as_deref()) {
        return Some(cover);
    }
    if let Some(ArtistImageRef::Remote(url)) = art.photo {
        return Some(utils::cover_url_from_string(url.clone()));
    }
    if let ArtistFetchState::Resolved(Some(url)) = art.fetched {
        return Some(utils::cover_url_from_string(url.to_string()));
    }
    if let Some(ArtistImageRef::Local(path)) = art.photo
        && let Some(cover) = utils::format_artwork_url(Some(path))
    {
        return Some(cover);
    }
    match art.view {
        ArtistView::Library if art.fetched != ArtistFetchState::Pending => {
            from_path(config, art.album_cover, max_width)
        }
        _ => None,
    }
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
            active_source: config::Source::Local,
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
            artists: Vec::new(),
            replay_gain: config::ReplayGainInfo::default(),
        }
    }

    fn subsonic_config(with_creds: bool) -> AppConfig {
        AppConfig {
            active_source: config::Source::Local,
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
            active_source: config::Source::Local,
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

    /// The artist chain, candidate by candidate. `format_artwork_url` is pure
    /// string formatting (no filesystem access), so fake absolute paths work.
    #[test]
    fn artist_chain_resolves_in_priority_order() {
        let cfg = local_active();
        let album = Path::new("/music/band/album/cover.jpg");
        let over = Path::new("/pics/custom.png");
        let remote = ArtistImageRef::Remote("https://yt/photo.jpg".into());
        let local = ArtistImageRef::Local(PathBuf::from("/music/band/artist.jpg"));
        let art = |override_path, photo, fetched, album_cover, view| ArtistArt {
            override_path,
            photo,
            fetched,
            album_cover,
            view,
        };
        let hit = ArtistFetchState::Resolved(Some("https://fetched/p.jpg"));
        let miss = ArtistFetchState::Resolved(None);
        let none = ArtistFetchState::NotFetching;
        let lib = ArtistView::Library;
        let rem = ArtistView::Remote;

        // Override beats everything, in every state, on every view.
        for view in [lib, rem] {
            let got = artist(
                &cfg,
                art(Some(over), Some(&remote), hit, Some(album), view),
                320,
            )
            .unwrap();
            assert!(got.contains("custom.png"));
        }

        // DB server photo > fetched hit.
        let got = artist(&cfg, art(None, Some(&remote), hit, Some(album), lib), 320).unwrap();
        assert_eq!(&*got, "https://yt/photo.jpg");

        // Fetched hit > DB local photo.
        let got = artist(&cfg, art(None, Some(&local), hit, Some(album), lib), 320).unwrap();
        assert_eq!(&*got, "https://fetched/p.jpg");

        // DB local photo without a fetched hit — even while pending.
        let got = artist(
            &cfg,
            art(
                None,
                Some(&local),
                ArtistFetchState::Pending,
                Some(album),
                lib,
            ),
            320,
        )
        .unwrap();
        assert!(got.contains("artist.jpg"));

        // Library: pending blocks the last resort (placeholder, no swap)…
        assert_eq!(
            artist(
                &cfg,
                art(None, None, ArtistFetchState::Pending, Some(album), lib),
                320
            ),
            None
        );
        // …a miss or no fetch falls to the album-art candidate…
        for state in [miss, none] {
            let got = artist(&cfg, art(None, None, state, Some(album), lib), 320).unwrap();
            assert!(got.contains("cover.jpg"));
        }
        // …and no candidates at all → placeholder.
        assert_eq!(artist(&cfg, art(None, None, miss, None, lib), 320), None);

        // Remote catalogs NEVER render album art as an artist: photo or
        // placeholder, whatever the fetch state — its album covers come from
        // liked tracks, and a shared track's cover on every credited artist's
        // tile was the duped-grid bug.
        for state in [miss, none, ArtistFetchState::Pending] {
            assert_eq!(
                artist(&cfg, art(None, None, state, Some(album), rem), 320),
                None
            );
        }
    }

    /// The last resort goes through `from_path`, so a remote (service-encoded)
    /// album ref resolves to its URL, exactly like album tiles do.
    #[test]
    fn artist_last_resort_resolves_remote_album_refs() {
        let url = "https://example.com/album.jpg";
        let reff = format!("ytmusic:_:{}", CoverRef::encode_url(url));
        let got = artist(
            &local_active(),
            ArtistArt {
                override_path: None,
                photo: None,
                fetched: ArtistFetchState::Resolved(None),
                album_cover: Some(Path::new(&reff)),
                view: ArtistView::Library,
            },
            320,
        )
        .unwrap();
        assert_eq!(&*got, url);
    }

    /// `from_caches` bridges the two key spaces: DB caches are normalized-key,
    /// the session map is display-key.
    #[test]
    fn from_caches_bridges_norm_and_display_keys() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("cool&create".to_string(), PathBuf::from("/pics/cc.png"));
        let mut photos = std::collections::HashMap::new();
        photos.insert(
            "cool&create".to_string(),
            ArtistImageRef::Remote("https://p/cc.jpg".into()),
        );
        let images: db::ArtistImages = (overrides, photos);
        let mut fetched = FetchedArtistImages::default();
        fetched.insert_hit("COOL&CREATE".into(), "https://f/cc.jpg".into());

        let art = ArtistArt::from_caches(
            &images,
            &fetched,
            "cool&create",
            "COOL&CREATE",
            None,
            ArtistView::Library,
        );
        assert!(art.override_path.is_some());
        assert!(matches!(art.photo, Some(ArtistImageRef::Remote(_))));
        assert_eq!(
            art.fetched,
            ArtistFetchState::Resolved(Some("https://f/cc.jpg"))
        );

        // Empty caches → nothing fetches, no candidates.
        let empty: db::ArtistImages = Default::default();
        let no_fetch = FetchedArtistImages::default();
        let art = ArtistArt::from_caches(&empty, &no_fetch, "x", "X", None, ArtistView::Library);
        assert!(art.override_path.is_none() && art.photo.is_none());
        assert_eq!(art.fetched, ArtistFetchState::NotFetching);
    }

    #[test]
    fn fetched_map_states() {
        let mut m = FetchedArtistImages::default();
        assert_eq!(m.state("A"), ArtistFetchState::NotFetching);
        m.mark_pending(["A".to_string()]);
        assert_eq!(m.state("A"), ArtistFetchState::Pending);
        assert!(m.contains("A"));
        m.insert_hit("A".into(), "https://u".into());
        assert_eq!(m.state("A"), ArtistFetchState::Resolved(Some("https://u")));
        m.insert_miss("A".into());
        assert_eq!(m.state("A"), ArtistFetchState::Resolved(None));
        // mark_pending never downgrades a resolved entry.
        m.mark_pending(["A".to_string()]);
        assert_eq!(m.state("A"), ArtistFetchState::Resolved(None));
        m.replace_all([("B".to_string(), "https://b".to_string())]);
        assert_eq!(m.state("B"), ArtistFetchState::Resolved(Some("https://b")));
        assert_eq!(
            m.state("A"),
            ArtistFetchState::NotFetching,
            "replace_all resets"
        );
    }
}
