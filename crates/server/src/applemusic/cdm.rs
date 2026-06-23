#![allow(dead_code)]

use aes::cipher::{BlockDecryptMut, KeyIvInit};
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
    let value_start = *pos;
    match wire {
        0 => {
            decode_varint(data, pos)?;
        }
        2 => {
            let len = decode_varint(data, pos)? as usize;
            *pos += len;
        }
        _ => return None,
    }
    Some((field, wire, value_start))
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
    // algorithm: AESCTR = 1
    msg.extend_from_slice(&encode_varint_field(1, 1));
    // key_id (repeated bytes)
    msg.extend_from_slice(&encode_bytes_field(2, key_id));
    // content_id (bytes)
    msg.extend_from_slice(&encode_bytes_field(4, content_id_encoded.as_bytes()));
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
    encode_message_field(3, cenc)
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
    key_ids: Vec<Vec<u8>>,
    content_id: Vec<u8>,
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

        let (_key_ids, _content_id) = parse_widevine_cenc_header(&init_data[32..]);

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

        // We need the full key_ids from the header for the request
        let (key_ids, _content_id_inner) = parse_widevine_cenc_header(&init_data[32..]);

        Ok(Self {
            private_key: rsa::pkcs1v15::SigningKey::<Sha1>::new(private_key.clone()),
            private_key_raw: private_key,
            client_id,
            session_id,
            key_ids,
            content_id: _content_id_inner,
        })
    }

    pub fn new_default(init_data: &[u8]) -> Result<Self, String> {
        Self::new(DEFAULT_PRIVATE_KEY, DEFAULT_CLIENT_ID.to_vec(), init_data)
    }

    pub fn get_license_request(&self) -> Result<Vec<u8>, String> {
        let pssh_header = encode_widevine_cenc_header(
            self.key_ids.first().map(|k| k.as_slice()).unwrap_or(&[]),
            &String::from_utf8_lossy(&self.content_id),
        );

        let cenc = encode_content_identification_cenc(
            &pssh_header,
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
                continue;
            }

            // AES-CBC decrypt
            let mut cipher = Aes128CbcDec::new(
                &encryption_key,
                iv[..16].into(),
            );
            let mut decrypted = encrypted_key.clone();
            let end = 16.min(decrypted.len());
            let mut block = aes::Block::clone_from_slice(&decrypted[..end]);
            cipher.decrypt_block_mut(&mut block);
            decrypted[..end].copy_from_slice(&block);

            // PKCS7 unpad
            if let Some(&pad_len) = decrypted.last() {
                let pad_len = pad_len as usize;
                if pad_len > 0 && pad_len <= decrypted.len() {
                    decrypted.truncate(decrypted.len() - pad_len);
                }
            }

            keys.push(CdmKey {
                id,
                key_type,
                value: decrypted,
            });
        }

        Ok(keys)
    }
}

// ── Device constants ───────────────────────────────────────────────

pub const DEFAULT_PRIVATE_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA2bO3yvFwNnIHsbDl3MTjKdDsiBWsuZWOGVxInFWAVMp+nffG\nYlquTKpJurEry95yprcRB3hYhvA5ghsACidcWPDEPVqqRZ7YXLevyUA+Sn2Jxpvt\nOcwyFHbSwruNxprWOkHCT774O4L/wJUt5x2C4iFCrJByjw0omN8u+EHdavvH7ZPn\nb3/EZp/cpZa9/+HOkutvBHBvaPp18F8JQhzUQ9MwLuDFTr+QLDB5+Y57Je2tNYDK\nxD1K+Ed5Ja0A4OKhPKIwPwPre0nt5scjLba3LSAKtKxiGqFtWO4U7Tf1YrdjJv2o\n9o8Sf8qcnbpzvQ4KwFqehuJnB7+W7mdJJw12PQIDAQABAoIBACE32wOMc6LbI3Fp\nnKljIYZv6qeZJxHqUBRukGXKZhqKC2fvNsYrMA1irn1eK2CgQL5PkLmjE18DqMLB\ne/AQsXagxlDWVMTqx/jdzmTW+KpFHZDAmiIHllypBN/R3oA/gBDDl/KzIQ1zn7Kz\nEJ4DUsVObe4G3HQXfepVo8Udx7tbB7X6wHe2kEgFyY3lPdvubik0C4t4ipSD79y7\nSfW7XVA5XUQmqN4U2kWM0uSwzd4BA7hqyScJsygf6KgpMWPS2xFZEZQRUpYcBH48\nE7YqNrrlYP3yaQ+9Jx56kKS0mvv3vUXS7AfUbU8CiHwD9I3BGwswEUueOGGVeXbx\ntFF8s8ECgYEA97BDcL/bt+r3qJF0dxtMB5ZngJbFx9RdsblYepVpblr2UfxnFttO\nPoNSKa4W36HuDsun49dkaoABJWdtZs2Hy6q+xvEgozvhMaBVE3spnWnzCT1yTMYL\nG02uDEl0dPiTg116bVElaswtqMXvnnpbOTMTe7Ig9sWiUW/GH9RM+N8CgYEA4QHb\n+OA0BfczbVQP9B+plt4mAuu4BDm4GPwq1yXOWo3Ct8Ik+HeY1hqOObpfyQMAza+E\ne/kP6W8vXpiElGrmiUbTXK4Rzmf+yYeOrvl3D80bFq4GtDNAIQD3jpj6zjlT+Gzw\nI501gRx5iPl4fSccRSdpoeri7F9ANtc6EEGFyGMCgYEAjMznWYXHGkL47BtbkIW0\n769BQSj0X4dKh8gsEusylugglDSeSbD7RrASGd175T7A/CorU2rTC3OesyubVlBJ\n/K4gaykRe5mDh1l0Y3GlE3XyEXObsSb3k1rSMOvkxsWz3X5bJR923MIaxpFWiMlX\naCmvzqZQ9NceUZrvjpJ5+xMCgYAJa8KCESEcftUwZqykVA8Nug9tX+E8jA4hPa2t\nhG+3augUOZTCsn87t7Dsydjo2a9W7Vpmtm7sHzOkik5CyJcOeGCxKLimI8SPO5XF\nzbwmdTgFIxQ0x1CQETJMTityJwRVCnqjgxmSZlbQXWGmG9UbMCNEHEmUDAjsQuaz\nd4racQKBgQDR1Y2kalvleYGrhwcA8LTnIh0rYEfAt9YxNmTi5qDKf5QPvUP2v+WO\nfSB5coUqR8LBweHE5V8JgFt74fdLBqZV/k2z/dI0r+EQWmpZ2uPEC0Khk/Sb9iRD\nfH7at3PMusrkwZCGZ8beFEAr6icXclV08nPCNGB6WckacfzpAj8Azg==\n-----END RSA PRIVATE KEY-----";

pub const DEFAULT_CLIENT_ID: &[u8] = &[
    0x0a, 0x0b, 0x08, 0x01, 0x12, 0x00, 0x22, 0x25, 0x08, 0x03, 0x12, 0x21, 0x0a, 0x1f, 0x0a,
    0x10, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x5f, 0x77, 0x69, 0x64, 0x65, 0x76, 0x69, 0x6e,
    0x65, 0x0a, 0x0d, 0x41, 0x6e, 0x64, 0x72, 0x6f, 0x69, 0x64, 0x5f, 0x4f, 0x54, 0x54, 0x00,
    0x10, 0x01, 0x28, 0x01,
];

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_encoding() {
        assert_eq!(encode_varint(0), vec![0x00]);
        assert_eq!(encode_varint(1), vec![0x01]);
        assert_eq!(encode_varint(127), vec![0x7F]);
        assert_eq!(encode_varint(128), vec![0x80, 0x01]);
        assert_eq!(encode_varint(21), vec![0x15]);
    }

    #[test]
    fn test_cdm_creation() {
        let init_data = vec![0u8; 64];
        let cdm = Cdm::new(DEFAULT_PRIVATE_KEY, DEFAULT_CLIENT_ID.to_vec(), &init_data);
        assert!(cdm.is_ok());
    }

    #[test]
    fn test_license_request_generation() {
        let init_data = vec![0u8; 64];
        let cdm = Cdm::new(DEFAULT_PRIVATE_KEY, DEFAULT_CLIENT_ID.to_vec(), &init_data).unwrap();
        let request = cdm.get_license_request().unwrap();
        assert!(!request.is_empty());
    }
}
