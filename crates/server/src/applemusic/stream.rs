use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::cdm::Cdm;
use super::auth;

const LICENSE_SERVER_URL: &str = "https://play.itunes.apple.com/WebObjects/MZPlay.woa/wa/acquireWebPlaybackLicense";

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

    let json: serde_json::Value = resp.json().await.map_err(|e| format!("parse webPlayback: {e}"))?;

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
        tracing::info!("am.webplayback: asset flavor={} url={}",
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

    tracing::info!("am.webplayback: asset URL found, extracting KID");

    // Fetch the asset URL as M3U8 to extract the KID
    let m3u8_resp = client
        .get(&asset_url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("fetch M3U8: {e}"))?;

    let m3u8_body = m3u8_resp.text().await.map_err(|e| format!("read M3U8: {e}"))?;

    let (_, media_playlist) = m3u8_rs::parse_media_playlist(m3u8_body.as_bytes())
        .map_err(|e| format!("parse M3U8: {e}"))?;

    // Extract KID from the KEY URI (format: "uriPrefix,kidBase64")
    let key_uri = media_playlist
        .segments
        .first()
        .and_then(|s| s.key.as_ref())
        .and_then(|k| k.uri.as_deref())
        .ok_or("no KEY in media playlist")?;

    tracing::info!("am.webplayback: raw KEY URI = {key_uri}");

    let (uri_prefix, kid_base64) = key_uri
        .split_once(',')
        .ok_or("KEY URI not in expected format 'prefix,kid'")?;

    tracing::info!("am.webplayback: uri_prefix = {uri_prefix}, kid = {kid_base64}");

    tracing::info!("am.webplayback: KID extracted, uri_prefix present");

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

/// Builds a Widevine PSSH from the KID (matching Go's getPSSH).
fn build_pssh(kid_base64: &str) -> Result<String, String> {
    let kid = STANDARD
        .decode(kid_base64)
        .map_err(|e| format!("decode KID base64: {e}"))?;

    let content_id_encoded = STANDARD.encode(b"");

    let header = super::cdm::encode_widevine_cenc_header(&kid, &content_id_encoded);

    let mut pssh = b"0123456789abcdef0123456789abcdef".to_vec();
    pssh.extend_from_slice(&header);

    Ok(STANDARD.encode(&pssh))
}

/// Gets the content decryption key via Widevine CDM license exchange.
async fn get_content_key(
    cdm: &super::cdm::Cdm,
    license_request: &[u8],
    adam_id: &str,
    uri_prefix: &str,
    kid_base64: &str,
    bearer_token: &str,
    media_user_token: &str,
) -> Result<(String, Vec<u8>), String> {
    let envelope = serde_json::json!({
        "challenge": STANDARD.encode(license_request),
        "key-system": "com.widevine.alpha",
        "uri": format!("{uri_prefix},{kid_base64}"),
        "adamId": adam_id,
        "isLibrary": false,
        "user-initiated": true,
    });

    tracing::info!("am.license: sending envelope (challenge_b64_len={}, uri={})", envelope["challenge"].as_str().unwrap_or("").len(), envelope["uri"].as_str().unwrap_or(""));
    tracing::debug!("am.license: full envelope: {}", serde_json::to_string(&envelope).unwrap_or_default());

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

    let resp_body = resp.text().await.map_err(|e| format!("read license body: {e}"))?;
    tracing::info!("am.license: raw response len={} body: {}", resp_body.len(), &resp_body[..resp_body.len().min(500)]);

    let license_json: serde_json::Value = serde_json::from_str(&resp_body).map_err(|e| {
        tracing::warn!("am.license: parse license failed: {e}");
        format!("parse license: {e}")
    })?;

    if let Some(obj) = license_json.as_object() {
        tracing::info!("am.license: response keys: {:?}", obj.keys().collect::<Vec<_>>());
    }

    if let Some(err_code) = license_json["errorCode"].as_i64() {
        if err_code != 0 {
            return Err(format!("license error code: {err_code}"));
        }
    }

    let license_b64 = license_json["license"]
        .as_str()
        .ok_or("no license in response")?;

    tracing::info!("am.license: license b64 len={}", license_b64.len());

    let license_data = STANDARD
        .decode(license_b64)
        .map_err(|e| format!("decode license: {e}"))?;

    tracing::info!("am.license: license binary len={}, calling cdm.get_license_keys", license_data.len());

    let keys = cdm.get_license_keys(license_request, &license_data)
        .map_err(|e| {
            tracing::warn!("am.license: get_license_keys failed: {e}");
            e
        })?;

    tracing::info!("am.license: got {} keys from CDM", keys.len());

    for key in &keys {
        if key.key_type == 1 {
            // CONTENT key
            let key_hex = hex::encode(&key.value);
            tracing::info!("am.license: got content key ({} bytes)", key.value.len());
            return Ok((key_hex, key.value.clone()));
        }
    }

    Err("no content key found in license response".to_string())
}

/// If the id looks like a library id (contains "."), resolve it to a catalog Adam id.
/// Library ids like "i.xxx" are not valid for web playback — only numeric Adam ids work.
async fn resolve_adam_id(item_id: &str, bearer_token: &str, media_user_token: &str) -> Result<String, String> {
    if item_id.chars().all(|c| c.is_ascii_digit()) {
        return Ok(item_id.to_string());
    }

    tracing::info!("am.stream: resolving library id {item_id} to catalog Adam id");

    let client = reqwest::Client::new();
    let url = format!(
        "https://amp-api.music.apple.com/v1/me/library/songs/{}/catalog?l=en",
        item_id
    );
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {bearer_token}"))
        .header("User-Agent", USER_AGENT)
        .header("Origin", "https://music.apple.com")
        .header("Referer", "https://music.apple.com/")
        .header("x-apple-music-user-token", media_user_token)
        .header("Cookie", format!("media-user-token={media_user_token}"))
        .send()
        .await
        .map_err(|e| format!("resolve catalog id: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        tracing::warn!("am.stream: catalog resolve failed ({status}), library song may not have a catalog equivalent");
        return Err(format!("library song {} has no catalog equivalent (HTTP {status})", item_id));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| format!("parse catalog response: {e}"))?;

    if let Some(data) = body["data"].as_array() {
        if let Some(first) = data.first() {
            if let Some(id) = first["id"].as_str() {
                tracing::info!("am.stream: resolved to catalog Adam id {id}");
                return Ok(id.to_string());
            }
        }
    }

    tracing::warn!("am.stream: no catalog id found in response");
    Ok(item_id.to_string())
}

/// Full pipeline: resolve + download + decrypt. Returns decrypted fMP4 bytes.
pub async fn resolve_and_decrypt(adam_id: &str, media_user_token: &str) -> Result<Vec<u8>, String> {
    let bearer_token = auth::get_bearer_token().await?;

    // Resolve the id to a catalog Adam id if needed (library ids don't work with web playback)
    let adam_id = resolve_adam_id(adam_id, &bearer_token, media_user_token).await?;

    tracing::info!("am.stream: resolving web playback for adam_id={adam_id}");

    let playback = get_web_playback(&adam_id, &bearer_token, media_user_token).await?;

    tracing::info!("am.stream: building PSSH and CDM license request");

    let pssh = build_pssh(&playback.kid_base64)?;
    tracing::info!("am.stream: PSSH built ({} bytes)", pssh.len());
    let init_data = STANDARD
        .decode(&pssh)
        .map_err(|e| format!("decode PSSH: {e}"))?;

    tracing::info!("am.stream: creating CDM with {} byte init_data", init_data.len());
    let cdm = Cdm::new_default(&init_data)?;
    let license_request = cdm.get_license_request()?;
    tracing::info!("am.stream: license request generated ({} bytes)", license_request.len());
    tracing::debug!("am.stream: license request first 50 bytes: {}", license_request[..license_request.len().min(50)].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
    tracing::info!("am.stream: KID (b64) = {}, uri_prefix = {}", playback.kid_base64, playback.uri_prefix);
    tracing::debug!("am.stream: kid decoded len = {}", STANDARD.decode(&playback.kid_base64).unwrap_or_default().len());
    tracing::debug!("am.stream: pssh (b64) = {pssh}");
    tracing::debug!("am.stream: pssh decoded len = {}", init_data.len());

    tracing::info!("am.stream: exchanging license with Apple");

    let (key_hex, key_bytes) = get_content_key(
        &cdm,
        &license_request,
        &adam_id,
        &playback.uri_prefix,
        &playback.kid_base64,
        &bearer_token,
        media_user_token,
    )
    .await?;

    tracing::info!("am.stream: got content key (len={}, hex={})", key_bytes.len(), &key_hex[..32.min(key_hex.len())]);

    tracing::info!("am.stream: downloading encrypted fMP4 from {}", playback.file_url);

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
        "am.stream: downloaded {} bytes, decrypting with key {}",
        encrypted_bytes.len(),
        &key_hex[..32.min(key_hex.len())]
    );

    // Two-step: mp4decrypt decrypts the fMP4, then MP4Box extracts raw AAC.
    // mp4decrypt handles Apple Music's CENC/fPIFF encryption.
    // MP4Box -raw extracts plain ADTS AAC that Symphonia can decode.
    let decrypted = decrypt_and_extract_aac(&encrypted_bytes, &key_hex).await?;

    tracing::info!("am.stream: decrypted {} bytes", decrypted.len());

    Ok(decrypted)
}

/// Two-step decryption + AAC extraction pipeline:
/// 1. mp4decrypt: decrypt Apple Music's fragmented CENC MP4 → standard m4a
/// 2. MP4Box -raw: extract plain AAC from the m4a
/// Falls back to the m4a if MP4Box fails.
async fn decrypt_and_extract_aac(encrypted: &[u8], key_hex: &str) -> Result<Vec<u8>, String> {
    use tokio::fs;
    use std::process::Stdio;

    let tmp_dir = std::env::temp_dir().join("kopuz_am_decrypt");
    fs::create_dir_all(&tmp_dir).await.map_err(|e| format!("create tmp dir: {e}"))?;

    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let encrypted_path = tmp_dir.join(format!("enc_{id}.m4s"));
    let decrypted_path = tmp_dir.join(format!("dec_{id}.m4a"));
    let aac_path = tmp_dir.join(format!("aac_{id}.aac"));

    fs::write(&encrypted_path, encrypted).await.map_err(|e| format!("write encrypted: {e}"))?;

    // Step 1: mp4decrypt
    tracing::info!("am.decrypt: step 1 — mp4decrypt --key 1:{key_hex}");
    let status = tokio::process::Command::new("mp4decrypt")
        .arg("--key")
        .arg(format!("1:{key_hex}"))
        .arg(&encrypted_path)
        .arg(&decrypted_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("mp4decrypt spawn: {e}"))?;

    if !status.success() {
        let _ = fs::remove_file(&encrypted_path).await;
        return Err(format!("mp4decrypt failed with status {status}"));
    }

    let dec_size = fs::metadata(&decrypted_path).await.map(|m| m.len()).unwrap_or(0);
    tracing::info!("am.decrypt: mp4decrypt produced {dec_size} bytes → {}", decrypted_path.display());

    // Step 2: MP4Box -raw to extract plain AAC
    tracing::info!("am.decrypt: step 2 — MP4Box -raw 1 → AAC");
    let output = tokio::process::Command::new("MP4Box")
        .arg("-raw")
        .arg("1")
        .arg(&decrypted_path)
        .arg("-out")
        .arg(&aac_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("MP4Box spawn: {e}"))?;

    let mp4box_stdout = String::from_utf8_lossy(&output.stdout);
    let mp4box_stderr = String::from_utf8_lossy(&output.stderr);
    if !mp4box_stdout.is_empty() {
        tracing::info!("am.decrypt: MP4Box stdout: {}", mp4box_stdout.trim());
    }
    if !mp4box_stderr.is_empty() {
        tracing::warn!("am.decrypt: MP4Box stderr: {}", mp4box_stderr.trim());
    }

    if !output.status.success() {
        tracing::warn!("am.decrypt: MP4Box failed (status={}), falling back to m4a at {}",
            output.status, decrypted_path.display());
        let aac_bytes = fs::read(&decrypted_path).await.map_err(|e| format!("read m4a: {e}"))?;
        let _ = fs::remove_file(&encrypted_path).await;
        let _ = fs::remove_file(&aac_path).await;
        return Ok(aac_bytes);
    }

    let aac_bytes = fs::read(&aac_path).await.map_err(|e| format!("read AAC: {e}"))?;
    let _ = fs::remove_file(&encrypted_path).await;
    // Keep decrypted m4a and extracted aac for debugging
    tracing::info!("am.decrypt: files kept at {} and {}", decrypted_path.display(), aac_path.display());

    tracing::info!("am.decrypt: extracted {} bytes of raw AAC", aac_bytes.len());
    Ok(aac_bytes)
}
