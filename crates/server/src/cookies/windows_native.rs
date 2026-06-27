//! Windows cookie decryption for kopuz's isolated browser profile.
//!
//! Chrome currently writes **v10/v11** cookies (legacy DPAPI) even for the
//! signed-in Google/YouTube auth cookies — App-Bound Encryption (the v20 tier)
//! is generated but Finch-gated off for our fresh isolated profile. We decrypt
//! v10 with the profile's DPAPI key (non-admin `CryptUnprotectData`).
//!
//! As insurance against Google flipping the v20 rollout on, we also plant a
//! `PROTECTION_NONE` app-bound key (which Chrome accepts) before launch and
//! stash it DPAPI-wrapped in the profile; if a `v20` cookie ever appears we
//! decrypt it with that key. No admin, no process injection, no read-time COM.

use std::path::Path;

use base64::Engine;
use config::Browser;
use windows::Win32::Foundation::{SysAllocStringByteLen, SysStringByteLen};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, EOAC_DYNAMIC_CLOAKING, RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_DEFAULT, RPC_C_AUTHZ_DEFAULT};
use windows::core::{BSTR, GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, interface};

use super::store::Cookie;

// Chrome's elevation-service COM interface. Vtable after IUnknown is
// RunRecoveryCRXElevated, EncryptData, DecryptData — we only call EncryptData
// (slot 4). The trait IID is the base IElevator; we QI the brand IID at runtime.
#[interface("A949CB4E-C4F9-44C4-B213-6BF8AA9AC69C")]
unsafe trait IElevator: IUnknown {
    unsafe fn run_recovery_crx_elevated(
        &self,
        crx_path: *const u16,
        browser_appid: *const u16,
        browser_version: *const u16,
        session_id: *const u16,
        caller_proc_id: u32,
        proc_handle: *mut usize,
    ) -> HRESULT;
    unsafe fn encrypt_data(
        &self,
        protection_level: u32,
        plaintext: BSTR,
        ciphertext: *mut BSTR,
        last_error: *mut u32,
    ) -> HRESULT;
    unsafe fn decrypt_data(
        &self,
        ciphertext: BSTR,
        plaintext: *mut BSTR,
        last_error: *mut u32,
    ) -> HRESULT;
}

const PROTECTION_NONE: u32 = 0;
const ABE_KEY_FILE: &str = ".kopuz-abe";

/// (elevation CLSID, candidate IElevator IIDs newest-first). Chrome 149 rotated
/// to IElevator2Chrome (== the elevation typelib GUID); the older IElevatorChrome
/// IID is kept as a fallback for pre-149. Brands without an elevation service
/// (Chromium/Vivaldi) return None — the plant is skipped, v10 still works.
fn brand_elevation(browser: Browser) -> Option<(u128, &'static [u128])> {
    match browser {
        Browser::Chrome => Some((
            0x708860E0_F641_4611_8895_7D867DD3675B,
            &[
                0x1BF5208B_295F_4992_B5F4_3A9BB6494838,
                0x463ABECF_410D_407F_8AF5_0DF35A005CC8,
            ],
        )),
        Browser::Edge => Some((
            0x1FCBE96C_1697_43AF_9140_2897C7C69767,
            &[0xC9C2B807_7731_4F34_81B7_44FF7779522B],
        )),
        Browser::Brave => Some((
            0x576B31AF_6369_4B6B_8560_E4B203A97A8B,
            &[0xF396861E_0C8E_4C71_8256_2FAE6D759CE9],
        )),
        Browser::Chromium | Browser::Vivaldi => None,
    }
}

fn elevator_encrypt(browser: Browser, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let (clsid, iids) = brand_elevation(browser).ok_or("no elevation service for browser")?;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let clsid = GUID::from_u128(clsid);
        let factory: IUnknown = CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER)
            .map_err(|e| format!("CoCreateInstance: {e}"))?;
        // QI the first registered brand interface (vtable layout-compatible).
        let mut unk: Option<IElevator> = None;
        for iid in iids {
            let mut raw = core::ptr::null_mut();
            if factory.query(&GUID::from_u128(*iid), &mut raw).is_ok() && !raw.is_null() {
                unk = Some(IElevator::from_raw(raw));
                break;
            }
        }
        let unk =
            unk.ok_or("no registered IElevator interface (Chrome version rotated the IID?)")?;
        CoSetProxyBlanket(
            &unk.cast::<IUnknown>().map_err(|e| e.to_string())?,
            RPC_C_AUTHN_DEFAULT as u32,
            RPC_C_AUTHZ_DEFAULT,
            None,
            RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_DYNAMIC_CLOAKING,
        )
        .map_err(|e| format!("CoSetProxyBlanket: {e}"))?;

        let pt = SysAllocStringByteLen(Some(plaintext));
        let mut ct = BSTR::default();
        let mut last_err: u32 = 0;
        let hr = unk.encrypt_data(PROTECTION_NONE, pt, &mut ct, &mut last_err);
        if hr.is_err() {
            return Err(format!("EncryptData hr={hr:?} last_error={last_err}"));
        }
        let len = SysStringByteLen(&ct) as usize;
        let ptr = ct.as_ptr() as *const u8;
        if ptr.is_null() || len == 0 {
            return Err("EncryptData returned empty".into());
        }
        Ok(std::slice::from_raw_parts(ptr, len).to_vec())
    }
}

fn dpapi(data: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    unsafe {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB::default();
        let res = if protect {
            CryptProtectData(&in_blob, None, None, None, None, 0, &mut out)
        } else {
            CryptUnprotectData(&in_blob, None, None, None, None, 0, &mut out)
        };
        res.map_err(|e| {
            format!(
                "DPAPI {}: {e}",
                if protect { "protect" } else { "unprotect" }
            )
        })?;
        Ok(std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec())
    }
}

fn os_crypt_field(profile_root: &Path, field: &str) -> Option<String> {
    let txt = std::fs::read_to_string(profile_root.join("Local State")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&txt).ok()?;
    v.get("os_crypt")?.get(field)?.as_str().map(str::to_owned)
}

/// The legacy v10/v11 AES key: base64( "DPAPI" | CryptProtectData(key) ).
fn load_v10_key(profile_root: &Path) -> Option<Vec<u8>> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(os_crypt_field(profile_root, "encrypted_key")?)
        .ok()?;
    let stripped = raw.strip_prefix(b"DPAPI")?;
    dpapi(stripped, false).ok()
}

/// True while the browser holds the profile's cookie store open — an exclusive
/// open then fails with `ERROR_SHARING_VIOLATION`.
pub(crate) fn cookie_db_locked(profile_root: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    let p = profile_root.join("Default").join("Network").join("Cookies");
    if !p.exists() {
        return false;
    }
    // share_mode(0) = deny-all; 32 = ERROR_SHARING_VIOLATION.
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&p)
    {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(32),
    }
}

/// Recover the planted app-bound (v20) key we DPAPI-wrapped at plant time.
fn load_v20_key(profile_root: &Path) -> Option<Vec<u8>> {
    let wrapped = std::fs::read(profile_root.join(ABE_KEY_FILE)).ok()?;
    dpapi(&wrapped, false).ok()
}

/// Mint a random app-bound key, seal it via the elevation service with
/// PROTECTION_NONE, plant it into the fresh profile's `Local State`, and stash
/// the plaintext DPAPI-wrapped so a later read can decrypt v20 cookies. Must run
/// AFTER the profile dir exists and BEFORE the browser launches. Best-effort:
/// failure is non-fatal (v10 cookies still decrypt without it).
pub(crate) fn plant_app_bound_key(browser: Browser, profile_root: &Path) -> Result<(), String> {
    if brand_elevation(browser).is_none() {
        return Ok(());
    }
    let k: [u8; 32] = rand::random();
    let sealed = elevator_encrypt(browser, &k)?;

    let mut planted = b"APPB".to_vec();
    planted.extend_from_slice(&sealed);
    let planted_b64 = base64::engine::general_purpose::STANDARD.encode(&planted);

    let ls_path = profile_root.join("Local State");
    let mut ls: serde_json::Value = std::fs::read_to_string(&ls_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !ls.is_object() {
        ls = serde_json::json!({});
    }
    let oc = ls
        .as_object_mut()
        .unwrap()
        .entry("os_crypt")
        .or_insert_with(|| serde_json::json!({}));
    oc.as_object_mut()
        .ok_or("os_crypt not an object")?
        .insert("app_bound_encrypted_key".into(), planted_b64.into());
    if let Some(parent) = ls_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        &ls_path,
        serde_json::to_vec(&ls).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write Local State: {e}"))?;

    let wrapped = dpapi(&k, true)?;
    std::fs::write(profile_root.join(ABE_KEY_FILE), wrapped)
        .map_err(|e| format!("stash app-bound key: {e}"))?;
    Ok(())
}

/// Chrome cookie-encryption tier, by the `encrypted_value` tag prefix.
enum Scheme {
    /// `v10`/`v11` — legacy DPAPI key.
    Dpapi,
    /// `v20` — planted app-bound key.
    AppBound,
}

impl Scheme {
    fn from_tag(tag: &[u8]) -> Option<Self> {
        match tag {
            b"v10" | b"v11" => Some(Self::Dpapi),
            b"v20" => Some(Self::AppBound),
            _ => None,
        }
    }
}

/// Decrypt one cookie value: `<tag> | nonce[12] | ct+tag`, AES-256-GCM, then drop
/// the 32-byte SHA(domain) prefix current Chrome adds to both tiers.
fn decrypt_value(enc: &[u8], dpapi: Option<&[u8]>, app_bound: Option<&[u8]>) -> Option<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    if enc.len() < 3 + 12 + 16 {
        return None;
    }
    let key = match Scheme::from_tag(&enc[..3])? {
        Scheme::Dpapi => dpapi?,
        Scheme::AppBound => app_bound?,
    };
    if key.len() != 32 {
        return None;
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let pt = cipher
        .decrypt(Nonce::from_slice(&enc[3..15]), &enc[15..])
        .ok()?;
    Some(String::from_utf8_lossy(pt.get(32..).unwrap_or(&pt)).into_owned())
}

/// Copy the (possibly browser-locked) cookie store to a temp file and read every
/// cookie whose host is scoped to `domain`, decrypting v10/v20 values.
pub(crate) async fn read_cookies(
    _browser: Browser,
    profile_root: &Path,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    use sqlx::{ConnectOptions, Row};

    let src = profile_root.join("Default").join("Network").join("Cookies");
    if !src.exists() {
        return Err(format!("no Cookies store under {}", profile_root.display()));
    }
    // Snapshot-copy so we read consistently even while the browser holds it open.
    let tmp = std::env::temp_dir().join(format!("kopuz-ck-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let db = tmp.join("Cookies");
    for ext in ["", "-wal", "-shm", "-journal"] {
        let s = src.with_file_name(format!("Cookies{ext}"));
        if s.exists() {
            let _ = std::fs::copy(&s, tmp.join(format!("Cookies{ext}")));
        }
    }

    let dpapi = load_v10_key(profile_root);
    let app_bound = load_v20_key(profile_root);

    // Read-write (not read_only) so the rollback journal can recover if the
    // browser was killed mid-write.
    let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db)
        .create_if_missing(false)
        .connect()
        .await
        .map_err(|e| format!("open Cookies: {e}"))?;
    let rows = sqlx::query("SELECT host_key, name, value, encrypted_value FROM cookies")
        .fetch_all(&mut conn)
        .await
        .map_err(|e| format!("query cookies: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        let host: String = row.try_get("host_key").unwrap_or_default();
        if !host.contains(domain) {
            continue;
        }
        let name: String = row.try_get("name").unwrap_or_default();
        let plain: String = row.try_get("value").unwrap_or_default();
        let value = if !plain.is_empty() {
            plain
        } else {
            let enc: Vec<u8> = row.try_get("encrypted_value").unwrap_or_default();
            match decrypt_value(&enc, dpapi.as_deref(), app_bound.as_deref()) {
                Some(v) => v,
                None => continue,
            }
        };
        out.push(Cookie { name, value });
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(out)
}
