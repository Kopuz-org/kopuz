//! `kopuz.crypto`: the digests and encodings backend auth schemes are built from.
//!
//! Every argument and every result is a Lua byte string, not text, so a plugin can
//! hash a raw response body or feed a decoded key straight into an HMAC without a
//! UTF-8 detour. The hex functions and the digests return lowercase hex, which is
//! what a signed query string almost always wants.
//!
//! What is deliberately missing: symmetric ciphers and key derivation. Nothing a
//! music backend asks for needs them, and a plugin that thinks it does is
//! probably about to store a secret it should have kept in `kopuz.store`.

use std::sync::Arc;

use base64::Engine as _;
use base64::alphabet;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{
    GeneralPurpose, GeneralPurposeConfig, STANDARD, URL_SAFE_NO_PAD,
};
use mlua::{Lua, Table};
use rand::Rng as _;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use super::HostCtx;

/// Block size of both SHA-1 and SHA-256, which is what HMAC pads the key to.
const HMAC_BLOCK: usize = 64;

/// Ceiling on `random_bytes`. Well past a nonce or a PKCE verifier, and small
/// enough that a loop asking for entropy cannot be a memory problem.
const MAX_RANDOM_BYTES: i64 = 1024;

/// Decoding is tolerant about padding in both alphabets: real services emit
/// unpadded standard base64 and padded base64url about as often as the RFC forms,
/// and rejecting them would only push every plugin into re-padding by hand.
const DECODE_STANDARD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

const DECODE_URL_SAFE: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Digests, HMACs, base64, hex and random bytes.
///
/// | function | result |
/// | --- | --- |
/// | `md5(s)`, `sha1(s)`, `sha256(s)` | lowercase hex |
/// | `hmac_sha1(key, msg)`, `hmac_sha256(key, msg)` | lowercase hex |
/// | `base64_encode(s)` | standard alphabet, padded |
/// | `base64url_encode(s)` | url-safe alphabet, unpadded, as OAuth and JWT want |
/// | `base64_decode(s)`, `base64url_decode(s)` | bytes, tolerant of padding either way |
/// | `hex_encode(s)`, `hex_decode(s)` | lowercase hex, and bytes back |
/// | `random_bytes(n)` | `n` bytes from the OS, `n` capped at 1024 |
///
/// A decode function raises `kopuz:invalid_input` on input that is not in the
/// alphabet it expects.
///
/// ```lua
/// local sig = kopuz.crypto.hmac_sha1(secret, kopuz.url.build_query(params))
/// ```
pub(super) fn module(lua: &Lua, _ctx: &Arc<HostCtx>) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    table.set(
        "md5",
        lua.create_function(|_, s: mlua::LuaString| {
            Ok(format!("{:x}", md5::compute(super::bytes_of(&s))))
        })?,
    )?;
    table.set(
        "sha1",
        lua.create_function(|_, s: mlua::LuaString| {
            Ok(hex::encode(Sha1::digest(super::bytes_of(&s))))
        })?,
    )?;
    table.set(
        "sha256",
        lua.create_function(|_, s: mlua::LuaString| {
            Ok(hex::encode(Sha256::digest(super::bytes_of(&s))))
        })?,
    )?;

    table.set(
        "hmac_sha1",
        lua.create_function(|_, (key, msg): (mlua::LuaString, mlua::LuaString)| {
            Ok(hex::encode(hmac::<Sha1>(
                &super::bytes_of(&key),
                &super::bytes_of(&msg),
            )))
        })?,
    )?;
    table.set(
        "hmac_sha256",
        lua.create_function(|_, (key, msg): (mlua::LuaString, mlua::LuaString)| {
            Ok(hex::encode(hmac::<Sha256>(
                &super::bytes_of(&key),
                &super::bytes_of(&msg),
            )))
        })?,
    )?;

    table.set(
        "base64_encode",
        lua.create_function(|_, s: mlua::LuaString| Ok(STANDARD.encode(super::bytes_of(&s))))?,
    )?;
    table.set(
        "base64url_encode",
        lua.create_function(|_, s: mlua::LuaString| {
            Ok(URL_SAFE_NO_PAD.encode(super::bytes_of(&s)))
        })?,
    )?;
    table.set(
        "base64_decode",
        lua.create_function(|lua, s: mlua::LuaString| {
            lua.create_string(base64_decode(&DECODE_STANDARD, &super::bytes_of(&s))?)
        })?,
    )?;
    table.set(
        "base64url_decode",
        lua.create_function(|lua, s: mlua::LuaString| {
            lua.create_string(base64_decode(&DECODE_URL_SAFE, &super::bytes_of(&s))?)
        })?,
    )?;

    table.set(
        "hex_encode",
        lua.create_function(|_, s: mlua::LuaString| Ok(hex::encode(super::bytes_of(&s))))?,
    )?;
    table.set(
        "hex_decode",
        lua.create_function(|lua, s: mlua::LuaString| {
            let bytes = hex::decode(super::bytes_of(&s))
                .map_err(|e| super::invalid_input(format!("not hex: {e}")))?;
            lua.create_string(bytes)
        })?,
    )?;

    table.set(
        "random_bytes",
        lua.create_function(|lua, n: i64| lua.create_string(random_bytes(n)?))?,
    )?;

    Ok(table)
}

/// HMAC (RFC 2104) over any `Digest` whose block size is [`HMAC_BLOCK`].
///
/// Hand-rolled rather than pulling in the `hmac` crate: it is nine lines, and the
/// two hashes a music backend ever asks for share one block size.
fn hmac<D: Digest>(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut block = [0u8; HMAC_BLOCK];
    if key.len() > HMAC_BLOCK {
        let hashed = D::digest(key);
        block[..hashed.len()].copy_from_slice(&hashed);
    } else {
        block[..key.len()].copy_from_slice(key);
    }

    let mut inner = D::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finalize();

    let mut outer = D::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().to_vec()
}

fn base64_decode(engine: &GeneralPurpose, input: &[u8]) -> mlua::Result<Vec<u8>> {
    engine
        .decode(input)
        .map_err(|e| super::invalid_input(format!("not base64: {e}")))
}

fn random_bytes(n: i64) -> mlua::Result<Vec<u8>> {
    if n < 0 {
        return Err(super::invalid_input(
            "random_bytes needs a non-negative count",
        ));
    }
    let mut buf = vec![0u8; n.min(MAX_RANDOM_BYTES) as usize];
    rand::rng().fill_bytes(&mut buf);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 2202 test vectors for HMAC-SHA1: a short key, an ASCII key, and a key
    /// longer than the block size (which exercises the hash-the-key branch).
    #[test]
    fn hmac_sha1_matches_rfc_2202() {
        assert_eq!(
            hex::encode(hmac::<Sha1>(&[0x0b; 20], b"Hi There")),
            "b617318655057264e28bc0b6fb378c8ef146be00"
        );
        assert_eq!(
            hex::encode(hmac::<Sha1>(b"Jefe", b"what do ya want for nothing?")),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
        assert_eq!(
            hex::encode(hmac::<Sha1>(
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "aa4ae5e15272d00e95705637ce8a3b55ed402112"
        );
    }

    /// RFC 4231 test vectors 1, 2 and 6 for HMAC-SHA256.
    #[test]
    fn hmac_sha256_matches_rfc_4231() {
        assert_eq!(
            hex::encode(hmac::<Sha256>(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex::encode(hmac::<Sha256>(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hex::encode(hmac::<Sha256>(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn digests_match_their_published_vectors() {
        assert_eq!(
            format!("{:x}", md5::compute(b"abc")),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hex::encode(Sha1::digest(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            hex::encode(Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hex_round_trips_binary() {
        let bytes: Vec<u8> = (0..=u8::MAX).collect();
        let encoded = hex::encode(&bytes);
        assert_eq!(hex::decode(&encoded).expect("decode"), bytes);
        assert!(hex::decode("nothex").is_err());
    }

    #[test]
    fn base64_round_trips_binary_in_both_alphabets() {
        let bytes: Vec<u8> = (0..=u8::MAX).collect();

        let standard = STANDARD.encode(&bytes);
        assert_eq!(
            base64_decode(&DECODE_STANDARD, standard.as_bytes()).expect("decode"),
            bytes
        );

        let url = URL_SAFE_NO_PAD.encode(&bytes);
        assert!(!url.contains('+') && !url.contains('/') && !url.contains('='));
        assert_eq!(
            base64_decode(&DECODE_URL_SAFE, url.as_bytes()).expect("decode"),
            bytes
        );
    }

    /// Padding is the thing services disagree about most, so both decoders take it
    /// or leave it.
    #[test]
    fn decoding_tolerates_padding_either_way() {
        assert_eq!(
            base64_decode(&DECODE_STANDARD, b"aGk").expect("unpadded"),
            b"hi"
        );
        assert_eq!(
            base64_decode(&DECODE_URL_SAFE, b"aGk=").expect("padded"),
            b"hi"
        );
        assert!(base64_decode(&DECODE_STANDARD, b"not base64!").is_err());
    }

    #[test]
    fn random_bytes_is_capped_and_rejects_a_negative_count() {
        assert_eq!(random_bytes(16).expect("bytes").len(), 16);
        assert_eq!(
            random_bytes(4096).expect("capped").len(),
            MAX_RANDOM_BYTES as usize
        );
        assert!(random_bytes(-1).is_err());
    }
}
