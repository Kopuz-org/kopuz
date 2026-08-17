//! Resolving a Widevine CDM to download, rather than borrowing one from a
//! browser the user happens to have installed.
//!
//! Google publishes the CDM as a Chrome component, but only serves the
//! standalone package for Linux — on macOS and Windows it ships inside the
//! Chrome installer, and the component service answers `noupdate` for every
//! arch. Mozilla's GMP service covers all of them, and hands back URLs on
//! Google's own CDN: the same component packages, with the right one picked per
//! platform. It's the path Firefox itself takes, and it comes with a sha512, so
//! the download can be verified before anything is loaded into the process.
//!
//! [`ensure`] is the whole flow: reuse an already-downloaded CDM, else resolve,
//! download, check the hash, and unpack into the config dir. [`prefetch`] starts
//! that in the background as soon as an Apple Music source exists, so the
//! download isn't paid for with someone waiting on a track.

/// A Widevine CDM release, as the GMP manifest describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdmRelease {
    pub version: String,
    pub url: String,
    /// Lowercase hex sha512 of the `.crx3`. Verify before loading it.
    pub sha512: String,
    pub size: u64,
}

/// Mozilla's platform key for the host we're running on.
///
/// ARM is fine — macOS and Windows both publish arm64 builds. `None` covers the
/// platforms with no usable CDM:
///
/// - **ARM Linux**: none exists. Mozilla serves OpenH264 for `Linux_aarch64` but
///   no Widevine, so Firefox can't play Widevine content there either — meaning
///   there is no browser CDM to fall back to, not merely nothing to download.
///   The only known route is prising one out of a ChromeOS recovery image.
/// - **32-bit Linux**: frozen on a 2019 build, not worth offering.
pub fn gmp_platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "Linux_x86_64-gcc3",
        ("macos", "aarch64") => "Darwin_aarch64-gcc3",
        ("macos", "x86_64") => "Darwin_x86_64-gcc3-u-i386-x86_64",
        ("windows", "x86_64") => "WINNT_x86_64-msvc-x64",
        ("windows", "aarch64") => "WINNT_aarch64-msvc-aarch64",
        _ => return None,
    })
}

/// The manifest URL for `platform`.
///
/// The version and build id in the path are Firefox's, not ours — this is
/// Firefox's update endpoint and it wants to be told which Firefox is asking.
/// They only have to be recent enough to be offered the current CDM.
pub fn manifest_url(platform: &str) -> String {
    format!(
        "https://aus5.mozilla.org/update/3/GMP/140.0/20250801000000/{platform}/en-US/release/default/default/default/update.xml"
    )
}

/// Pull the `gmp-widevinecdm` entry out of a GMP manifest.
///
/// The manifest also lists OpenH264 and others, so the id has to be matched
/// rather than taking the first addon.
pub fn parse_manifest(xml: &str) -> Result<CdmRelease, String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("parse GMP manifest: {e}"))?;
        match event {
            Event::Empty(ref e) | Event::Start(ref e) if e.name().as_ref() == b"addon" => {
                let attr = |key: &[u8]| -> Option<String> {
                    e.attributes().flatten().find_map(|a| {
                        (a.key.as_ref() == key)
                            .then(|| String::from_utf8_lossy(a.value.as_ref()).into_owned())
                    })
                };
                if attr(b"id").as_deref() != Some("gmp-widevinecdm") {
                    buf.clear();
                    continue;
                }
                // A malformed entry is worth an error rather than a silent skip:
                // the alternative is downloading something unverifiable.
                let hash_fn = attr(b"hashFunction").unwrap_or_default();
                if hash_fn != "sha512" {
                    return Err(format!(
                        "unexpected hash function {hash_fn:?}, wanted sha512"
                    ));
                }
                return Ok(CdmRelease {
                    version: attr(b"version").ok_or("CDM entry has no version")?,
                    url: attr(b"URL").ok_or("CDM entry has no URL")?,
                    sha512: attr(b"hashValue")
                        .ok_or("CDM entry has no hash")?
                        .to_ascii_lowercase(),
                    size: attr(b"size")
                        .ok_or("CDM entry has no size")?
                        .parse()
                        .map_err(|e| format!("CDM entry size: {e}"))?,
                });
            }
            Event::Eof => return Err("no gmp-widevinecdm in the manifest".to_string()),
            _ => {}
        }
        buf.clear();
    }
}

/// Where the library sits inside the extracted `.crx3`.
///
/// Only a fallback for extraction to fall back *to*. The package ships a
/// `manifest.json` listing `platforms[].sub_package_path` keyed by os/arch, and
/// that is what a client is meant to select on — the Linux package is one bundle
/// for every arch, which is why the update service ignores the arch it's asked
/// for. Reading the manifest also means an arch Google adds later is picked up
/// for free: 4.10.3050.0 already declares `_platform_specific/linux_arm64/`
/// while shipping only x86-64.
pub fn archive_member() -> Option<String> {
    let (dir, name) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux_x64", "libwidevinecdm.so"),
        ("macos", "aarch64") => ("mac_arm64", "libwidevinecdm.dylib"),
        ("macos", "x86_64") => ("mac_x64", "libwidevinecdm.dylib"),
        ("windows", "x86_64") => ("win_x64", "widevinecdm.dll"),
        ("windows", "aarch64") => ("win_arm64", "widevinecdm.dll"),
        _ => return None,
    };
    Some(format!("_platform_specific/{dir}/{name}"))
}

/// Ask Mozilla which CDM this platform should use.
pub async fn resolve() -> Result<CdmRelease, String> {
    let platform = gmp_platform().ok_or_else(|| {
        format!(
            "no Widevine CDM is published for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let url = manifest_url(platform);
    tracing::debug!("am.widevine.fetch: resolving CDM for {platform}");

    let xml = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("fetch GMP manifest: {e}"))?
        .text()
        .await
        .map_err(|e| format!("read GMP manifest: {e}"))?;

    let release = parse_manifest(&xml)?;
    tracing::info!(
        "am.widevine.fetch: CDM {} available ({} MiB)",
        release.version,
        release.size / (1024 * 1024)
    );
    Ok(release)
}

/// Where fetched CDMs live: `<config>/widevine/<version>/<library>`.
///
/// The config dir (local app data on Windows), matching where the Apple Music
/// sign-in profile goes — this is something kopuz installs and keeps, not a
/// derived artefact it can regenerate at will.
pub fn install_root() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("com", "temidaradev", "kopuz").map(|d| {
        #[cfg(target_os = "windows")]
        let base = d.data_local_dir();
        #[cfg(not(target_os = "windows"))]
        let base = d.config_dir();
        base.join("widevine")
    })
}

/// The newest CDM already downloaded, if any. No network, no side effects.
pub fn installed() -> Option<std::path::PathBuf> {
    let name = super::discover::cdm_file_name();
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(install_root()?)
        .ok()?
        .flatten()
        .map(|e| e.path().join(name))
        .filter(|p| p.is_file())
        .collect();
    found.sort_by_key(|p| super::discover::version_key(p));
    found.pop()
}

/// Serializes downloads. Two tracks starting together would otherwise both pull
/// 20MB and race to install it.
static DOWNLOAD_LOCK: std::sync::OnceLock<std::sync::Arc<tokio::sync::Mutex<()>>> =
    std::sync::OnceLock::new();

/// A CDM on disk, downloading it first if there isn't one.
pub async fn ensure() -> Result<std::path::PathBuf, String> {
    if let Some(path) = installed() {
        tracing::debug!(path = %path.display(), "am.widevine.fetch: using downloaded CDM");
        return Ok(path);
    }

    let lock = DOWNLOAD_LOCK
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;
    // Another task may have finished while we waited for the lock.
    if let Some(path) = installed() {
        return Ok(path);
    }

    let release = resolve().await?;
    install(&release).await
}

/// Whether a background prefetch is worth starting.
///
/// Arms once per process whatever the answer, so a failed download doesn't get
/// retried on every source rebuild — [`ensure`] will try again when a track
/// actually needs the CDM.
fn should_prefetch() -> bool {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    // Nothing to do if it's already here, or if this platform has no CDM at all.
    installed().is_none() && gmp_platform().is_some()
}

/// Start fetching the CDM in the background, if it isn't already on disk.
///
/// Called when an Apple Music source is built rather than at first play. The
/// download is ~20MB, and the moment someone has pressed play is exactly when
/// they'd notice paying for it. Fire-and-forget: playback doesn't depend on this
/// finishing, because [`ensure`] covers the case where it hasn't.
pub fn prefetch() {
    if !should_prefetch() {
        return;
    }
    // Source construction is sync and isn't guaranteed to be inside a runtime;
    // `tokio::spawn` would panic there, so ask rather than assume.
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::debug!("am.widevine.fetch: no runtime to prefetch on, leaving it to playback");
        return;
    };
    handle.spawn(async {
        match ensure().await {
            Ok(path) => {
                tracing::info!(path = %path.display(), "am.widevine.fetch: CDM ready before playback")
            }
            Err(e) => tracing::warn!(
                "am.widevine.fetch: prefetch failed ({e}) — retrying when a track needs it"
            ),
        }
    });
}

/// Download, verify and unpack one release.
async fn install(release: &CdmRelease) -> Result<std::path::PathBuf, String> {
    let root = install_root().ok_or("no config directory to install a CDM into")?;
    tracing::info!(
        "am.widevine.fetch: downloading Widevine CDM {} ({} MiB)",
        release.version,
        release.size / (1024 * 1024)
    );

    let bytes = reqwest::Client::new()
        .get(&release.url)
        .send()
        .await
        .map_err(|e| format!("download CDM: {e}"))?
        .error_for_status()
        .map_err(|e| format!("download CDM: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("read CDM download: {e}"))?;

    if bytes.len() as u64 != release.size {
        return Err(format!(
            "CDM download is {} bytes, manifest says {}",
            bytes.len(),
            release.size
        ));
    }
    verify_sha512(&bytes, &release.sha512)?;
    tracing::debug!("am.widevine.fetch: sha512 verified");

    // Unpack beside the final directory, then rename: a half-written CDM that
    // looked installed would be loaded on the next run and fail opaquely.
    let staging = root.join(format!("{}.part", release.version));
    let final_dir = root.join(&release.version);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("create {}: {e}", staging.display()))?;

    let library = extract_cdm(&bytes, &staging).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&staging);
    })?;

    let _ = std::fs::remove_dir_all(&final_dir);
    std::fs::rename(&staging, &final_dir).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging);
        format!("install CDM to {}: {e}", final_dir.display())
    })?;

    let installed_path = final_dir.join(library.file_name().ok_or("extracted CDM has no name")?);
    tracing::info!(path = %installed_path.display(), "am.widevine.fetch: CDM installed");
    Ok(installed_path)
}

fn verify_sha512(bytes: &[u8], expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha512};
    let actual = hex_lower(&Sha512::digest(bytes));
    if actual != expected {
        return Err(format!(
            "CDM download failed its checksum (expected {expected}, got {actual})"
        ));
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Offset of the zip inside a CRX3: a 12-byte header, then a protobuf signature
/// block of the declared length, then a plain zip archive.
fn crx3_zip_offset(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < 16 {
        return Err("CRX is truncated".to_string());
    }
    if &bytes[..4] != b"Cr24" {
        return Err("not a CRX archive".to_string());
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != 3 {
        return Err(format!("unsupported CRX version {version}"));
    }
    let header_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let offset = 12usize
        .checked_add(header_len)
        .filter(|o| *o <= bytes.len())
        .ok_or("CRX header length runs past the end of the file")?;
    Ok(offset)
}

/// The sub-package directory for this host, per the component manifest.
///
/// The Linux package is one bundle covering every arch, so the manifest — not
/// the file name or the download URL — is what says where the library is.
fn sub_package_path(manifest_json: &str) -> Option<String> {
    let (os, arch) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", a) => ("linux", a),
        ("macos", a) => ("mac", a),
        ("windows", a) => ("win", a),
        _ => return None,
    };
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        other => other,
    };
    let manifest: serde_json::Value = serde_json::from_str(manifest_json).ok()?;
    manifest["platforms"]
        .as_array()?
        .iter()
        .find(|p| p["os"].as_str() == Some(os) && p["arch"].as_str() == Some(arch))
        .and_then(|p| p["sub_package_path"].as_str())
        .map(|s| s.trim_start_matches('/').to_string())
}

/// Unpack the CDM out of a CRX into `dest`, flattened. Returns its path.
fn extract_cdm(crx: &[u8], dest: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let offset = crx3_zip_offset(crx)?;
    let cursor = std::io::Cursor::new(&crx[offset..]);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("open CRX zip: {e}"))?;

    // Prefer the manifest's own answer; fall back to the conventional layout so a
    // manifest that stops listing platforms doesn't break the fetch outright.
    let wanted_dir = zip
        .index_for_name("manifest.json")
        .and_then(|i| {
            let mut f = zip.by_index(i).ok()?;
            let mut text = String::new();
            std::io::Read::read_to_string(&mut f, &mut text).ok()?;
            sub_package_path(&text)
        })
        .or_else(|| {
            archive_member().and_then(|m| m.rsplit_once('/').map(|(dir, _)| format!("{dir}/")))
        })
        .ok_or("no sub-package for this platform in the CRX manifest")?;

    let name = super::discover::cdm_file_name();
    let member = format!("{wanted_dir}{name}");
    let index = zip
        .index_for_name(&member)
        .ok_or_else(|| format!("{member} is not in the CRX"))?;

    let mut file = zip
        .by_index(index)
        .map_err(|e| format!("read {member} from CRX: {e}"))?;
    let out_path = dest.join(name);
    let mut out = std::fs::File::create(&out_path)
        .map_err(|e| format!("create {}: {e}", out_path.display()))?;
    std::io::copy(&mut file, &mut out).map_err(|e| format!("write {}: {e}", out_path.display()))?;
    drop(out);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real response, captured from the service.
    const MANIFEST: &str = r#"<?xml version="1.0"?>
<updates>
    <addons>
        <addon id="gmp-gmpopenh264" URL="https://ciscobinary.openh264.org/openh264-macosx64-aarch64-652bdb7719f30b52b08e506645a7322ff1b2cc6f.zip" hashFunction="sha512" hashValue="d69514fa5a04483674b9d5a9c2ab0c1736db6363f1afc83bad4e54f0c155949e34cdf9746e07f9d855b3fcad16da8b9e8d79b2707fa1ae1e9aaeaaab620d1026" size="475261" version="2.6.0"/>
        <addon id="gmp-widevinecdm" URL="https://edgedl.me.gvt1.com/edgedl/release2/chrome_component/ad7g6ajom265ggbvq6rrx4nb22ra_4.10.3050.0/oimompecagnajdejgnnjijobebaeigek_4.10.3050.0_mac_arm64_ad6r3hn3iuwofjkdi4widjwuy3na.crx3" hashFunction="sha512" hashValue="5ade9a40703c835026d26dc660cf5793a03e275229438f7ff7154116a33ce595d1ee99a79f6f0579231bdc9f51e72363d35c958f02ace911a7c47fb260402560" size="20189918" version="4.10.3050.0"/>
    </addons>
</updates>"#;

    #[test]
    fn the_cdm_entry_is_picked_out_of_the_manifest() {
        let r = parse_manifest(MANIFEST).expect("parse");
        assert_eq!(r.version, "4.10.3050.0");
        assert_eq!(r.size, 20_189_918);
        assert!(
            r.url
                .ends_with("_mac_arm64_ad6r3hn3iuwofjkdi4widjwuy3na.crx3")
        );
        assert_eq!(r.sha512.len(), 128, "sha512 is 64 bytes of hex");
        assert!(r.sha512.starts_with("5ade9a40"));
    }

    /// OpenH264 is listed first and has the same attribute shape, so taking the
    /// first addon would download the wrong thing entirely.
    #[test]
    fn the_first_addon_is_not_assumed_to_be_the_cdm() {
        let r = parse_manifest(MANIFEST).expect("parse");
        assert!(!r.url.contains("openh264"), "picked up OpenH264: {}", r.url);
    }

    #[test]
    fn a_manifest_without_the_cdm_is_an_error() {
        let xml =
            r#"<updates><addons><addon id="gmp-gmpopenh264" version="2.6.0"/></addons></updates>"#;
        assert!(parse_manifest(xml).is_err());
    }

    /// The hash is the only thing standing between the manifest and `dlopen`, so
    /// an entry that isn't sha512 must not be treated as verified.
    #[test]
    fn an_unexpected_hash_function_is_refused() {
        let xml = MANIFEST.replace("hashFunction=\"sha512\"", "hashFunction=\"md5\"");
        let err = parse_manifest(&xml).expect_err("md5 must be refused");
        assert!(err.contains("sha512"), "{err}");
    }

    #[test]
    fn empty_input_does_not_hang_or_panic() {
        assert!(parse_manifest("").is_err());
        assert!(parse_manifest("<updates>").is_err());
    }

    /// Host mapping and archive layout have to agree — a platform we can resolve
    /// a download for is one we can find the library inside.
    #[test]
    fn platform_and_archive_member_agree() {
        assert_eq!(gmp_platform().is_some(), archive_member().is_some());
        if let Some(member) = archive_member() {
            assert!(member.starts_with("_platform_specific/"));
            let expected = if cfg!(target_os = "windows") {
                "widevinecdm.dll"
            } else if cfg!(target_os = "macos") {
                "libwidevinecdm.dylib"
            } else {
                "libwidevinecdm.so"
            };
            assert!(member.ends_with(expected), "{member}");
        }
    }

    /// The whole flow against the real services, ending in a CDM the shim can
    /// actually drive: resolve, download, checksum, strip the CRX header, unzip,
    /// pick the sub-package from the manifest, then load it and sign a challenge.
    ///
    /// Ignored by default — it downloads ~20MB and installs into the config dir.
    #[tokio::test]
    #[ignore = "downloads a CDM and installs it into the config dir"]
    async fn fetch_and_open_a_real_cdm() {
        let path = ensure().await.expect("fetch a CDM");
        assert!(path.is_file(), "{} is not a file", path.display());

        let size = std::fs::metadata(&path).expect("stat").len();
        assert!(size > 5 * 1024 * 1024, "CDM is only {size} bytes");

        // A second call must reuse it rather than download again.
        let again = ensure().await.expect("reuse the installed CDM");
        assert_eq!(path, again);

        // The real proof: the shim loads it and it signs a challenge.
        let cdm = super::super::Cdm::open(&path)
            .await
            .expect("the fetched CDM should load");
        let session = cdm.begin_license().await;
        let (challenge, _cdm_session) = cdm
            .challenge(&session, &super::super::build_pssh(&[0x11u8; 16]))
            .expect("the fetched CDM should produce a license challenge");
        assert!(
            challenge.windows(9).any(|w| w == b"ChromeCDM"),
            "expected a ChromeCDM SignedMessage, got {} bytes",
            challenge.len()
        );
    }

    /// Two things at once, because both depend on process-global state and so
    /// have to be asserted in a known order: prefetch outside a runtime must not
    /// panic (source construction isn't guaranteed to be inside one, and
    /// `tokio::spawn` panics there), and it must arm only once so a failed
    /// download isn't retried on every source rebuild.
    #[test]
    fn prefetch_is_safe_without_a_runtime_and_arms_once() {
        prefetch();
        assert!(
            !should_prefetch(),
            "prefetch re-armed; a failing download would retry on every rebuild"
        );
    }

    /// Hits Mozilla's service. Ignored by default — it's a network test, and its
    /// job is to catch the endpoint or manifest shape changing under us.
    #[tokio::test]
    #[ignore = "requires network access to aus5.mozilla.org"]
    async fn resolve_against_the_live_service() {
        let release = resolve().await.expect("resolve a CDM for this platform");
        assert!(
            release.version.starts_with("4."),
            "unexpected version {}",
            release.version
        );
        assert!(release.url.ends_with(".crx3"), "{}", release.url);
        assert_eq!(release.sha512.len(), 128);
        assert!(
            release.size > 5 * 1024 * 1024,
            "a CDM is tens of MB, got {}",
            release.size
        );
        // `resolve` logs the version and size; run with RUST_LOG=info to see the
        // resolved URL rather than printing it (stdout is clippy-denied here).
        tracing::info!("am.widevine.fetch: resolved {}", release.url);
    }

    /// Real component manifest from 4.10.3050.0 — note it declares an arm64
    /// sub-package the package doesn't actually ship.
    const COMPONENT_MANIFEST: &str = r#"{
      "name": "WidevineCdm",
      "version": "4.10.3050.0",
      "platforms": [
        {"os": "linux", "arch": "x64", "sub_package_path": "_platform_specific/linux_x64/"},
        {"os": "linux", "arch": "arm64", "sub_package_path": "_platform_specific/linux_arm64/"}
      ]
    }"#;

    #[test]
    fn the_zip_starts_after_the_crx_header() {
        // "Cr24", version 3, header length 5, five header bytes, then the zip.
        let mut crx = b"Cr24".to_vec();
        crx.extend_from_slice(&3u32.to_le_bytes());
        crx.extend_from_slice(&5u32.to_le_bytes());
        crx.extend_from_slice(&[0xAA; 5]);
        crx.extend_from_slice(b"PK\x03\x04rest");
        assert_eq!(crx3_zip_offset(&crx).expect("offset"), 17);
    }

    #[test]
    fn a_non_crx_is_refused() {
        let mut zip = b"PK\x03\x04".to_vec();
        zip.extend_from_slice(&[0u8; 20]);
        assert!(crx3_zip_offset(&zip).is_err(), "a bare zip is not a CRX");
        assert!(crx3_zip_offset(b"Cr24").is_err(), "truncated");
        assert!(crx3_zip_offset(&[]).is_err(), "empty");
    }

    /// A header length past the end of the file would slice out of bounds.
    #[test]
    fn a_lying_header_length_is_refused() {
        let mut crx = b"Cr24".to_vec();
        crx.extend_from_slice(&3u32.to_le_bytes());
        crx.extend_from_slice(&u32::MAX.to_le_bytes());
        crx.extend_from_slice(&[0u8; 8]);
        assert!(crx3_zip_offset(&crx).is_err());
    }

    #[test]
    fn an_unsupported_crx_version_is_refused() {
        let mut crx = b"Cr24".to_vec();
        crx.extend_from_slice(&2u32.to_le_bytes());
        crx.extend_from_slice(&0u32.to_le_bytes());
        crx.extend_from_slice(&[0u8; 8]);
        let err = crx3_zip_offset(&crx).expect_err("CRX2 is a different format");
        assert!(err.contains("version 2"), "{err}");
    }

    /// The manifest is what says where the library is — the Linux package holds
    /// every arch, so the path can't be inferred from the download.
    #[test]
    fn the_sub_package_comes_from_the_manifest() {
        let picked = sub_package_path(COMPONENT_MANIFEST);
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert_eq!(picked.as_deref(), Some("_platform_specific/linux_x64/"));
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            assert_eq!(picked.as_deref(), Some("_platform_specific/linux_arm64/"));
        } else {
            // This manifest only lists Linux; other hosts must not match one.
            assert_eq!(picked, None, "matched a foreign platform: {picked:?}");
        }
    }

    #[test]
    fn a_manifest_without_our_platform_yields_nothing() {
        let other = r#"{"platforms":[{"os":"beos","arch":"ppc","sub_package_path":"x/"}]}"#;
        assert_eq!(sub_package_path(other), None);
        assert_eq!(sub_package_path("not json"), None);
        assert_eq!(sub_package_path("{}"), None);
    }

    #[test]
    fn the_checksum_gate_rejects_the_wrong_bytes() {
        // sha512 of "kopuz", checked against the real digest.
        let good = {
            use sha2::{Digest, Sha512};
            hex_lower(&Sha512::digest(b"kopuz"))
        };
        assert_eq!(good.len(), 128);
        assert!(verify_sha512(b"kopuz", &good).is_ok());
        assert!(
            verify_sha512(b"kopuZ", &good).is_err(),
            "one flipped bit must fail"
        );
        assert!(verify_sha512(b"kopuz", "deadbeef").is_err());
    }

    /// Uppercase hex from the manifest would never match a lowercase digest, so
    /// the parser normalises it.
    #[test]
    fn manifest_hashes_are_normalised_to_lowercase() {
        let xml = MANIFEST.replace(
            "5ade9a40703c835026d26dc660cf5793a03e275229438f7ff7154116a33ce595d1ee99a79f6f0579231bdc9f51e72363d35c958f02ace911a7c47fb260402560",
            "5ADE9A40703C835026D26DC660CF5793A03E275229438F7FF7154116A33CE595D1EE99A79F6F0579231BDC9F51E72363D35C958F02ACE911A7C47FB260402560",
        );
        let r = parse_manifest(&xml).expect("parse");
        assert!(r.sha512.starts_with("5ade9a40"), "{}", r.sha512);
    }

    #[test]
    fn the_install_root_sits_under_a_kopuz_directory() {
        let root = install_root().expect("a config dir");
        assert!(root.ends_with("widevine"), "{}", root.display());
        assert!(
            root.to_string_lossy().to_lowercase().contains("kopuz"),
            "{}",
            root.display()
        );
    }

    #[test]
    fn the_manifest_url_carries_the_platform() {
        let url = manifest_url("Darwin_aarch64-gcc3");
        assert!(url.starts_with("https://aus5.mozilla.org/update/3/GMP/"));
        assert!(url.contains("/Darwin_aarch64-gcc3/"));
        assert!(url.ends_with("update.xml"));
    }
}
