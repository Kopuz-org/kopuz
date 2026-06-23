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

    // Find the "28:ctrp256" audio asset
    let assets = song["assets"].as_array().ok_or("no assets")?;
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

    let (uri_prefix, kid_base64) = key_uri
        .split_once(',')
        .ok_or("KEY URI not in expected format 'prefix,kid'")?;

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
    pssh: &str,
    bearer_token: &str,
    media_user_token: &str,
) -> Result<(String, Vec<u8>), String> {
    let envelope = serde_json::json!({
        "challenge": STANDARD.encode(license_request),
        "key-system": "com.widevine.alpha",
        "uri": format!("{uri_prefix},{pssh}"),
        "adamId": adam_id,
        "isLibrary": false,
        "user-initiated": true,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(LICENSE_SERVER_URL)
        .header("Content-Type", "application/json")
        .header("Origin", "https://music.apple.com")
        .header("User-Agent", USER_AGENT)
        .header("Referer", "https://music.apple.com/")
        .header("Authorization", format!("Bearer {bearer_token}"))
        .header("Cookie", format!("media-user-token={media_user_token}"))
        .json(&envelope)
        .send()
        .await
        .map_err(|e| format!("license request: {e}"))?;

    let status = resp.status();
    tracing::info!("am.license: HTTP {status}");
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("license HTTP {status}: {text}"));
    }

    let license_json: serde_json::Value = resp.json().await.map_err(|e| format!("parse license: {e}"))?;

    if let Some(err_code) = license_json["errorCode"].as_i64() {
        if err_code != 0 {
            return Err(format!("license error code: {err_code}"));
        }
    }

    let license_b64 = license_json["license"]
        .as_str()
        .ok_or("no license in response")?;

    let license_data = STANDARD
        .decode(license_b64)
        .map_err(|e| format!("decode license: {e}"))?;

    let keys = cdm.get_license_keys(license_request, &license_data)?;

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

/// Decrypt a CENC-encrypted fMP4 using AES-CTR with the content key.
/// For Apple Music, the file body after the mdat box header is AES-CTR encrypted.
pub fn decrypt_fmp4(encrypted: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    // Find mdat box offset
    let mut mdat_offset = 0;
    let mut pos = 0;
    while pos + 8 <= encrypted.len() {
        let box_size = u32::from_be_bytes([
            encrypted[pos],
            encrypted[pos + 1],
            encrypted[pos + 2],
            encrypted[pos + 3],
        ]) as usize;
        let box_type = &encrypted[pos + 4..pos + 8];

        if box_type == b"mdat" {
            mdat_offset = pos + 8;
            break;
        }

        if box_size < 8 {
            pos += 8;
            continue;
        }
        pos += box_size;
    }

    if mdat_offset == 0 {
        tracing::warn!("am.decrypt_fmp4: no mdat box found, returning as-is");
        return Ok(encrypted.to_vec());
    }

    // Apple Music uses AES-128-CTR with IV = 16 zero bytes
    let iv = [0u8; 16];
    let mut decrypted = encrypted.to_vec();

    // Decrypt from mdat_offset onwards using AES-CTR
    let encrypted_part = &mut decrypted[mdat_offset..];

    // Manual AES-CTR implementation (keystream XOR)
    use aes::Aes128;
    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockEncrypt, KeyInit};

    struct AesCtr {
        cipher: Aes128,
        counter: [u8; 16],
    }

    impl AesCtr {
        fn new(key: &[u8; 16], iv: &[u8; 16]) -> Self {
            let cipher = Aes128::new(GenericArray::from_slice(key));
            Self {
                cipher,
                counter: *iv,
            }
        }

        fn apply_keystream(&mut self, data: &mut [u8]) {
            let mut remaining = data;
            while !remaining.is_empty() {
                let mut keystream_block = aes::Block::clone_from_slice(&self.counter);
                self.cipher.encrypt_block(&mut keystream_block);

                let len = remaining.len().min(16);
                for i in 0..len {
                    remaining[i] ^= keystream_block[i];
                }
                remaining = &mut remaining[len..];

                for i in (0..16).rev() {
                    self.counter[i] = self.counter[i].wrapping_add(1);
                    if self.counter[i] != 0 {
                        break;
                    }
                }
            }
        }
    }

    let mut ctr = AesCtr::new(
        key.try_into().map_err(|_| "key must be 16 bytes")?,
        &iv,
    );
    ctr.apply_keystream(encrypted_part);

    Ok(decrypted)
}

/// Full pipeline: resolve + download + decrypt. Returns decrypted fMP4 bytes.
pub async fn resolve_and_decrypt(adam_id: &str, media_user_token: &str) -> Result<Vec<u8>, String> {
    let bearer_token = auth::get_bearer_token().await?;

    tracing::info!("am.stream: resolving web playback for adam_id={adam_id}");

    let playback = get_web_playback(adam_id, &bearer_token, media_user_token).await?;

    tracing::info!("am.stream: building PSSH and CDM license request");

    let pssh = build_pssh(&playback.kid_base64)?;
    let init_data = STANDARD
        .decode(&pssh)
        .map_err(|e| format!("decode PSSH: {e}"))?;

    let cdm = Cdm::new_default(&init_data)?;
    let license_request = cdm.get_license_request()?;

    tracing::info!("am.stream: exchanging license with Apple");

    let (key_hex, key_bytes) = get_content_key(
        &cdm,
        &license_request,
        adam_id,
        &playback.uri_prefix,
        &pssh,
        &bearer_token,
        media_user_token,
    )
    .await?;

    tracing::info!("am.stream: downloading encrypted fMP4 from {}", playback.file_url);

    let client = reqwest::Client::new();
    let encrypted_resp = client
        .get(&playback.file_url)
        .header("User-Agent", USER_AGENT)
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
        &key_hex[..16.min(key_hex.len())]
    );

    let decrypted = decrypt_fmp4(&encrypted_bytes, &key_bytes)?;

    tracing::info!("am.stream: decrypted {} bytes", decrypted.len());

    Ok(decrypted)
}
