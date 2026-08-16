use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::auth;

const LICENSE_SERVER_URL: &str =
    "https://play.itunes.apple.com/WebObjects/MZPlay.woa/wa/acquireWebPlaybackLicense";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug)]
pub struct WebPlaybackInfo {
    pub file_url: String,
    pub kid_base64: String,
    pub uri_prefix: String,
}

/// Calls the Apple Music web playback API and extracts the audio stream info.
pub async fn get_web_playback(
    adam_id: &str,
    bearer_token: &str,
    media_user_token: &str,
) -> Result<WebPlaybackInfo, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "salableAdamId": adam_id });

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

    // Log all available assets
    let assets = song["assets"].as_array().ok_or("no assets")?;
    for asset in assets {
        tracing::debug!(
            "am.webplayback: asset flavor={} url={}",
            asset["flavor"].as_str().unwrap_or("?"),
            asset["URL"].as_str().unwrap_or("?"),
        );
    }

    // Find the audio asset — only 28:ctrp256 (CTR-encrypted) works with our Widevine CDM.
    // cbcp flavors use Apple's proprietary skd:// key delivery which our CDM can't handle.
    let asset_url = assets
        .iter()
        .find(|a| a["flavor"].as_str() == Some("28:ctrp256"))
        .and_then(|a| a["URL"].as_str())
        .ok_or("no 28:ctrp256 asset found")?
        .to_string();

    tracing::debug!("am.webplayback: asset URL found, extracting KID");

    // Fetch the asset URL as M3U8 to extract the KID
    let m3u8_resp = client
        .get(&asset_url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("fetch M3U8: {e}"))?;

    let m3u8_body = m3u8_resp
        .text()
        .await
        .map_err(|e| format!("read M3U8: {e}"))?;

    let (_, media_playlist) = m3u8_rs::parse_media_playlist(m3u8_body.as_bytes())
        .map_err(|e| format!("parse M3U8: {e}"))?;

    // Extract KID from the KEY URI (format: "uriPrefix,kidBase64")
    let key_uri = media_playlist
        .segments
        .first()
        .and_then(|s| s.key.as_ref())
        .and_then(|k| k.uri.as_deref())
        .ok_or("no KEY in media playlist")?;

    tracing::debug!("am.webplayback: raw KEY URI = {key_uri}");

    let (uri_prefix, kid_base64) = key_uri
        .split_once(',')
        .ok_or("KEY URI not in expected format 'prefix,kid'")?;

    tracing::debug!("am.webplayback: uri_prefix = {uri_prefix}, kid = {kid_base64}");

    tracing::debug!("am.webplayback: KID extracted, uri_prefix present");

    // Build the file download URL from the MAP URI
    let base_url = asset_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(&asset_url);

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

    Ok(WebPlaybackInfo {
        file_url,
        kid_base64: kid_base64.to_string(),
        uri_prefix: uri_prefix.to_string(),
    })
}

/// Exchange the CDM's challenge for a license and load it back into the CDM.
///
/// Nothing is returned: the content key stays sealed inside the CDM, which is
/// the point of driving the real one rather than a hand-rolled protocol.
async fn load_license(
    cdm: &super::widevine::Cdm,
    license_request: &[u8],
    adam_id: &str,
    uri_prefix: &str,
    kid_base64: &str,
    bearer_token: &str,
    media_user_token: &str,
) -> Result<(), String> {
    let envelope = serde_json::json!({
        "challenge": STANDARD.encode(license_request),
        "key-system": "com.widevine.alpha",
        "uri": format!("{uri_prefix},{kid_base64}"),
        "adamId": adam_id,
        "isLibrary": false,
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

    cdm.update(&license_data).map_err(|e| {
        tracing::warn!("am.license: loading the license failed: {e}");
        e
    })?;

    tracing::debug!("am.license: keys loaded");
    Ok(())
}

/// Full pipeline: resolve + download + decrypt. Returns decrypted fMP4 bytes.
pub async fn resolve_and_decrypt(
    adam_id: &str,
    media_user_token: &str,
    storefront: &str,
    language: &str,
) -> Result<Vec<u8>, String> {
    let bearer_token = auth::get_bearer_token().await?;
    // Resolve the id to a catalog Adam id if needed (library ids don't work with web playback)
    let api = crate::applemusic::AppleMusicApi::new(
        Some(media_user_token.to_string()),
        storefront,
        language,
    );
    let adam_id = api.resolve_catalog_id(adam_id).await?;

    tracing::info!("am.stream: resolving web playback for adam_id={adam_id}");

    let playback = get_web_playback(&adam_id, &bearer_token, media_user_token).await?;

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
    let cdm = super::widevine::Cdm::open_system()?;
    let license_request = cdm.challenge(&init_data)?;
    tracing::debug!(
        "am.stream: license challenge generated ({} bytes)",
        license_request.len()
    );

    tracing::debug!("am.stream: exchanging license with Apple");

    load_license(
        &cdm,
        &license_request,
        &adam_id,
        &playback.uri_prefix,
        &playback.kid_base64,
        &bearer_token,
        media_user_token,
    )
    .await?;

    tracing::info!(
        "am.stream: downloading encrypted fMP4 from {}",
        playback.file_url
    );

    let client = reqwest::Client::new();
    let encrypted_resp = client
        .get(&playback.file_url)
        .header("User-Agent", USER_AGENT)
        .header("x-apple-music-user-token", media_user_token)
        .header("Cookie", format!("media-user-token={media_user_token}"))
        .send()
        .await
        .map_err(|e| format!("download fMP4: {e}"))?;

    let status = encrypted_resp.status();
    if !status.is_success() {
        return Err(format!("download fMP4 HTTP {status}"));
    }

    let encrypted_bytes = encrypted_resp
        .bytes()
        .await
        .map_err(|e| format!("read fMP4 bytes: {e}"))?;

    tracing::info!(
        "am.stream: downloaded {} bytes, decrypting through the CDM",
        encrypted_bytes.len()
    );

    let decrypted = crate::applemusic::cenc::decrypt_fmp4(&encrypted_bytes, &cdm, &key_id)?;

    // Save decrypted output for testing
    let tmp_dir = std::env::temp_dir().join("kopuz_am_decrypt");
    let _ = tokio::fs::create_dir_all(&tmp_dir).await;
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out_path = tmp_dir.join(format!("decrypted_{id}.m4a"));
    let _ = tokio::fs::write(&out_path, &decrypted).await;
    tracing::info!(
        "am.stream: decrypted {} bytes → {}",
        decrypted.len(),
        out_path.display()
    );

    Ok(decrypted)
}
