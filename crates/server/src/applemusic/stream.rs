use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::auth;

const LICENSE_SERVER_URL: &str =
    "https://play.itunes.apple.com/WebObjects/MZPlay.woa/wa/acquireWebPlaybackLicense";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// The only catalog flavour our Widevine CDM can open: `cbcp` flavours use
/// Apple's own `skd://` key delivery, which it can't speak.
const CTR_FLAVOR: &str = "28:ctrp256";

#[derive(Debug)]
pub struct WebPlaybackInfo {
    pub file_url: String,
    pub kid_base64: String,
    pub uri_prefix: String,
}

/// What `webPlayback` handed back for a track.
#[derive(Debug)]
pub enum WebPlayback {
    /// Widevine-encrypted fMP4 — needs a licence before a byte can be read.
    Encrypted(WebPlaybackInfo),
    /// A plain file. An uploaded iCloud Music Library track is the user's own
    /// audio, so Apple has no content key for it and there is nothing to unwrap.
    Plain { file_url: String },
}

/// True for the ids iCloud Music Library hands out (`i.`/`l.`/`a.`/`p.` and an
/// alphanumeric tail), as opposed to a numeric catalog Adam ID. Same test
/// MusicKit applies before choosing how to dispatch playback.
fn is_library_id(id: &str) -> bool {
    let mut chars = id.chars();
    if !matches!(chars.next(), Some('a' | 'i' | 'l' | 'p')) || chars.next() != Some('.') {
        return false;
    }
    let tail = &id[2..];
    !tail.is_empty() && tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The `webPlayback` request body.
///
/// A library item is dispatched by its library id; only catalog items have
/// something *salable* to look up, which is why posting a library id as
/// `salableAdamId` came back 200 with an empty `songList` and no error. Apple's
/// client also sends `subscriptionAdamId` here, but that is the catalog id — and
/// if we had one, `resolve_catalog_id` would already have swapped it in and we
/// would be on the salable path instead.
fn web_playback_body(id: &str) -> serde_json::Value {
    if is_library_id(id) {
        serde_json::json!({ "universalLibraryId": id })
    } else {
        serde_json::json!({ "salableAdamId": id })
    }
}

/// The asset to play out of a `songList` entry.
#[derive(Debug)]
enum Asset {
    /// A catalog encode, named by flavour: an M3U8 whose KEY carries the KID.
    Flavored(String),
    /// An uploaded track: a single asset with no flavour at all, pointing
    /// straight at the file the user uploaded.
    Uploaded(String),
}

/// Pick the asset to play, following the three shapes MusicKit distinguishes.
fn select_asset(song: &serde_json::Value) -> Result<Asset, String> {
    if song["hls-playlist-url"]
        .as_str()
        .is_some_and(|url| !url.is_empty())
    {
        return Err("track is served as an HLS playlist, which kopuz can't play yet".to_string());
    }

    let assets = song["assets"].as_array().ok_or("no assets")?;

    // A lone flavourless asset is an upload rather than one of Apple's encodes.
    if let [only] = assets.as_slice()
        && only["flavor"].as_str().is_none()
        && let Some(url) = only["URL"].as_str().filter(|u| !u.is_empty())
    {
        return Ok(Asset::Uploaded(url.to_string()));
    }

    assets
        .iter()
        .find(|a| a["flavor"].as_str() == Some(CTR_FLAVOR))
        .and_then(|a| a["URL"].as_str())
        .map(|url| Asset::Flavored(url.to_string()))
        .ok_or_else(|| format!("no {CTR_FLAVOR} asset found"))
}

/// Calls the Apple Music web playback API and extracts the audio stream info.
pub async fn get_web_playback(
    adam_id: &str,
    bearer_token: &str,
    media_user_token: &str,
) -> Result<WebPlayback, String> {
    let client = reqwest::Client::new();
    let body = web_playback_body(adam_id);

    let resp = client
        .post("https://play.music.apple.com/WebObjects/MZPlay.woa/wa/webPlayback")
        .header("Content-Type", "application/json")
        .header("Origin", "https://music.apple.com")
        .header("User-Agent", USER_AGENT)
        .header("Referer", "https://music.apple.com/")
        .header("Authorization", format!("Bearer {bearer_token}"))
        .header("x-apple-music-user-token", media_user_token)
        .header("Cookie", format!("media-user-token={media_user_token}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("webPlayback request: {e}"))?;

    let status = resp.status();
    tracing::info!("am.webplayback: HTTP {status}");
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("webPlayback HTTP {status}: {text}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("parse webPlayback: {e}"))?;

    let song_list = json["songList"]
        .as_array()
        .ok_or("no songList in response")?;

    if song_list.is_empty() {
        return Err("empty songList".to_string());
    }

    let song = &song_list[0];

    if let Some(assets) = song["assets"].as_array() {
        for asset in assets {
            tracing::debug!(
                "am.webplayback: asset flavor={} url={}",
                asset["flavor"].as_str().unwrap_or("none"),
                asset["URL"].as_str().unwrap_or("?"),
            );
        }
    }

    match select_asset(song)? {
        Asset::Flavored(url) => {
            tracing::debug!("am.webplayback: {CTR_FLAVOR} asset found, extracting KID");
            let info = read_encrypted_playlist(&client, &url)
                .await?
                .ok_or("catalog asset carried no Widevine KEY")?;
            Ok(WebPlayback::Encrypted(info))
        }
        // Uploaded audio has no encode behind it, so it may be the file itself
        // rather than a playlist. Try to read it as one; if there's no KEY there
        // is no DRM to unwrap and the URL is already what we want to play.
        Asset::Uploaded(url) => match read_encrypted_playlist(&client, &url).await? {
            Some(info) => {
                tracing::info!("am.webplayback: uploaded asset is encrypted");
                Ok(WebPlayback::Encrypted(info))
            }
            None => {
                tracing::info!("am.webplayback: uploaded asset is a plain file");
                Ok(WebPlayback::Plain { file_url: url })
            }
        },
    }
}

/// Read the M3U8 at `asset_url` and pull out the Widevine KID plus the fMP4 its
/// segments map to.
///
/// `Ok(None)` means it simply isn't an encrypted media playlist — an unreadable
/// playlist, or one with no KEY. Only a transport failure is an error.
async fn read_encrypted_playlist(
    client: &reqwest::Client,
    asset_url: &str,
) -> Result<Option<WebPlaybackInfo>, String> {
    let m3u8_resp = client
        .get(asset_url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("fetch M3U8: {e}"))?;

    let m3u8_body = m3u8_resp
        .text()
        .await
        .map_err(|e| format!("read M3U8: {e}"))?;

    let Ok((_, media_playlist)) = m3u8_rs::parse_media_playlist(m3u8_body.as_bytes()) else {
        tracing::debug!("am.webplayback: asset is not a media playlist");
        return Ok(None);
    };

    // KEY URI is "uriPrefix,kidBase64".
    let key_uri = media_playlist
        .segments
        .first()
        .and_then(|s| s.key.as_ref())
        .and_then(|k| k.uri.as_deref());
    let Some((uri_prefix, kid_base64)) = key_uri.and_then(|uri| uri.split_once(',')) else {
        tracing::debug!("am.webplayback: media playlist carries no KEY");
        return Ok(None);
    };

    tracing::debug!("am.webplayback: uri_prefix = {uri_prefix}, kid = {kid_base64}");

    // Build the file download URL from the MAP URI
    let base_url = asset_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(asset_url);

    let map_uri = media_playlist
        .segments
        .first()
        .and_then(|s| s.map.as_ref())
        .map(|m| m.uri.as_str())
        .unwrap_or("");

    let file_url = if map_uri.starts_with("http") {
        map_uri.to_string()
    } else {
        format!("{base_url}/{map_uri}")
    };

    Ok(Some(WebPlaybackInfo {
        file_url,
        kid_base64: kid_base64.to_string(),
        uri_prefix: uri_prefix.to_string(),
    }))
}

/// Exchange the CDM's challenge for a license and load it back into the CDM.
///
/// Nothing is returned: the content key stays sealed inside the CDM, which is
/// the point of driving the real one rather than a hand-rolled protocol.
async fn load_license(
    cdm: &super::widevine::Cdm,
    session: &super::widevine::LicenseSession,
    cdm_session: &super::widevine::CdmSession,
    license_request: &[u8],
    adam_id: &str,
    playback: &WebPlaybackInfo,
    bearer_token: &str,
    media_user_token: &str,
) -> Result<(), String> {
    let envelope = serde_json::json!({
        "challenge": STANDARD.encode(license_request),
        "key-system": "com.widevine.alpha",
        "uri": format!("{},{}", playback.uri_prefix, playback.kid_base64),
        "adamId": adam_id,
        "isLibrary": is_library_id(adam_id),
        "user-initiated": true,
    });

    tracing::debug!(
        "am.license: sending envelope (challenge_b64_len={}, uri={})",
        envelope["challenge"].as_str().unwrap_or("").len(),
        envelope["uri"].as_str().unwrap_or("")
    );
    tracing::debug!(
        "am.license: full envelope: {}",
        serde_json::to_string(&envelope).unwrap_or_default()
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(LICENSE_SERVER_URL)
        .header("Content-Type", "application/json")
        .header("Origin", "https://music.apple.com")
        .header("User-Agent", USER_AGENT)
        .header("Referer", "https://music.apple.com/")
        .header("Authorization", format!("Bearer {bearer_token}"))
        .header("x-apple-music-user-token", media_user_token)
        .header("Cookie", format!("media-user-token={media_user_token}"))
        .json(&envelope)
        .send()
        .await
        .map_err(|e| format!("license request: {e}"))?;

    let status = resp.status();
    tracing::info!("am.license: HTTP {status}");
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        tracing::warn!("am.license: error body: {text}");
        return Err(format!("license HTTP {status}: {text}"));
    }

    let resp_body = resp
        .text()
        .await
        .map_err(|e| format!("read license body: {e}"))?;
    tracing::debug!(
        "am.license: raw response len={} body: {}",
        resp_body.len(),
        &resp_body[..resp_body.len().min(500)]
    );

    let license_json: serde_json::Value = serde_json::from_str(&resp_body).map_err(|e| {
        tracing::warn!("am.license: parse license failed: {e}");
        format!("parse license: {e}")
    })?;

    if let Some(obj) = license_json.as_object() {
        tracing::debug!(
            "am.license: response keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    if let Some(err_code) = license_json["errorCode"].as_i64()
        && err_code != 0
    {
        // Apple answers 200 with the failure in the body, and these codes aren't
        // documented anywhere — so log the whole thing rather than just the
        // number, which on its own says nothing about what to fix.
        tracing::warn!(
            "am.license: rejected with errorCode {err_code} for adamId={adam_id} \
             (isLibrary={}), body: {resp_body}",
            is_library_id(adam_id)
        );
        return Err(format!("license error code: {err_code}"));
    }

    let license_b64 = license_json["license"]
        .as_str()
        .ok_or("no license in response")?;

    tracing::debug!("am.license: license b64 len={}", license_b64.len());

    let license_data = STANDARD
        .decode(license_b64)
        .map_err(|e| format!("decode license: {e}"))?;

    tracing::debug!(
        "am.license: license binary len={}, loading into CDM",
        license_data.len()
    );

    cdm.update(session, cdm_session, &license_data)
        .map_err(|e| {
            tracing::warn!("am.license: loading the license failed: {e}");
            e
        })?;

    tracing::debug!("am.license: keys loaded");
    Ok(())
}

/// Full pipeline: resolve + download + decrypt. Returns decrypted fMP4 bytes.
/// Resolve a track to a decrypted, readable stream.
///
/// Returns as soon as the licence is loaded and the ciphertext is in hand — the
/// CDM work then runs behind the returned [`ProgressiveTrack`], so playback can
/// start on the first fragment instead of waiting out the whole track.
pub async fn resolve_and_decrypt(
    adam_id: &str,
    media_user_token: &str,
    storefront: &str,
    language: &str,
    progress: Option<utils::stream_buffer::BufferProgressCallback>,
) -> Result<super::progressive::ProgressiveTrack, String> {
    let bearer_token = auth::get_bearer_token().await?;
    // Resolve the id to a catalog Adam id if needed (library ids don't work with web playback)
    let api = crate::applemusic::AppleMusicApi::new(
        Some(media_user_token.to_string()),
        storefront,
        language,
    );
    let adam_id = api.resolve_catalog_id(adam_id).await?;
    let cache_path = decrypted_cache_path(&adam_id);
    if let Ok(cached) = tokio::fs::read(&cache_path).await
        && !cached.is_empty()
    {
        tracing::info!(
            "am.stream: reusing decrypted {} ({} bytes)",
            cache_path.display(),
            cached.len()
        );
        if let Some(p) = &progress {
            let total = cached.len() as u64;
            p(0, total, Some(total));
        }
        return Ok(super::progressive::ProgressiveTrack::ready(cached));
    }

    tracing::info!("am.stream: resolving web playback for adam_id={adam_id}");

    let playback = match get_web_playback(&adam_id, &bearer_token, media_user_token).await? {
        WebPlayback::Encrypted(info) => info,
        // An uploaded library track: the user's own audio, no licence to fetch
        // and nothing to decrypt. Cache it like any other so a replay is local.
        WebPlayback::Plain { file_url } => {
            tracing::info!("am.stream: downloading unencrypted library asset from {file_url}");
            let bytes = download_asset(&file_url, media_user_token).await?;
            tracing::info!("am.stream: downloaded {} bytes, no DRM", bytes.len());
            if let Some(p) = &progress {
                let total = bytes.len() as u64;
                p(0, total, Some(total));
            }
            store_decrypted_blocking(&cache_path, &bytes);
            return Ok(super::progressive::ProgressiveTrack::ready(bytes));
        }
    };

    let key_id = STANDARD
        .decode(&playback.kid_base64)
        .map_err(|e| format!("decode KID: {e}"))?;
    let init_data = super::widevine::build_pssh(&key_id);
    tracing::debug!(
        "am.stream: pssh box built ({} bytes) for kid={}",
        init_data.len(),
        playback.kid_base64
    );

    // Borrow the CDM from an installed browser. Its device key stays sealed, so
    // no key material ships with kopuz.
    let cdm = super::widevine::Cdm::open_system().await?;
    // Held only for challenge → licence → update. Decryption runs without it, so
    // a track already playing never blocks the next one from starting.
    let license = cdm.begin_license().await;
    // The session outlives the licence exchange: it holds the content keys, so it
    // travels with the track and is closed when the track is done with it.
    let (license_request, cdm_session) = cdm.challenge(&license, &init_data)?;
    tracing::debug!(
        "am.stream: license challenge generated ({} bytes)",
        license_request.len()
    );

    tracing::debug!("am.stream: exchanging license with Apple");

    load_license(
        &cdm,
        &license,
        &cdm_session,
        &license_request,
        &adam_id,
        &playback,
        &bearer_token,
        media_user_token,
    )
    .await?;

    drop(license);

    tracing::info!(
        "am.stream: downloading encrypted fMP4 from {}",
        playback.file_url
    );

    let encrypted_bytes = download_asset(&playback.file_url, media_user_token).await?;

    tracing::info!(
        "am.stream: downloaded {} bytes, decrypting through the CDM",
        encrypted_bytes.len()
    );

    super::progressive::ProgressiveTrack::spawn(
        encrypted_bytes,
        cdm,
        cdm_session,
        key_id,
        progress,
        move |decrypted| {
            store_decrypted_blocking(&cache_path, &decrypted);
        },
    )
}

/// Download a track asset. Carries the user token: library assets are the
/// user's own files and are served only to them.
async fn download_asset(url: &str, media_user_token: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("x-apple-music-user-token", media_user_token)
        .header("Cookie", format!("media-user-token={media_user_token}"))
        .send()
        .await
        .map_err(|e| format!("download asset: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("download asset HTTP {status}"));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("read asset bytes: {e}"))
}

/// Where a track's decrypted audio is cached, keyed by catalog id so a replay
/// reuses it.
///
/// The user cache dir, not the temp dir: a decrypted track is several MB and
/// `/tmp` is tmpfs (RAM) on most Linux systems, so caching whole tracks there
/// spends memory to save disk. Small artefacts like cover thumbnails are fine in
/// temp; these are not.
fn decrypted_cache_path(adam_id: &str) -> std::path::PathBuf {
    let safe: String = adam_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|dirs| dirs.cache_dir().join("applemusic"))
        .unwrap_or_else(|| std::env::temp_dir().join("kopuz-applemusic"))
        .join(format!("{safe}.m4a"))
}

/// Publish `bytes` at `path` via a temp file + rename, so a crash mid-write
/// can't leave a truncated file that later reads back as a valid cache hit.
///
/// Blocking: it runs on the decrypt thread once the last fragment lands.
fn store_decrypted_blocking(path: &std::path::Path, bytes: &[u8]) {
    let Some(dir) = path.parent() else { return };
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::debug!("am.stream: cache dir {}: {e}", dir.display());
        return;
    }
    let staging = path.with_extension("part");
    if let Err(e) = std::fs::write(&staging, bytes) {
        tracing::debug!("am.stream: cache write {}: {e}", staging.display());
        return;
    }
    if let Err(e) = std::fs::rename(&staging, path) {
        tracing::debug!("am.stream: cache publish {}: {e}", path.display());
        let _ = std::fs::remove_file(&staging);
    } else {
        tracing::info!(
            "am.stream: cached {} bytes → {}",
            bytes.len(),
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_ids_are_told_apart_from_catalog_ids() {
        // The id that sent us down the wrong dispatch.
        assert!(is_library_id("i.ZOMr5KaurEbG7lz"));
        assert!(is_library_id("l.abc123"));
        assert!(is_library_id("p.playlist-1"));
        assert!(is_library_id("a.Album2"));

        assert!(!is_library_id("1811922756"), "catalog Adam IDs are numeric");
        assert!(!is_library_id("i."), "a prefix alone is not an id");
        assert!(!is_library_id("x.abc"), "only a/i/l/p prefix the library");
        assert!(!is_library_id(""));
        assert!(!is_library_id("i.has_underscore"));
    }

    /// Posting a library id as `salableAdamId` is what produced a 200 with no
    /// `songList` — the store has nothing to sell for a track you uploaded.
    #[test]
    fn a_library_track_is_dispatched_by_its_library_id() {
        assert_eq!(
            web_playback_body("i.ZOMr5KaurEbG7lz"),
            serde_json::json!({ "universalLibraryId": "i.ZOMr5KaurEbG7lz" })
        );
        assert_eq!(
            web_playback_body("1811922756"),
            serde_json::json!({ "salableAdamId": "1811922756" })
        );
    }

    /// An uploaded track comes back as a single asset with no `flavor` field at
    /// all, so the flavour search that serves catalog tracks skips right past it.
    #[test]
    fn a_lone_flavourless_asset_is_an_upload() {
        let song = serde_json::json!({
            "assets": [{ "URL": "https://example.com/uploaded.m4a" }]
        });
        match select_asset(&song) {
            Ok(Asset::Uploaded(url)) => assert_eq!(url, "https://example.com/uploaded.m4a"),
            Ok(Asset::Flavored(_)) => panic!("an upload has no flavour to match"),
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn a_catalog_song_picks_the_ctr_flavour() {
        let song = serde_json::json!({
            "assets": [
                { "flavor": "32:cbcp64", "URL": "https://example.com/cbcp.m3u8" },
                { "flavor": CTR_FLAVOR, "URL": "https://example.com/ctr.m3u8" },
            ]
        });
        match select_asset(&song) {
            Ok(Asset::Flavored(url)) => assert_eq!(url, "https://example.com/ctr.m3u8"),
            Ok(Asset::Uploaded(_)) => panic!("a multi-asset encode is not an upload"),
            Err(e) => panic!("{e}"),
        }
    }

    /// A single *flavoured* asset is still an encode, not an upload — the
    /// upload test is the absence of a flavour, not the count alone.
    #[test]
    fn a_lone_flavoured_asset_is_not_an_upload() {
        let song = serde_json::json!({
            "assets": [{ "flavor": CTR_FLAVOR, "URL": "https://example.com/ctr.m3u8" }]
        });
        assert!(matches!(select_asset(&song), Ok(Asset::Flavored(_))));
    }

    #[test]
    fn an_hls_playlist_is_reported_rather_than_mis_parsed() {
        let song = serde_json::json!({
            "hls-playlist-url": "https://example.com/playlist.m3u8",
            "assets": [],
        });
        let err = select_asset(&song).expect_err("HLS playlists are unsupported");
        assert!(err.contains("HLS playlist"), "{err}");
    }

    #[test]
    fn a_song_with_no_playable_asset_is_an_error() {
        let song = serde_json::json!({
            "assets": [{ "flavor": "32:cbcp64", "URL": "https://example.com/cbcp.m3u8" }]
        });
        assert!(select_asset(&song).is_err());
    }
}
