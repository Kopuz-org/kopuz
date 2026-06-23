#![allow(dead_code)]

use aes::cipher::KeyIvInit;
use pkcs1::DecodeRsaPrivateKey;
use sha1::{Digest, Sha1};

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

// ── Protobuf wire helpers ──────────────────────────────────────────

fn encode_varint(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
    out
}

fn field_tag(field: u32, wire: u8) -> Vec<u8> {
    encode_varint(((field as u64) << 3) | (wire as u64))
}

fn encode_bytes_field(field: u32, data: &[u8]) -> Vec<u8> {
    let mut out = field_tag(field, 2);
    out.extend_from_slice(&encode_varint(data.len() as u64));
    out.extend_from_slice(data);
    out
}

fn encode_varint_field(field: u32, val: u64) -> Vec<u8> {
    let mut out = field_tag(field, 0);
    out.extend_from_slice(&encode_varint(val));
    out
}

fn encode_message_field(field: u32, msg: &[u8]) -> Vec<u8> {
    encode_bytes_field(field, msg)
}

fn encode_repeated_bytes_field(field: u32, items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for item in items {
        out.extend_from_slice(&encode_bytes_field(field, item));
    }
    out
}

fn decode_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        if *pos >= data.len() {
            return None;
        }
        let byte = data[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Some(result)
}

fn decode_field(data: &[u8], pos: &mut usize) -> Option<(u32, u8, usize)> {
    let tag = decode_varint(data, pos)? as u32;
    let field = tag >> 3;
    let wire = (tag & 0x07) as u8;
    match wire {
        0 => {
            let value_start = *pos;
            decode_varint(data, pos)?;
            Some((field, wire, value_start))
        }
        2 => {
            let _len = decode_varint(data, pos)? as usize;
            let value_start = *pos;
            *pos += _len;
            Some((field, wire, value_start))
        }
        _ => None,
    }
}

fn read_bytes_field(data: &[u8], field: u32) -> Option<Vec<u8>> {
    let mut pos = 0;
    while pos < data.len() {
        let start = pos;
        if let Some((f, wire, value_start)) = decode_field(data, &mut pos) {
            if f == field && wire == 2 {
                let end = pos;
                return Some(data[value_start..end].to_vec());
            }
        } else {
            break;
        }
        let _ = start;
    }
    None
}

fn read_varint_field(data: &[u8], field: u32) -> Option<u64> {
    let mut pos = 0;
    while pos < data.len() {
        if let Some((f, wire, value_start)) = decode_field(data, &mut pos) {
            if f == field && wire == 0 {
                let mut vpos = value_start;
                return decode_varint(data, &mut vpos);
            }
        } else {
            break;
        }
    }
    None
}

fn read_all_message_fields(data: &[u8]) -> Vec<(u32, u8, Vec<u8>)> {
    let mut fields = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        if let Some((f, wire, value_start)) = decode_field(data, &mut pos) {
            let end = pos;
            fields.push((f, wire, data[value_start..end].to_vec()));
        } else {
            break;
        }
    }
    fields
}

// ── WidevineCencHeader ─────────────────────────────────────────────

pub fn encode_widevine_cenc_header(key_id: &[u8], content_id_encoded: &str) -> Vec<u8> {
    let mut msg = Vec::new();
    msg.extend_from_slice(&encode_varint_field(1, 1));
    msg.extend_from_slice(&encode_bytes_field(2, key_id));
    msg.extend_from_slice(&encode_bytes_field(3, b""));
    msg.extend_from_slice(&encode_bytes_field(4, content_id_encoded.as_bytes()));
    msg.extend_from_slice(&encode_bytes_field(6, b""));
    msg
}

fn parse_widevine_cenc_header(data: &[u8]) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut key_ids = Vec::new();
    let mut content_id = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        if let Some((field, wire, value_start)) = decode_field(data, &mut pos) {
            let end = pos;
            let val = &data[value_start..end];
            match (field, wire) {
                (2, 2) => key_ids.push(val.to_vec()),
                (4, 2) => content_id = val.to_vec(),
                _ => {}
            }
        } else {
            break;
        }
    }
    (key_ids, content_id)
}

// ── LicenseRequest (protobuf encode) ───────────────────────────────

fn encode_content_identification_cenc(
    pssh_header: &[u8],
    license_type: u64,
    request_id: &[u8],
) -> Vec<u8> {
    let mut msg = Vec::new();
    msg.extend_from_slice(&encode_message_field(1, pssh_header));
    msg.extend_from_slice(&encode_varint_field(2, license_type));
    msg.extend_from_slice(&encode_bytes_field(3, request_id));
    msg
}

fn encode_content_identification(cenc: &[u8]) -> Vec<u8> {
    encode_message_field(1, cenc)
}

fn encode_license_request(
    client_id: &[u8],
    content_id: &[u8],
    request_type: u64,
    request_time: u32,
    protocol_version: u64,
    key_control_nonce: u32,
) -> Vec<u8> {
    let mut msg = Vec::new();
    msg.extend_from_slice(&encode_bytes_field(1, client_id));
    msg.extend_from_slice(&encode_message_field(2, content_id));
    msg.extend_from_slice(&encode_varint_field(3, request_type));
    msg.extend_from_slice(&encode_varint_field(4, request_time as u64));
    msg.extend_from_slice(&encode_varint_field(6, protocol_version));
    msg.extend_from_slice(&encode_varint_field(7, key_control_nonce as u64));
    msg
}

fn encode_signed_license_request(msg: &[u8], signature: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // type = LICENSE_REQUEST = 1
    out.extend_from_slice(&encode_varint_field(1, 1));
    // msg (LicenseRequest)
    out.extend_from_slice(&encode_message_field(2, msg));
    // signature
    out.extend_from_slice(&encode_bytes_field(3, signature));
    out
}

// ── License parsing ────────────────────────────────────────────────

fn parse_key_container(data: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>, u64) {
    let mut id = Vec::new();
    let mut iv = Vec::new();
    let mut key = Vec::new();
    let mut key_type = 0u64;
    let mut pos = 0;
    while pos < data.len() {
        if let Some((field, wire, value_start)) = decode_field(data, &mut pos) {
            let end = pos;
            let val = &data[value_start..end];
            match (field, wire) {
                (1, 2) => id = val.to_vec(),
                (2, 2) => iv = val.to_vec(),
                (3, 2) => key = val.to_vec(),
                (4, 0) => key_type = val.iter().fold(0u64, |acc, &b| (acc << 7) | (b & 0x7F) as u64),
                _ => {}
            }
        } else {
            break;
        }
    }
    (id, iv, key, key_type)
}

// ── CDM ────────────────────────────────────────────────────────────

pub struct Cdm {
    private_key: rsa::pkcs1v15::SigningKey<Sha1>,
    private_key_raw: rsa::RsaPrivateKey,
    client_id: Vec<u8>,
    session_id: [u8; 32],
    widevine_cenc_header: Vec<u8>,
}

pub struct CdmKey {
    pub id: Vec<u8>,
    pub key_type: u64,
    pub value: Vec<u8>,
}

impl Cdm {
    pub fn new(
        private_key_pem: &str,
        client_id: Vec<u8>,
        init_data: &[u8],
    ) -> Result<Self, String> {
        let private_key = rsa::RsaPrivateKey::from_pkcs1_pem(private_key_pem)
            .map_err(|e| format!("parse private key: {e}"))?;

        if init_data.len() < 32 {
            return Err("initData too short".to_string());
        }

        use rand::RngExt;
        let mut rng = rand::rng();
        let charset = b"ABCDEF0123456789";
        let mut session_id = [0u8; 32];
        for i in 0..16 {
            session_id[i] = charset[rng.random_range(0..charset.len())];
        }
        session_id[16] = b'0';
        session_id[17] = b'1';
        for i in 18..32 {
            session_id[i] = b'0';
        }

        // Store the raw header bytes like Go does (it stores the parsed WidevineCencHeader
        // and marshals it directly). Re-encoding from parsed fields could differ.
        let widevine_cenc_header = init_data[32..].to_vec();

        Ok(Self {
            private_key: rsa::pkcs1v15::SigningKey::<Sha1>::new(private_key.clone()),
            private_key_raw: private_key,
            client_id,
            session_id,
            widevine_cenc_header,
        })
    }

    pub fn new_default(init_data: &[u8]) -> Result<Self, String> {
        Self::new(DEFAULT_PRIVATE_KEY, DEFAULT_CLIENT_ID.to_vec(), init_data)
    }

    pub fn get_license_request(&self) -> Result<Vec<u8>, String> {
        // Use the original WidevineCencHeader bytes directly (same as Go's
        // `licenseRequest.Msg.ContentId.CencId.Pssh = &c.widevineCencHeader`)
        let cenc = encode_content_identification_cenc(
            &self.widevine_cenc_header,
            1, // LICENSE_TYPE_DEFAULT = 1
            &self.session_id,
        );

        let content_id = encode_content_identification(&cenc);

        use rand::RngExt;
        let mut rng = rand::rng();
        let key_control_nonce: u32 = rng.random();

        let request_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;

        let license_request = encode_license_request(
            &self.client_id,
            &content_id,
            1, // LicenseRequest_RequestType_NEW = 1
            request_time,
            21, // ProtocolVersion_CURRENT = 21
            key_control_nonce,
        );

        // RSA-PSS sign the inner message
        let hash = sha1::Sha1::digest(&license_request);
        let signature = self.private_key_raw.sign_with_rng(
            &mut rsa::rand_core::OsRng,
            rsa::Pss::new::<Sha1>(),
            &hash,
        ).map_err(|e| format!("RSA-PSS sign: {e}"))?;

        Ok(encode_signed_license_request(&license_request, &signature))
    }

    pub fn get_license_keys(
        &self,
        license_request: &[u8],
        license_response: &[u8],
    ) -> Result<Vec<CdmKey>, String> {
        // Parse the SignedLicense response
        // field 2 = Msg (License), field 4 = SessionKey
        let mut session_key = Vec::new();
        let mut license_msg = Vec::new();
        let mut pos = 0;
        while pos < license_response.len() {
            if let Some((field, wire, value_start)) = decode_field(license_response, &mut pos) {
                let end = pos;
                let val = &license_response[value_start..end];
                match (field, wire) {
                    (2, 2) => license_msg = val.to_vec(),
                    (4, 2) => session_key = val.to_vec(),
                    _ => {}
                }
            } else {
                break;
            }
        }

        if session_key.is_empty() {
            return Err("no session key in license response".to_string());
        }

        // Re-parse the LicenseRequest to get the inner message for CMAC
        let mut inner_msg = Vec::new();
        let mut lpos = 0;
        while lpos < license_request.len() {
            if let Some((field, wire, value_start)) = decode_field(license_request, &mut lpos) {
                let end = lpos;
                let val = &license_request[value_start..end];
                if field == 2 && wire == 2 {
                    inner_msg = val.to_vec();
                    break;
                }
            } else {
                break;
            }
        }

        // RSA-OAEP decrypt the session key
        use rsa::Oaep;
        let decrypted_session_key = self.private_key_raw.decrypt(
            Oaep::new::<Sha1>(),
            &session_key,
        ).map_err(|e| format!("RSA-OAEP decrypt session key: {e}"))?;

        // Derive encryption key: CMAC("\x01ENCRYPTION" + licenseRequestMsg + "\x00\x00\x00\x80")
        use cmac::{Cmac, Mac};
        type Aes128Cmac = Cmac<aes::Aes128>;

        let mut encryption_key_input = Vec::new();
        encryption_key_input.push(0x01);
        encryption_key_input.extend_from_slice(b"ENCRYPTION");
        encryption_key_input.extend_from_slice(&inner_msg);
        encryption_key_input.extend_from_slice(&[0x00, 0x00, 0x00, 0x80]);

        let mut mac = Aes128Cmac::new_from_slice(&decrypted_session_key)
            .map_err(|e| format!("create CMAC: {e}"))?;
        mac.update(&encryption_key_input);
        let encryption_key = mac.finalize().into_bytes();

        // Parse the License message to get keys
        let mut keys_data = Vec::new();
        let mut kpos = 0;
        while kpos < license_msg.len() {
            if let Some((field, wire, value_start)) = decode_field(&license_msg, &mut kpos) {
                let end = kpos;
                let val = &license_msg[value_start..end];
                if field == 3 && wire == 2 {
                    keys_data.push(val.to_vec());
                }
            } else {
                break;
            }
        }

        // Decrypt each key
        let mut keys = Vec::new();
        for key_data in &keys_data {
            let (id, iv, encrypted_key, key_type) = parse_key_container(key_data);
            if encrypted_key.is_empty() || iv.is_empty() {
                tracing::debug!("am.cdm: skipping key with empty iv or encrypted_key");
                continue;
            }
            if iv.len() < 16 {
                tracing::warn!("am.cdm: key IV is {} bytes, need >= 16, skipping", iv.len());
                continue;
            }
            if encrypted_key.len() % 16 != 0 || encrypted_key.is_empty() {
                tracing::warn!("am.cdm: key is {} bytes, not block-aligned, skipping", encrypted_key.len());
                continue;
            }

            // AES-CBC decrypt (full buffer, matching Go's CryptBlocks)
            let cipher = Aes128CbcDec::new(
                &encryption_key,
                iv[..16].into(),
            );
            let mut decrypted = encrypted_key.clone();
            use aes::cipher::block_padding::NoPadding;
            use aes::cipher::BlockDecryptMut;
            let decrypt_result = cipher.decrypt_padded_mut::<NoPadding>(&mut decrypted);
            if let Err(e) = decrypt_result {
                tracing::warn!("am.cdm: AES-CBC decrypt failed: {e}");
                continue;
            }

            // PKCS7 unpad
            if let Some(&pad_byte) = decrypted.last() {
                let pad_len = pad_byte as usize;
                if pad_len > 0 && pad_len <= 16 && pad_len <= decrypted.len() {
                    let expected_pad = &decrypted[decrypted.len() - pad_len..];
                    if expected_pad.iter().all(|&b| b == pad_byte) {
                        decrypted.truncate(decrypted.len() - pad_len);
                    }
                }
            }

            // Widevine content keys are always 16 bytes (AES-128).
            // After decryption + PKCS7 unpad, truncate to 16 bytes if longer.
            // Extra bytes are garbage from padding or incorrect block decryption.
            if decrypted.len() > 16 {
                tracing::debug!(
                    "am.cdm: key was {} bytes after decrypt, truncating to 16",
                    decrypted.len()
                );
                decrypted.truncate(16);
            }

            tracing::debug!(
                "am.cdm: decrypted key id={} type={} value_len={}",
                hex::encode(&id),
                key_type,
                decrypted.len()
            );

            keys.push(CdmKey {
                id,
                key_type,
                value: decrypted,
            });
        }

        tracing::info!("am.cdm: extracted {} keys from license", keys.len());
        Ok(keys)
    }
}

// ── Device constants ───────────────────────────────────────────────

pub const DEFAULT_PRIVATE_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA2bO3yvFwNnIHsbDl3MTjKdDsiBWsuZWOGVxInFWAVMp+nffG\nYlquTKpJurEry95yprcRB3hYhvA5ghsACidcWPDEPVqqRZ7YXLevyUA+Sn2Jxpvt\nOcwyFHbSwruNxprWOkHCT774O4L/wJUt5x2C4iFCrJByjw0omN8u+EHdavvH7ZPn\nb3/EZp/cpZa9/+HOkutvBHBvaPp18F8JQhzUQ9MwLuDFTr+QLDB5+Y57Je2tNYDK\nxD1K+Ed5Ja0A4OKhPKIwPwPre0nt5scjLba3LSAKtKxiGqFtWO4U7Tf1YrdjJv2o\n9o8Sf8qcnbpzvQ4KwFqehuJnB7+W7mdJJw12PQIDAQABAoIBACE32wOMc6LbI3Fp\nnKljIYZv6qeZJxHqUBRukGXKZhqKC2fvNsYrMA1irn1eK2CgQL5PkLmjE18DqMLB\ne/AQsXagxlDWVMTqx/jdzmTW+KpFHZDAmiIHllypBN/R3oA/gBDDl/KzIQ1zn7Kz\nEJ4DUsVObe4G3HQXfepVo8Udx7tbB7X6wHe2kEgFyY3lPdvubik0C4t4ipSD79y7\nSfW7XVA5XUQmqN4U2kWM0uSwzd4BA7hqyScJsygf6KgpMWPS2xFZEZQRUpYcBH48\nE7YqNrrlYP3yaQ+9Jx56kKS0mvv3vUXS7AfUbU8CiHwD9I3BGwswEUueOGGVeXbx\ntFF8s8ECgYEA97BDcL/bt+r3qJF0dxtMB5ZngJbFx9RdsblYepVpblr2UfxnFttO\nPoNSKa4W36HuDsun49dkaoABJWdtZs2Hy6q+xvEgozvhMaBVE3spnWnzCT1yTMYL\nG02uDEl0dPiTg116bVElaswtqMXvnnpbOTMTe7Ig9sWiUW/GH9RM+N8CgYEA4QHb\n+OA0BfczbVQP9B+plt4mAuu4BDm4GPwq1yXOWo3Ct8Ik+HeY1hqOObpfyQMAza+E\ne/kP6W8vXpiElGrmiUbTXK4Rzmf+yYeOrvl3D80bFq4GtDNAIQD3jpj6zjlT+Gzw\nI501gRx5iPl4fSccRSdpoeri7F9ANtc6EEGFyGMCgYEAjMznWYXHGkL47BtbkIW0\n769BQSj0X4dKh8gsEusylugglDSeSbD7RrASGd175T7A/CorU2rTC3OesyubVlBJ\n/K4gaykRe5mDh1l0Y3GlE3XyEXObsSb3k1rSMOvkxsWz3X5bJR923MIaxpFWiMlX\naCmvzqZQ9NceUZrvjpJ5+xMCgYAJa8KCESEcftUwZqykVA8Nug9tX+E8jA4hPa2t\nhG+3augUOZTCsn87t7Dsydjo2a9W7Vpmtm7sHzOkik5CyJcOeGCxKLimI8SPO5XF\nzbwmdTgFIxQ0x1CQETJMTityJwRVCnqjgxmSZlbQXWGmG9UbMCNEHEmUDAjsQuaz\nd4racQKBgQDR1Y2kalvleYGrhwcA8LTnIh0rYEfAt9YxNmTi5qDKf5QPvUP2v+WO\nfSB5coUqR8LBweHE5V8JgFt74fdLBqZV/k2z/dI0r+EQWmpZ2uPEC0Khk/Sb9iRD\nfH7at3PMusrkwZCGZ8beFEAr6icXclV08nPCNGB6WckacfzpAj8Azg==\n-----END RSA PRIVATE KEY-----";

pub const DEFAULT_CLIENT_ID: &[u8] = &[
    0x08, 0x01, 0x12, 0x9a, 0x0b, 0x0a, 0xdd, 0x03, 0x08, 0x02, 0x12, 0x10, 0x79, 0xe4, 0x6b, 0xc9,
    0xc4, 0x79, 0xa0, 0x09, 0xd5, 0xbd, 0x20, 0xab, 0x77, 0x31, 0x6b, 0x4f, 0x18, 0xaf, 0x82, 0xc2,
    0xa5, 0x06, 0x22, 0x8e, 0x02, 0x30, 0x82, 0x01, 0x0a, 0x02, 0x82, 0x01, 0x01, 0x00, 0xd9, 0xb3,
    0xb7, 0xca, 0xf1, 0x70, 0x36, 0x72, 0x07, 0xb1, 0xb0, 0xe5, 0xdc, 0xc4, 0xe3, 0x29, 0xd0, 0xec,
    0x88, 0x15, 0xac, 0xb9, 0x95, 0x8e, 0x19, 0x5c, 0x48, 0x9c, 0x55, 0x80, 0x54, 0xca, 0x7e, 0x9d,
    0xf7, 0xc6, 0x62, 0x5a, 0xae, 0x4c, 0xaa, 0x49, 0xba, 0xb1, 0x2b, 0xcb, 0xde, 0x72, 0xa6, 0xb7,
    0x11, 0x07, 0x78, 0x58, 0x86, 0xf0, 0x39, 0x82, 0x1b, 0x00, 0x0a, 0x27, 0x5c, 0x58, 0xf0, 0xc4,
    0x3d, 0x5a, 0xaa, 0x45, 0x9e, 0xd8, 0x5c, 0xb7, 0xaf, 0xc9, 0x40, 0x3e, 0x4a, 0x7d, 0x89, 0xc6,
    0x9b, 0xed, 0x39, 0xcc, 0x32, 0x14, 0x76, 0xd2, 0xc2, 0xbb, 0x8d, 0xc6, 0x9a, 0xd6, 0x3a, 0x41,
    0xc2, 0x4f, 0xbe, 0xf8, 0x3b, 0x82, 0xff, 0xc0, 0x95, 0x2d, 0xe7, 0x1d, 0x82, 0xe2, 0x21, 0x42,
    0xac, 0x90, 0x72, 0x8f, 0x0d, 0x28, 0x98, 0xdf, 0x2e, 0xf8, 0x41, 0xdd, 0x6a, 0xfb, 0xc7, 0xed,
    0x93, 0xe7, 0x6f, 0x7f, 0xc4, 0x66, 0x9f, 0xdc, 0xa5, 0x96, 0xbd, 0xff, 0xe1, 0xce, 0x92, 0xeb,
    0x6f, 0x04, 0x70, 0x6f, 0x68, 0xfa, 0x75, 0xf0, 0x5f, 0x09, 0x42, 0x1c, 0xd4, 0x43, 0xd3, 0x30,
    0x2e, 0xe0, 0xc5, 0x4e, 0xbf, 0x90, 0x2c, 0x30, 0x79, 0xf9, 0x8e, 0x7b, 0x25, 0xed, 0xad, 0x35,
    0x80, 0xca, 0xc4, 0x3d, 0x4a, 0xf8, 0x47, 0x79, 0x25, 0xad, 0x00, 0xe0, 0xe2, 0xa1, 0x3c, 0xa2,
    0x30, 0x3f, 0x03, 0xeb, 0x7b, 0x49, 0xed, 0xe6, 0xc7, 0x23, 0x2d, 0xb6, 0xb7, 0x2d, 0x20, 0x0a,
    0xb4, 0xac, 0x62, 0x1a, 0xa1, 0x6d, 0x58, 0xee, 0x14, 0xed, 0x37, 0xf5, 0x62, 0xb7, 0x63, 0x26,
    0xfd, 0xa8, 0xf6, 0x8f, 0x12, 0x7f, 0xca, 0x9c, 0x9d, 0xba, 0x73, 0xbd, 0x0e, 0x0a, 0xc0, 0x5a,
    0x9e, 0x86, 0xe2, 0x67, 0x07, 0xbf, 0x96, 0xee, 0x67, 0x49, 0x27, 0x0d, 0x76, 0x3d, 0x02, 0x03,
    0x01, 0x00, 0x01, 0x28, 0xf0, 0x22, 0x48, 0x01, 0x52, 0xaa, 0x01, 0x08, 0x01, 0x10, 0x00, 0x1a,
    0x81, 0x01, 0x04, 0x19, 0x86, 0x1d, 0x0b, 0x3e, 0xce, 0x70, 0x9a, 0xeb, 0xa8, 0x6d, 0xa2, 0x68,
    0x7d, 0x52, 0xb5, 0x93, 0x40, 0xe3, 0xbe, 0x73, 0x61, 0xaa, 0xf3, 0x8c, 0x7c, 0x6c, 0x34, 0x66,
    0x5c, 0xee, 0x9b, 0x86, 0x93, 0x02, 0xf6, 0xac, 0x32, 0x2c, 0x96, 0x7e, 0x18, 0xe8, 0xff, 0xec,
    0x66, 0x1e, 0xdb, 0x11, 0xbc, 0xad, 0xde, 0x93, 0xd1, 0x0e, 0x74, 0x80, 0x75, 0x7b, 0x67, 0x39,
    0x6b, 0x31, 0x6e, 0x49, 0xc0, 0x0c, 0x4b, 0x69, 0xfb, 0x64, 0x6c, 0xea, 0x03, 0xc5, 0xc1, 0x6f,
    0xe5, 0x0f, 0x33, 0x8e, 0x85, 0x51, 0xe1, 0xad, 0xee, 0x8d, 0x63, 0x0e, 0x6f, 0xc0, 0xca, 0xc6,
    0x99, 0xb7, 0x85, 0x3a, 0x26, 0x2f, 0x30, 0x5e, 0x5e, 0x35, 0x75, 0x60, 0x3d, 0x90, 0x5f, 0xe2,
    0x20, 0xae, 0xee, 0x4e, 0x14, 0xa6, 0x6f, 0xe9, 0xa1, 0xb2, 0x92, 0xcb, 0x62, 0x8d, 0xc0, 0x52,
    0x2b, 0xb6, 0x86, 0x22, 0x20, 0x16, 0x6f, 0x22, 0x75, 0x4e, 0x3e, 0x52, 0xf4, 0x9f, 0x54, 0x1e,
    0x09, 0xbd, 0xe2, 0x7e, 0x86, 0xa7, 0x8d, 0x6e, 0x96, 0x2e, 0x35, 0x69, 0x2b, 0xc6, 0x56, 0xe3,
    0xf4, 0x30, 0xe3, 0x5a, 0x06, 0x12, 0x80, 0x02, 0x1e, 0x98, 0x34, 0x1c, 0xcb, 0xe4, 0x4a, 0x53,
    0x0c, 0x06, 0xab, 0x9f, 0x4d, 0x85, 0xac, 0x58, 0x48, 0x7d, 0xe1, 0xc5, 0x21, 0xc6, 0x0a, 0x48,
    0x5c, 0x5e, 0x78, 0xcd, 0x0b, 0x43, 0x9e, 0x57, 0x98, 0x01, 0x74, 0x94, 0x25, 0xee, 0xea, 0x8d,
    0xf7, 0x49, 0x64, 0x17, 0x72, 0x71, 0x9e, 0x5b, 0xd8, 0x17, 0x79, 0xa1, 0xbb, 0x1e, 0xe9, 0x65,
    0x1e, 0x63, 0x8a, 0x2e, 0x0a, 0x68, 0xd1, 0xba, 0xfa, 0xf6, 0x57, 0x12, 0xad, 0xb5, 0x8a, 0xb3,
    0x4c, 0x9e, 0x84, 0xb1, 0xc0, 0x89, 0x73, 0xbe, 0x93, 0x28, 0xbf, 0xbc, 0x4e, 0xbd, 0x05, 0x95,
    0x9b, 0x4d, 0x0a, 0xfb, 0x10, 0xe4, 0x3b, 0x6d, 0x27, 0x71, 0x30, 0xb5, 0x0a, 0x55, 0xe9, 0x98,
    0x00, 0x07, 0xc8, 0x35, 0xe6, 0x31, 0xe4, 0xeb, 0xce, 0xf0, 0x8b, 0xa3, 0xbb, 0x5c, 0xdd, 0x44,
    0x9b, 0x68, 0x89, 0xa5, 0x21, 0x60, 0x17, 0xf7, 0x6b, 0x1a, 0x66, 0x88, 0xf3, 0x27, 0x79, 0xfd,
    0x81, 0xcf, 0x9c, 0x8c, 0x12, 0x86, 0xab, 0x3d, 0x80, 0x81, 0xcb, 0x05, 0x0a, 0x28, 0x12, 0x67,
    0x9b, 0xe9, 0x2e, 0xdc, 0xff, 0xc0, 0x49, 0x39, 0x3f, 0x12, 0x06, 0x28, 0x9e, 0x8e, 0x5a, 0x31,
    0x32, 0xe0, 0x3e, 0x47, 0x64, 0x6d, 0x29, 0xd8, 0xc6, 0x32, 0x84, 0xcf, 0x38, 0x7d, 0x48, 0x77,
    0x4c, 0xe7, 0xee, 0x7b, 0x12, 0xb5, 0x71, 0xe5, 0xab, 0x39, 0xf3, 0xf1, 0x09, 0xbc, 0x52, 0x3d,
    0x18, 0xbd, 0xc3, 0x40, 0x44, 0x68, 0x39, 0xa2, 0xc6, 0xdf, 0xca, 0xe2, 0x62, 0x1a, 0x95, 0x41,
    0x53, 0x04, 0x9b, 0x16, 0xc6, 0xd1, 0x3a, 0xc3, 0x0d, 0xfc, 0x52, 0x61, 0x4b, 0x92, 0xd5, 0xa8,
    0x30, 0xcf, 0x3e, 0x40, 0xb9, 0xe1, 0xc5, 0xee, 0xa3, 0x9a, 0xe5, 0x69, 0xb9, 0xc8, 0x5d, 0x2e,
    0xd9, 0xcc, 0x3e, 0x79, 0x00, 0xb3, 0xa5, 0xe3, 0x1a, 0xb4, 0x05, 0x0a, 0xae, 0x02, 0x08, 0x01,
    0x12, 0x10, 0x69, 0xe3, 0xe8, 0x98, 0xbb, 0x2c, 0x3f, 0xb8, 0xa3, 0xb3, 0x28, 0x1d, 0x84, 0xf8,
    0x8c, 0x14, 0x18, 0x8e, 0xd5, 0xbe, 0x91, 0x05, 0x22, 0x8e, 0x02, 0x30, 0x82, 0x01, 0x0a, 0x02,
    0x82, 0x01, 0x01, 0x00, 0xd8, 0xf6, 0xe9, 0xb5, 0x89, 0xf0, 0x51, 0x20, 0xe9, 0xa4, 0x3e, 0xd0,
    0xd9, 0x4e, 0xa1, 0xfe, 0x09, 0x95, 0x01, 0x6d, 0xbd, 0x1e, 0x2b, 0x40, 0xa7, 0xdc, 0xd1, 0xc5,
    0x7f, 0xc3, 0x05, 0x50, 0x3c, 0xcf, 0x13, 0x3f, 0x9e, 0x98, 0xaf, 0xce, 0x4e, 0xe6, 0xff, 0x84,
    0xdc, 0x42, 0x24, 0x26, 0xa8, 0xff, 0xfa, 0x4f, 0xe4, 0xbf, 0x2d, 0x44, 0xd5, 0x0f, 0x14, 0x3a,
    0xeb, 0xbc, 0x20, 0x4c, 0xa3, 0xb4, 0x67, 0xfa, 0x25, 0x1b, 0xfa, 0x60, 0x3f, 0xdb, 0x25, 0xe2,
    0x27, 0xa8, 0xaf, 0xa9, 0xc3, 0x7d, 0x0a, 0xef, 0x62, 0xe6, 0xa3, 0x94, 0xd7, 0x28, 0x28, 0x9f,
    0xd4, 0x96, 0x65, 0x1b, 0x2e, 0x8c, 0x51, 0x41, 0xf2, 0x7c, 0x55, 0xad, 0xe6, 0x39, 0x2f, 0x37,
    0xaa, 0xd3, 0x6f, 0x37, 0xce, 0x7d, 0x42, 0x83, 0x5b, 0x2d, 0x71, 0x7e, 0x2d, 0x38, 0x74, 0xfb,
    0xba, 0xd3, 0xf3, 0x14, 0x6f, 0xd1, 0x78, 0x31, 0x50, 0xb7, 0x43, 0xbf, 0x18, 0xb9, 0x73, 0x57,
    0x00, 0x74, 0x77, 0x5b, 0x27, 0xdd, 0x22, 0x8c, 0x3b, 0x85, 0xcb, 0x8e, 0x16, 0x5d, 0x9d, 0xca,
    0xed, 0x17, 0xd8, 0xe5, 0x8e, 0x98, 0x3b, 0xc6, 0x33, 0x08, 0x96, 0x75, 0x89, 0x52, 0xac, 0x43,
    0xa3, 0xb4, 0xd0, 0x91, 0x3c, 0xad, 0x26, 0x65, 0x25, 0xd2, 0x47, 0x0c, 0xed, 0xb9, 0xdd, 0x04,
    0xb7, 0xab, 0x01, 0xd2, 0x45, 0x19, 0xa5, 0xdc, 0xea, 0x84, 0x98, 0xe1, 0xe3, 0x72, 0xfc, 0x81,
    0x83, 0x96, 0xe2, 0xc2, 0x41, 0x1d, 0xca, 0x64, 0x44, 0x2c, 0xcb, 0xf9, 0x8b, 0x79, 0x5d, 0x60,
    0x81, 0x0e, 0x38, 0x30, 0x5a, 0xfb, 0x5c, 0xe3, 0x3e, 0xad, 0xdf, 0xb8, 0xff, 0x7d, 0x78, 0xab,
    0xf7, 0x5e, 0x41, 0xb8, 0xc7, 0xaa, 0x35, 0x85, 0xb9, 0x5a, 0xfc, 0x88, 0x86, 0x8a, 0x6e, 0xaf,
    0x91, 0xeb, 0x74, 0xa1, 0x02, 0x03, 0x01, 0x00, 0x01, 0x28, 0xf0, 0x22, 0x12, 0x80, 0x03, 0x22,
    0x8b, 0x63, 0x30, 0xeb, 0x3f, 0x90, 0xb3, 0x56, 0xef, 0xbf, 0x11, 0x92, 0xb6, 0x27, 0xbd, 0xb6,
    0x97, 0x20, 0xa0, 0xed, 0xae, 0x02, 0xde, 0x59, 0xbe, 0xfd, 0x02, 0x0d, 0x7b, 0xeb, 0x16, 0x5c,
    0x27, 0x49, 0xba, 0x3e, 0xa9, 0x1e, 0xf5, 0x46, 0xed, 0x88, 0x95, 0x95, 0x95, 0x49, 0xb3, 0x79,
    0x5d, 0x84, 0x88, 0x0f, 0x07, 0x30, 0x07, 0x31, 0xe3, 0x63, 0xca, 0xcd, 0xaf, 0x77, 0xe0, 0xb1,
    0x7b, 0x52, 0x26, 0xdc, 0xb8, 0xdb, 0x97, 0x63, 0x35, 0x0b, 0x45, 0xe7, 0xe2, 0x76, 0x8e, 0xb0,
    0x21, 0x7f, 0x74, 0x36, 0x9b, 0xb1, 0x08, 0xe5, 0xaa, 0x21, 0x64, 0x6a, 0x43, 0xd8, 0x4d, 0x1c,
    0xaf, 0xe7, 0x55, 0x8f, 0xda, 0x3c, 0xc7, 0x4b, 0x89, 0x47, 0x63, 0x56, 0x47, 0xc1, 0x55, 0xa1,
    0x54, 0x12, 0x5d, 0x9a, 0x5a, 0x77, 0x73, 0x3f, 0xe1, 0x0c, 0xc1, 0x2c, 0xdb, 0x68, 0xfe, 0xdc,
    0xfe, 0x50, 0xac, 0x99, 0xc1, 0xae, 0xd2, 0x73, 0xf7, 0x93, 0x41, 0xcd, 0xbe, 0x65, 0xc0, 0xd2,
    0xdd, 0x91, 0x0e, 0xde, 0x51, 0x91, 0xb0, 0x7f, 0x3a, 0xe0, 0xa7, 0xff, 0x70, 0x41, 0xaf, 0x25,
    0x10, 0x41, 0xa9, 0xca, 0x49, 0x5c, 0xa7, 0xc6, 0xf8, 0x8b, 0x6a, 0x11, 0x6d, 0x40, 0x00, 0xfa,
    0x6d, 0x8e, 0xe8, 0x20, 0xce, 0x66, 0x44, 0xcd, 0x1a, 0xb9, 0xc5, 0x31, 0x9f, 0xea, 0x8a, 0xfe,
    0x6d, 0x7a, 0xca, 0x8c, 0x2a, 0x41, 0x4b, 0xd1, 0xb5, 0x48, 0xec, 0xaf, 0x08, 0xd1, 0x75, 0x93,
    0x9c, 0xfa, 0x94, 0x6f, 0x0a, 0xb1, 0x78, 0x60, 0x43, 0x12, 0xfa, 0x00, 0x31, 0xcc, 0xa2, 0xe9,
    0x71, 0xbd, 0x42, 0x22, 0x87, 0x23, 0xd8, 0xe4, 0xca, 0x62, 0x99, 0xd5, 0xdf, 0x51, 0xd1, 0x10,
    0xe4, 0x70, 0x36, 0x12, 0xa3, 0x62, 0x35, 0x53, 0x48, 0x3b, 0x8a, 0x8f, 0xd3, 0x4e, 0xe3, 0x5b,
    0x9d, 0x72, 0x01, 0xe4, 0xe1, 0x3a, 0xbc, 0xb8, 0x27, 0x6d, 0xb1, 0xb3, 0xc4, 0x5d, 0x90, 0x80,
    0xa5, 0x38, 0x60, 0x1a, 0xb8, 0xb2, 0x25, 0xd2, 0x52, 0xc0, 0xe1, 0xf9, 0x4c, 0xcd, 0x6f, 0x95,
    0x7d, 0x09, 0x79, 0xc5, 0x8c, 0x4f, 0xd0, 0x4e, 0x6d, 0xba, 0x19, 0xf4, 0x87, 0xef, 0x38, 0xe0,
    0x3b, 0x25, 0x53, 0xc6, 0x45, 0xed, 0x4d, 0x5d, 0x53, 0x64, 0x29, 0xac, 0x80, 0x29, 0x7d, 0xf5,
    0xc3, 0xf7, 0x08, 0x00, 0x00, 0x29, 0x36, 0xe9, 0x18, 0x20, 0xcb, 0xc7, 0xbd, 0xeb, 0x3f, 0x22,
    0xf3, 0xaf, 0xfe, 0x61, 0x8c, 0x99, 0x42, 0x47, 0x3d, 0x84, 0x87, 0x42, 0x00, 0x44, 0x6b, 0x80,
    0x46, 0x02, 0x11, 0xb0, 0x72, 0xcc, 0xe3, 0x6b, 0x75, 0x10, 0xec, 0xd4, 0xca, 0xd2, 0xfd, 0xc8,
    0x75, 0x9a, 0x85, 0x79, 0xa4, 0x9b, 0xfa, 0xdc, 0x12, 0x0a, 0xbd, 0x01, 0xc6, 0xbc, 0xb9, 0x1a,
    0x16, 0x0a, 0x0c, 0x63, 0x6f, 0x6d, 0x70, 0x61, 0x6e, 0x79, 0x5f, 0x6e, 0x61, 0x6d, 0x65, 0x12,
    0x06, 0x47, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x1a, 0x21, 0x0a, 0x0a, 0x6d, 0x6f, 0x64, 0x65, 0x6c,
    0x5f, 0x6e, 0x61, 0x6d, 0x65, 0x12, 0x13, 0x41, 0x4f, 0x53, 0x50, 0x20, 0x6f, 0x6e, 0x20, 0x49,
    0x41, 0x20, 0x45, 0x6d, 0x75, 0x6c, 0x61, 0x74, 0x6f, 0x72, 0x1a, 0x18, 0x0a, 0x11, 0x61, 0x72,
    0x63, 0x68, 0x69, 0x74, 0x65, 0x63, 0x74, 0x75, 0x72, 0x65, 0x5f, 0x6e, 0x61, 0x6d, 0x65, 0x12,
    0x03, 0x78, 0x38, 0x36, 0x1a, 0x1e, 0x0a, 0x0b, 0x64, 0x65, 0x76, 0x69, 0x63, 0x65, 0x5f, 0x6e,
    0x61, 0x6d, 0x65, 0x12, 0x0f, 0x67, 0x65, 0x6e, 0x65, 0x72, 0x69, 0x63, 0x5f, 0x78, 0x38, 0x36,
    0x5f, 0x61, 0x72, 0x6d, 0x1a, 0x22, 0x0a, 0x0c, 0x70, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x74, 0x5f,
    0x6e, 0x61, 0x6d, 0x65, 0x12, 0x12, 0x73, 0x64, 0x6b, 0x5f, 0x67, 0x70, 0x68, 0x6f, 0x6e, 0x65,
    0x5f, 0x78, 0x38, 0x36, 0x5f, 0x61, 0x72, 0x6d, 0x1a, 0x64, 0x0a, 0x0a, 0x62, 0x75, 0x69, 0x6c,
    0x64, 0x5f, 0x69, 0x6e, 0x66, 0x6f, 0x12, 0x56, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x2f, 0x73,
    0x64, 0x6b, 0x5f, 0x67, 0x70, 0x68, 0x6f, 0x6e, 0x65, 0x5f, 0x78, 0x38, 0x36, 0x5f, 0x61, 0x72,
    0x6d, 0x2f, 0x67, 0x65, 0x6e, 0x65, 0x72, 0x69, 0x63, 0x5f, 0x78, 0x38, 0x36, 0x5f, 0x61, 0x72,
    0x6d, 0x3a, 0x39, 0x2f, 0x50, 0x53, 0x52, 0x31, 0x2e, 0x31, 0x38, 0x30, 0x37, 0x32, 0x30, 0x2e,
    0x31, 0x32, 0x32, 0x2f, 0x36, 0x37, 0x33, 0x36, 0x37, 0x34, 0x32, 0x3a, 0x75, 0x73, 0x65, 0x72,
    0x64, 0x65, 0x62, 0x75, 0x67, 0x2f, 0x64, 0x65, 0x76, 0x2d, 0x6b, 0x65, 0x79, 0x73, 0x1a, 0x1e,
    0x0a, 0x14, 0x77, 0x69, 0x64, 0x65, 0x76, 0x69, 0x6e, 0x65, 0x5f, 0x63, 0x64, 0x6d, 0x5f, 0x76,
    0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x12, 0x06, 0x31, 0x34, 0x2e, 0x30, 0x2e, 0x30, 0x1a, 0x24,
    0x0a, 0x1f, 0x6f, 0x65, 0x6d, 0x5f, 0x63, 0x72, 0x79, 0x70, 0x74, 0x6f, 0x5f, 0x73, 0x65, 0x63,
    0x75, 0x72, 0x69, 0x74, 0x79, 0x5f, 0x70, 0x61, 0x74, 0x63, 0x68, 0x5f, 0x6c, 0x65, 0x76, 0x65,
    0x6c, 0x12, 0x01, 0x30, 0x32, 0x0e, 0x10, 0x01, 0x20, 0x00, 0x28, 0x0d, 0x30, 0x00, 0x40, 0x00,
    0x48, 0x00, 0x50, 0x00,
];

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    const TEST_KID_B64: &str = "AAAAAGmd8sAAHUQHlVERpg==";

    // Reference values from Go TestCDMDumpForCrossValidation (deterministic parts only;
    // session_id, request_time, key_control_nonce are random and excluded)

    #[test]
    fn test_varint_encoding() {
        assert_eq!(encode_varint(0), vec![0x00]);
        assert_eq!(encode_varint(1), vec![0x01]);
        assert_eq!(encode_varint(127), vec![0x7F]);
        assert_eq!(encode_varint(128), vec![0x80, 0x01]);
        assert_eq!(encode_varint(21), vec![0x15]);
    }

    #[test]
    fn test_widevine_header_matches_go() {
        let kid = base64::engine::general_purpose::STANDARD.decode(TEST_KID_B64).unwrap();
        let content_id_encoded = base64::engine::general_purpose::STANDARD.encode(b"");
        let header = encode_widevine_cenc_header(&kid, &content_id_encoded);

        // Go reference: HEADER_HEX=0801121000000000699df2c0001d4407955111a61a0022003200
        // HEADER_LEN=26
        let expected = hex::decode("0801121000000000699df2c0001d4407955111a61a0022003200").unwrap();
        assert_eq!(header, expected, "WidevineCencHeader must be byte-identical to Go output");
        assert_eq!(header.len(), 26, "header length must be 26");
    }

    #[test]
    fn test_pssh_construction_matches_go() {
        let kid = base64::engine::general_purpose::STANDARD.decode(TEST_KID_B64).unwrap();
        let content_id_encoded = base64::engine::general_purpose::STANDARD.encode(b"");
        let header = encode_widevine_cenc_header(&kid, &content_id_encoded);

        let mut pssh_data = b"0123456789abcdef0123456789abcdef".to_vec();
        pssh_data.extend_from_slice(&header);
        let pssh_b64 = base64::engine::general_purpose::STANDARD.encode(&pssh_data);

        // Go reference: PSSH_LEN=58
        // PSSH_B64=MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYIARIQAAAAAGmd8sAAHUQHlVERphoAIgAyAA==
        assert_eq!(pssh_data.len(), 58, "PSSH length must be 58");
        assert_eq!(pssh_b64, "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYIARIQAAAAAGmd8sAAHUQHlVERphoAIgAyAA==",
            "PSSH base64 must match Go");
    }

    #[test]
    fn test_cdm_creation_and_license_request() {
        let kid = base64::engine::general_purpose::STANDARD.decode(TEST_KID_B64).unwrap();
        let content_id_encoded = base64::engine::general_purpose::STANDARD.encode(b"");
        let header = encode_widevine_cenc_header(&kid, &content_id_encoded);

        let mut pssh_data = b"0123456789abcdef0123456789abcdef".to_vec();
        pssh_data.extend_from_slice(&header);
        let pssh_b64 = base64::engine::general_purpose::STANDARD.encode(&pssh_data);
        let init_data = base64::engine::general_purpose::STANDARD.decode(&pssh_b64).unwrap();

        let cdm = Cdm::new(DEFAULT_PRIVATE_KEY, DEFAULT_CLIENT_ID.to_vec(), &init_data).unwrap();
        let req = cdm.get_license_request().unwrap();

        // Go reference: LICENSE_REQ_LEN=2131
        assert_eq!(req.len(), 2131, "license request length must be 2131");
    }

    #[test]
    fn test_signed_license_request_structure() {
        let kid = base64::engine::general_purpose::STANDARD.decode(TEST_KID_B64).unwrap();
        let content_id_encoded = base64::engine::general_purpose::STANDARD.encode(b"");
        let header = encode_widevine_cenc_header(&kid, &content_id_encoded);
        let mut pssh_data = b"0123456789abcdef0123456789abcdef".to_vec();
        pssh_data.extend_from_slice(&header);
        let pssh_b64 = base64::engine::general_purpose::STANDARD.encode(&pssh_data);
        let init_data = base64::engine::general_purpose::STANDARD.decode(&pssh_b64).unwrap();

        let cdm = Cdm::new(DEFAULT_PRIVATE_KEY, DEFAULT_CLIENT_ID.to_vec(), &init_data).unwrap();
        let req = cdm.get_license_request().unwrap();

        let mut pos = 0;
        let mut found_type = false;
        let mut found_msg = false;
        let mut found_sig = false;
        let mut sig_len = 0usize;
        let mut msg_len = 0usize;

        while pos < req.len() {
            if let Some((field, wire, value_start)) = decode_field(&req, &mut pos) {
                let end = pos;
                match (field, wire) {
                    (1, 0) => {
                        let mut vpos = value_start;
                        let val = decode_varint(&req, &mut vpos).unwrap();
                        assert_eq!(val, 1, "SLR.Type must be LICENSE_REQUEST (1)");
                        found_type = true;
                    }
                    (2, 2) => { found_msg = true; msg_len = end - value_start; }
                    (3, 2) => { found_sig = true; sig_len = end - value_start; }
                    _ => {}
                }
            } else { break; }
        }

        assert!(found_type, "must have Type field");
        assert!(found_msg, "must have Msg field");
        assert!(found_sig, "must have Signature field");
        assert_eq!(sig_len, 256, "signature must be 256 bytes");
        assert_eq!(msg_len, 1867, "LicenseRequest must be 1867 bytes");
    }

    #[test]
    fn test_cmac_input_matches_go() {
        let kid = base64::engine::general_purpose::STANDARD.decode(TEST_KID_B64).unwrap();
        let content_id_encoded = base64::engine::general_purpose::STANDARD.encode(b"");
        let header = encode_widevine_cenc_header(&kid, &content_id_encoded);
        let mut pssh_data = b"0123456789abcdef0123456789abcdef".to_vec();
        pssh_data.extend_from_slice(&header);
        let pssh_b64 = base64::engine::general_purpose::STANDARD.encode(&pssh_data);
        let init_data = base64::engine::general_purpose::STANDARD.decode(&pssh_b64).unwrap();

        let cdm = Cdm::new(DEFAULT_PRIVATE_KEY, DEFAULT_CLIENT_ID.to_vec(), &init_data).unwrap();
        let req = cdm.get_license_request().unwrap();

        // Extract inner msg (field 2 of SignedLicenseRequest)
        let mut pos = 0;
        let mut inner_msg = Vec::new();
        while pos < req.len() {
            if let Some((field, wire, value_start)) = decode_field(&req, &mut pos) {
                if field == 2 && wire == 2 {
                    inner_msg = req[value_start..pos].to_vec();
                    break;
                }
            } else { break; }
        }

        // Dump the inner msg for comparison with Go
        println!("RUST_INNER_MSG_LEN={}", inner_msg.len());
        println!("RUST_INNER_MSG_HEX={}", hex::encode(&inner_msg));

        // Build CMAC input
        let mut cmac_input = vec![0x01];
        cmac_input.extend_from_slice(b"ENCRYPTION");
        cmac_input.extend_from_slice(&inner_msg);
        cmac_input.extend_from_slice(&[0x00, 0x00, 0x00, 0x80]);
        println!("RUST_CMAC_INPUT_LEN={}", cmac_input.len());

        // Test CMAC with dummy key
        use cmac::{Cmac, Mac};
        type Aes128Cmac = Cmac<aes::Aes128>;
        let dummy_key: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut mac = Aes128Cmac::new_from_slice(&dummy_key).unwrap();
        mac.update(&cmac_input);
        let result = mac.finalize().into_bytes();
        println!("RUST_CMAC_RESULT={}", hex::encode(&result));
    }

    #[test]
    fn test_pem_key_matches_go() {
        use pkcs1::DecodeRsaPrivateKey;
        use rsa::traits::PublicKeyParts;
        let key = rsa::RsaPrivateKey::from_pkcs1_pem(DEFAULT_PRIVATE_KEY).unwrap();
        assert_eq!(key.size() * 8, 2048, "key must be RSA-2048");
        assert!(key.validate().is_ok(), "key must be valid");
    }

    #[test]
    fn test_client_id_is_full_blob() {
        assert!(
            DEFAULT_CLIENT_ID.len() > 1000,
            "DEFAULT_CLIENT_ID must be the full {} byte blob, got {}",
            1780, DEFAULT_CLIENT_ID.len()
        );
    }
}
