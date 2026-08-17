//! Safe wrapper over the system Widevine CDM.
//!
//! Drives a `libwidevinecdm` borrowed from an installed browser through its
//! official `cdm::ContentDecryptionModule` ABI (see `shim/widevine_shim.cc`) to
//! generate a license challenge and decrypt CENC samples.

use std::path::Path;

#[cfg(not(target_os = "android"))]
use std::ffi::CString;
#[cfg(not(target_os = "android"))]
use std::os::raw::{c_char, c_int};
#[cfg(not(target_os = "android"))]
use std::sync::{Arc, OnceLock};

pub mod discover;

// MediaDrm API is TODO
#[cfg(not(target_os = "android"))]
unsafe extern "C" {
    fn wv_open(so_path: *const c_char) -> c_int;
    fn wv_challenge(
        init_data: *const u8,
        len: u32,
        out: *mut *mut u8,
        out_len: *mut u32,
        out_type: *mut u32,
    ) -> c_int;
    fn wv_update(license: *const u8, len: u32) -> c_int;
    #[allow(clippy::too_many_arguments)]
    fn wv_decrypt(
        data: *const u8,
        data_size: u32,
        key_id: *const u8,
        key_id_size: u32,
        iv: *const u8,
        iv_size: u32,
        subs: *const u32,
        num_subs: u32,
        out: *mut *mut u8,
        out_len: *mut u32,
    ) -> c_int;
    fn wv_free(p: *mut u8);
}

/// Serializes the licence exchange only — `challenge` through `update`.
///
/// The shim tracks a single "current session id", so two tracks interleaving
/// their challenge/update pairs would load one's licence into the other's
/// session. Held across the licence HTTP round-trip (~0.5s), hence async.
///
/// Deliberately NOT held for the life of a track. A Widevine CDM retains the
/// keys of every session it is given and selects one per `Decrypt` by key id, so
/// tracks decrypt concurrently. Holding it longer made a track's background
/// decrypt block every later play until it finished.
#[cfg(not(target_os = "android"))]
static LICENSE_LOCK: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();

#[cfg(not(target_os = "android"))]
fn license_lock() -> Arc<tokio::sync::Mutex<()>> {
    LICENSE_LOCK
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Serializes individual calls into the native CDM, which is one process-wide
/// instance and not re-entrant. Each hold is a single operation (~1.6ms for a
/// decrypt), never a whole track.
#[cfg(not(target_os = "android"))]
static CALL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(not(target_os = "android"))]
fn call_lock() -> std::sync::MutexGuard<'static, ()> {
    CALL_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Proof the holder owns the CDM's session slot, for the span between generating
/// a challenge and loading the licence it returns.
#[cfg(not(target_os = "android"))]
pub struct LicenseSession(#[allow(dead_code)] tokio::sync::OwnedMutexGuard<()>);

#[cfg(target_os = "android")]
pub struct LicenseSession;

/// Copy a shim-allocated buffer into a `Vec` and release it.
///
/// # Safety
/// `out` must be a non-null buffer of `len` bytes returned by the shim.
#[cfg(not(target_os = "android"))]
unsafe fn take(out: *mut u8, len: u32) -> Vec<u8> {
    if out.is_null() {
        return Vec::new();
    }
    let v = unsafe { std::slice::from_raw_parts(out, len as usize) }.to_vec();
    unsafe { wv_free(out) };
    v
}

/// A handle to the process-wide system Widevine CDM.
///
/// Cheap and non-exclusive: holding one grants no ownership of the CDM. Loading
/// is idempotent, and per-call locking keeps the non-re-entrant native instance
/// safe, so several tracks can hold one at once.
pub struct Cdm {
    _private: (),
}

/// Android: Widevine lives behind `MediaDrm`, which is a separate implementation.
/// Until that lands, Apple Music playback reports why rather than failing at link
/// time or playing silence.
#[cfg(target_os = "android")]
impl Cdm {
    pub fn open(_path: impl AsRef<Path>) -> Result<Self, String> {
        Err("Apple Music playback isn't supported on Android yet (needs MediaDrm)".to_string())
    }

    pub fn open_system() -> Result<Self, String> {
        Self::open("")
    }

    pub async fn begin_license(&self) -> LicenseSession {
        LicenseSession
    }

    pub fn challenge(
        &self,
        _session: &LicenseSession,
        _pssh_box: &[u8],
    ) -> Result<Vec<u8>, String> {
        Self::open("").map(|_| Vec::new())
    }

    pub fn update(&self, _session: &LicenseSession, _license: &[u8]) -> Result<(), String> {
        Self::open("").map(|_| ())
    }

    pub fn decrypt(
        &self,
        _data: &[u8],
        _key_id: &[u8],
        _iv: &[u8],
        _subsamples: &[(u32, u32)],
    ) -> Result<Vec<u8>, String> {
        Self::open("").map(|_| Vec::new())
    }
}

#[cfg(not(target_os = "android"))]
impl Cdm {
    /// Take an exclusive session on the CDM at `path`, loading it if needed.
    ///
    /// Waits for any track already using the CDM to finish.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let s = path
            .to_str()
            .ok_or_else(|| format!("non-UTF-8 CDM path: {}", path.display()))?;
        let c = CString::new(s).map_err(|e| format!("CDM path has an interior NUL: {e}"))?;

        let _guard = call_lock();
        match unsafe { wv_open(c.as_ptr()) } {
            0 => Ok(Self { _private: () }),
            1 => Err(format!(
                "couldn't load the Widevine CDM at {} — the file may be corrupt or built for another architecture",
                path.display()
            )),
            2 => Err(format!(
                "{} isn't a Widevine CDM (missing CreateCdmInstance)",
                path.display()
            )),
            3 => Err("this Widevine CDM doesn't support the CDM-11 interface".to_string()),
            n => Err(format!("Widevine CDM failed to initialize (code {n})")),
        }
    }

    /// Locate a CDM from an installed browser and open it.
    pub async fn open_system() -> Result<Self, String> {
        let path = discover::locate().ok_or_else(|| {
            "no Widevine CDM found. Apple Music playback borrows one from an installed browser — \
             install Firefox (or Chrome/Brave) and play any DRM video once so it downloads the CDM, \
             or set $KOPUZ_WIDEVINE_CDM to a libwidevinecdm library"
                .to_string()
        })?;
        Self::open(path).await
    }

    /// Claim the CDM's session slot for one licence exchange.
    pub async fn begin_license(&self) -> LicenseSession {
        LicenseSession(license_lock().lock_owned().await)
    }

    /// `cdm::MessageType::kLicenseRequest` — the only message that is a licence
    /// challenge. `kIndividualizationRequest` (3) is a provisioning request for
    /// Google's server and means the CDM has no device certificate yet.
    const LICENSE_REQUEST: u32 = 0;
    const INDIVIDUALIZATION_REQUEST: u32 = 3;

    /// Generate a license challenge from a CENC pssh box.
    pub fn challenge(&self, _session: &LicenseSession, pssh_box: &[u8]) -> Result<Vec<u8>, String> {
        let _guard = call_lock();
        let mut out = std::ptr::null_mut();
        let mut len = 0u32;
        let mut msg_type = 0u32;
        match unsafe {
            wv_challenge(
                pssh_box.as_ptr(),
                pssh_box.len() as u32,
                &mut out,
                &mut len,
                &mut msg_type,
            )
        } {
            0 => {
                let message = unsafe { take(out, len) };
                // Posting the wrong message type to Apple's licence server gets
                // an opaque numeric rejection, so name the cause here instead.
                match msg_type {
                    Self::LICENSE_REQUEST => Ok(message),
                    Self::INDIVIDUALIZATION_REQUEST => Err(
                        "this Widevine CDM has no device certificate yet — it wants to provision \
                         itself first, which kopuz can't do. Play any DRM video (Netflix, Spotify \
                         web, a Widevine test page) in the browser the CDM came from, then retry"
                            .to_string(),
                    ),
                    other => Err(format!(
                        "the CDM returned message type {other}, not a license request"
                    )),
                }
            }
            11 => Err("the CDM rejected the pssh box".to_string()),
            12 => Err("the CDM produced no license challenge".to_string()),
            n => Err(format!("license challenge failed (code {n})")),
        }
    }

    /// Feed the license response back so the CDM loads the content keys.
    pub fn update(&self, _session: &LicenseSession, license: &[u8]) -> Result<(), String> {
        let _guard = call_lock();
        match unsafe { wv_update(license.as_ptr(), license.len() as u32) } {
            0 => Ok(()),
            21 => Err("the CDM rejected the license response".to_string()),
            22 => Err("the license response carried no usable key".to_string()),
            n => Err(format!("loading the license failed (code {n})")),
        }
    }

    /// Decrypt one CENC sample.
    ///
    /// `subsamples` is `(clear, encrypted)` byte counts; empty means the whole
    /// buffer is encrypted. Call [`update`](Self::update) first.
    pub fn decrypt(
        &self,
        data: &[u8],
        key_id: &[u8],
        iv: &[u8],
        subsamples: &[(u32, u32)],
    ) -> Result<Vec<u8>, String> {
        let mut subs: Vec<u32> = Vec::with_capacity(subsamples.len() * 2);
        for &(clear, encrypted) in subsamples {
            subs.push(clear);
            subs.push(encrypted);
        }

        let _guard = call_lock();
        let mut out = std::ptr::null_mut();
        let mut len = 0u32;
        let rc = unsafe {
            wv_decrypt(
                data.as_ptr(),
                data.len() as u32,
                key_id.as_ptr(),
                key_id.len() as u32,
                iv.as_ptr(),
                iv.len() as u32,
                subs.as_ptr(),
                subsamples.len() as u32,
                &mut out,
                &mut len,
            )
        };
        match rc {
            0 => Ok(unsafe { take(out, len) }),
            // 31 + Status::kNoKey — the usual cause is a license that didn't
            // cover this track's key id.
            32 => Err("no key for this track (the license didn't cover its key id)".to_string()),
            n => Err(format!("CENC decrypt failed (code {n})")),
        }
    }
}

/// The `WidevineCencHeader` protobuf that goes inside the pssh box.
///
/// Written out by hand rather than generated. It is six fields, only one of them
/// ever non-empty, and generating it cost a `protoc` on every build machine —
/// which nix and the Windows runner don't have, so the whole crate failed to
/// build there. Twenty bytes of wire format is the cheaper side of that trade.
///
/// Field numbers and types come from `WidevineCencHeader`: 1 `algorithm`
/// (varint), 2 `key_id` (repeated bytes), 3 `provider` and 6 `policy` (strings,
/// sent empty). Fields are written in tag order, as protobuf encoders do.
fn widevine_cenc_header(key_id: &[u8]) -> Vec<u8> {
    fn varint(out: &mut Vec<u8>, mut value: u64) {
        while value >= 0x80 {
            out.push(value as u8 | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
    }

    let mut out = Vec::with_capacity(key_id.len() + 10);
    out.extend_from_slice(&[0x08, 0x01]); // 1: algorithm = AESCTR
    out.push(0x12); // 2: key_id, length-delimited
    varint(&mut out, key_id.len() as u64);
    out.extend_from_slice(key_id);
    out.extend_from_slice(&[0x1a, 0x00]); // 3: provider = ""
    out.extend_from_slice(&[0x32, 0x00]); // 6: policy = ""
    out
}

/// Build a CENC pssh box for `key_id`, the init data a CDM expects.
///
/// The previous hand-rolled CDM was handed 32 bytes of filler followed by a bare
/// `WidevineCencHeader` and skipped past the filler; a real CDM parses this as an
/// actual ISO-BMFF box, so it has to be one.
pub fn build_pssh(key_id: &[u8]) -> Vec<u8> {
    /// `edef8ba9-79d6-4ace-a3c8-27dcd51d21ed`
    const WIDEVINE_SYSTEM_ID: [u8; 16] = [
        0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d, 0x21,
        0xed,
    ];

    let payload = widevine_cenc_header(key_id);

    // size | 'pssh' | version+flags | system id | data size | data
    let total = 4 + 4 + 4 + 16 + 4 + payload.len();
    let mut box_ = Vec::with_capacity(total);
    box_.extend_from_slice(&(total as u32).to_be_bytes());
    box_.extend_from_slice(b"pssh");
    box_.extend_from_slice(&[0, 0, 0, 0]); // version 0, no flags
    box_.extend_from_slice(&WIDEVINE_SYSTEM_ID);
    box_.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    box_.extend_from_slice(&payload);
    box_
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header the CDM parses out of the pssh box. Pinned byte-for-byte
    /// because it used to be generated from `wv.proto` by `protoc`; these are
    /// exactly the bytes prost emitted for the same message.
    #[test]
    fn cenc_header_is_encoded_as_protobuf() {
        assert_eq!(
            widevine_cenc_header(&[0xAA; 4]),
            vec![
                0x08, 0x01, // 1: algorithm = AESCTR
                0x12, 0x04, 0xAA, 0xAA, 0xAA, 0xAA, // 2: key_id
                0x1a, 0x00, // 3: provider = ""
                0x32, 0x00, // 6: policy = ""
            ]
        );
    }

    /// Key ids longer than 127 bytes need a multi-byte length varint.
    #[test]
    fn a_long_key_id_gets_a_multibyte_length() {
        let encoded = widevine_cenc_header(&[0u8; 200]);
        assert_eq!(&encoded[2..5], &[0x12, 0xC8, 0x01], "200 as a varint");
        assert_eq!(encoded.len(), 2 + 3 + 200 + 2 + 2);
    }

    #[test]
    fn pssh_box_is_well_formed() {
        let kid = [0xAAu8; 16];
        let b = build_pssh(&kid);

        let declared = u32::from_be_bytes(b[0..4].try_into().unwrap()) as usize;
        assert_eq!(declared, b.len(), "size field must cover the whole box");
        assert_eq!(&b[4..8], b"pssh");
        assert_eq!(&b[8..12], &[0, 0, 0, 0], "version 0, no flags");
        assert_eq!(
            &b[12..28],
            &[
                0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d,
                0x21, 0xed
            ],
            "Widevine system id"
        );

        let data_len = u32::from_be_bytes(b[28..32].try_into().unwrap()) as usize;
        assert_eq!(data_len, b.len() - 32, "data size must match the payload");
        // The key id must survive into the protobuf payload.
        assert!(b[32..].windows(16).any(|w| w == kid));
    }

    /// End-to-end check against a real CDM: load it and generate a licence
    /// challenge. Needs Google's proprietary binary, so it can't run in CI —
    /// point `$KOPUZ_WIDEVINE_CDM` at one (or at a browser profile) and run
    /// `cargo test -p kopuz-server -- --ignored` to exercise the C++ shim.
    #[tokio::test]
    #[ignore = "needs a system Widevine CDM"]
    #[cfg(not(target_os = "android"))]
    async fn challenge_against_a_real_cdm() {
        let cdm = Cdm::open_system().await.expect("open a system CDM");
        let session = cdm.begin_license().await;
        let challenge = cdm
            .challenge(&session, &build_pssh(&[0x11u8; 16]))
            .expect("generate a challenge");

        assert!(!challenge.is_empty(), "challenge must not be empty");
        // A ChromeCDM challenge carries the device's model name; its presence
        // means the CDM signed with its own sealed key rather than erroring out.
        assert!(
            challenge.windows(9).any(|w| w == b"ChromeCDM"),
            "challenge should be a ChromeCDM SignedMessage ({} bytes)",
            challenge.len()
        );
    }
}
