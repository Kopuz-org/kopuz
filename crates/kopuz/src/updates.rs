use std::path::{Path, PathBuf};

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/Kopuz-org/kopuz/releases/latest";

/// The version the check compares against. A build made with
/// `KOPUZ_UPDATE_PRETEND_VERSION` set reports that instead, so the updater can
/// be exercised against the published release; release CI never sets it.
const CURRENT_VERSION: &str = match option_env!("KOPUZ_UPDATE_PRETEND_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    pub release_url: String,
    /// Set only when this install can replace itself; everything else keeps
    /// the release link.
    pub install: Option<InstallPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallPlan {
    kind: InstallKind,
    asset_name: String,
    asset_url: String,
    checksum_url: String,
}

/// How this copy of Kopuz was installed, for the installs it can update.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "each platform builds only its own install kinds")
)]
enum InstallKind {
    /// The per-user NSIS installer, which leaves `uninstall.exe` beside the binary.
    Nsis {
        dir: PathBuf,
        exe: PathBuf,
    },
    /// The per-machine MSI under Program Files.
    Msi {
        dir: PathBuf,
        exe: PathBuf,
    },
    AppImage {
        path: PathBuf,
    },
    MacApp {
        bundle: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallProgress {
    Downloading {
        done: u64,
        total: Option<u64>,
    },
    /// The installer is staged and waits for this process to exit.
    Ready,
    Failed,
}

#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(serde::Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

fn parse_version_parts(version: &str) -> Option<Vec<u64>> {
    let core = version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()
        .unwrap_or_default();
    let parts: Option<Vec<u64>> = core
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect();
    parts.filter(|parts| !parts.is_empty())
}

fn is_newer_version(current: &str, candidate: &str) -> bool {
    let Some(current_parts) = parse_version_parts(current) else {
        return false;
    };
    let Some(candidate_parts) = parse_version_parts(candidate) else {
        return false;
    };

    let max_len = current_parts.len().max(candidate_parts.len());
    for idx in 0..max_len {
        let current_part = *current_parts.get(idx).unwrap_or(&0);
        let candidate_part = *candidate_parts.get(idx).unwrap_or(&0);
        match candidate_part.cmp(&current_part) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => {}
        }
    }

    false
}

fn http_client(timeout: std::time::Duration) -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("kopuz/{}", env!("CARGO_PKG_VERSION")))
        .timeout(timeout)
        .build()
        .ok()
}

pub async fn fetch_available() -> Option<AvailableUpdate> {
    let client = http_client(std::time::Duration::from_secs(8))?;
    let release = client
        .get(LATEST_RELEASE_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<GithubRelease>()
        .await
        .ok()?;

    if !is_newer_version(CURRENT_VERSION, &release.tag_name) {
        return None;
    }
    let install = detect_install()
        .and_then(|kind| select_asset(kind, std::env::consts::ARCH, &release.assets));
    Some(AvailableUpdate {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        release_url: release.html_url,
        install,
    })
}

/// Picks the release asset that matches this install and its published
/// checksum. No checksum, no self-update.
fn select_asset(kind: InstallKind, arch: &str, assets: &[GithubAsset]) -> Option<InstallPlan> {
    let windows_arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let suffix = match &kind {
        InstallKind::Nsis { .. } => format!("_{windows_arch}-setup.exe"),
        InstallKind::Msi { .. } => format!("_{windows_arch}.msi"),
        InstallKind::AppImage { .. } => format!("_{arch}.AppImage"),
        InstallKind::MacApp { .. } => format!("_{arch}.dmg"),
    };
    let asset = assets.iter().find(|asset| asset.name.ends_with(&suffix))?;
    let checksum_name = format!("{}.sha256", asset.name);
    let checksum = assets.iter().find(|asset| asset.name == checksum_name)?;
    Some(InstallPlan {
        kind,
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
    })
}

/// Reads a `sha256sum`-style line (`<hex>  <name>`) and compares it with the
/// digest of what was downloaded.
fn checksum_matches(digest: &[u8], checksum_file: &str) -> bool {
    let Some(expected) = checksum_file
        .trim_start_matches('\u{feff}')
        .split_whitespace()
        .next()
    else {
        return false;
    };
    let mut expected_bytes = [0u8; 32];
    hex::decode_to_slice(expected, &mut expected_bytes).is_ok() && expected_bytes == digest
}

/// Package managers, debug builds and anything this process cannot write
/// over get the release link instead.
fn detect_install() -> Option<InstallKind> {
    if cfg!(debug_assertions) || cfg!(target_os = "android") {
        return None;
    }
    if std::env::var_os("FLATPAK_ID").is_some() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    if exe.starts_with("/nix/store") {
        return None;
    }
    detect_platform_install(&exe)
}

#[cfg(target_os = "windows")]
fn detect_platform_install(exe: &Path) -> Option<InstallKind> {
    let dir = exe.parent()?;
    let program_files: Vec<PathBuf> = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    classify_windows_install(
        dir,
        exe,
        dir.join("uninstall.exe").is_file(),
        &program_files,
    )
}

#[cfg(any(target_os = "windows", test))]
fn classify_windows_install(
    dir: &Path,
    exe: &Path,
    has_uninstaller: bool,
    program_files: &[PathBuf],
) -> Option<InstallKind> {
    // Windows paths compare case-insensitively.
    let dir_lower = dir.to_string_lossy().to_lowercase();
    let under_program_files = program_files.iter().any(|root| {
        let root = root.to_string_lossy().to_lowercase();
        dir_lower
            .strip_prefix(root.trim_end_matches(['\\', '/']))
            .is_some_and(|rest| rest.starts_with(['\\', '/']))
    });
    let (dir, exe) = (dir.to_path_buf(), exe.to_path_buf());
    match (has_uninstaller, under_program_files) {
        // The NSIS installer runs unelevated, so it cannot update a copy
        // someone placed under Program Files.
        (true, false) => Some(InstallKind::Nsis { dir, exe }),
        (false, true) => Some(InstallKind::Msi { dir, exe }),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn detect_platform_install(_exe: &Path) -> Option<InstallKind> {
    let path = PathBuf::from(std::env::var_os("APPIMAGE")?);
    (path.is_file() && dir_is_writable(path.parent()?)).then_some(InstallKind::AppImage { path })
}

#[cfg(target_os = "macos")]
fn detect_platform_install(exe: &Path) -> Option<InstallKind> {
    let bundle = mac_bundle_of(exe)?;
    dir_is_writable(bundle.parent()?).then_some(InstallKind::MacApp { bundle })
}

#[cfg(any(target_os = "macos", test))]
fn mac_bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    let contents = macos_dir.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos_dir.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn detect_platform_install(_exe: &Path) -> Option<InstallKind> {
    None
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".kopuz-update-probe-{}", std::process::id()));
    let writable = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    writable
}

/// Downloads and verifies the update on its own thread, then starts the
/// helper that applies it once this process exits. The caller quits on
/// `Ready`.
pub fn install(plan: InstallPlan) -> tokio::sync::watch::Receiver<InstallProgress> {
    let (tx, rx) = tokio::sync::watch::channel(InstallProgress::Downloading {
        done: 0,
        total: None,
    });
    let spawned = std::thread::Builder::new()
        .name("kopuz-update".into())
        .spawn(move || {
            let outcome = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("update runtime: {error}"))
                .and_then(|runtime| runtime.block_on(download_and_stage(&plan, &tx)));
            let _ = tx.send(match outcome {
                Ok(()) => InstallProgress::Ready,
                Err(error) => {
                    tracing::error!(%error, asset = %plan.asset_name, "update failed");
                    InstallProgress::Failed
                }
            });
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "could not start the update thread");
        let (failed_tx, failed_rx) = tokio::sync::watch::channel(InstallProgress::Failed);
        drop(failed_tx);
        return failed_rx;
    }
    rx
}

async fn download_and_stage(
    plan: &InstallPlan,
    progress: &tokio::sync::watch::Sender<InstallProgress>,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::Write;

    let client = http_client(std::time::Duration::from_secs(30 * 60))
        .ok_or("could not build the HTTP client")?;
    let checksum = client
        .get(&plan.checksum_url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("checksum download: {error}"))?
        .text()
        .await
        .map_err(|error| format!("checksum download: {error}"))?;

    let target = download_path(plan)?;
    let mut response = client
        .get(&plan.asset_url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("download: {error}"))?;
    let total = response.content_length();
    let mut file =
        std::fs::File::create(&target).map_err(|error| format!("{}: {error}", target.display()))?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("download: {error}"))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .map_err(|error| format!("{}: {error}", target.display()))?;
        done += chunk.len() as u64;
        let _ = progress.send(InstallProgress::Downloading { done, total });
    }
    file.sync_all()
        .map_err(|error| format!("{}: {error}", target.display()))?;
    drop(file);

    if !checksum_matches(&hasher.finalize(), &checksum) {
        let _ = std::fs::remove_file(&target);
        return Err(format!("{} does not match its checksum", plan.asset_name));
    }
    stage(&plan.kind, &target)
}

fn download_path(plan: &InstallPlan) -> Result<PathBuf, String> {
    // An AppImage is renamed over the running one, which needs the same filesystem.
    let dir = match &plan.kind {
        InstallKind::AppImage { path } => path
            .parent()
            .map(Path::to_path_buf)
            .ok_or("AppImage has no parent directory")?,
        _ => std::env::temp_dir().join("kopuz-update"),
    };
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    Ok(dir.join(format!(".{}.part", plan.asset_name)))
}

#[cfg(target_os = "windows")]
fn stage(kind: &InstallKind, download: &Path) -> Result<(), String> {
    use base64::Engine as _;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Windows decides how to open a file by its extension.
    let installer = download.with_file_name(
        download
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.trim_start_matches('.').trim_end_matches(".part"))
            .ok_or("installer has no file name")?,
    );
    std::fs::rename(download, &installer)
        .map_err(|error| format!("{}: {error}", installer.display()))?;
    let script = windows_apply_script(kind, &installer, std::process::id())
        .ok_or("not a Windows install")?;
    let encoded: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
            &base64::engine::general_purpose::STANDARD.encode(encoded),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(drop)
        .map_err(|error| format!("could not start the installer helper: {error}"))
}

/// The PowerShell the helper runs: wait for Kopuz to exit so the installer can
/// replace its files, install, then start Kopuz again. A failed or cancelled
/// install still relaunches the old copy.
#[cfg(any(target_os = "windows", test))]
fn windows_apply_script(kind: &InstallKind, installer: &Path, pid: u32) -> Option<String> {
    let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let (dir, exe) = match kind {
        InstallKind::Nsis { dir, exe } | InstallKind::Msi { dir, exe } => (dir, exe),
        _ => return None,
    };
    let dir = dir.display().to_string();
    let installer = installer.display().to_string();
    // Not `-Wait`: on Windows PowerShell that also waits for whatever the
    // installer leaves running, such as the WebView2 updater it starts.
    let run = match kind {
        // NSIS takes `/D=` unquoted and only as the last argument.
        InstallKind::Nsis { .. } => format!(
            "(Start-Process -FilePath {} -ArgumentList {} -PassThru).WaitForExit()",
            quote(&installer),
            quote(&format!("/S /D={dir}")),
        ),
        // Basic UI, so a per-machine install can still raise the UAC prompt.
        _ => format!(
            "(Start-Process -FilePath 'msiexec.exe' -ArgumentList {} -PassThru).WaitForExit()",
            quote(&format!(
                "/i \"{installer}\" /passive /norestart INSTALLDIR=\"{dir}\""
            )),
        ),
    };
    Some(format!(
        "Wait-Process -Id {pid} -Timeout 60 -ErrorAction SilentlyContinue\n\
         {run}\n\
         Remove-Item -LiteralPath {} -ErrorAction SilentlyContinue\n\
         Start-Process -FilePath {}\n",
        quote(&installer),
        quote(&exe.display().to_string()),
    ))
}

#[cfg(target_os = "linux")]
fn stage(kind: &InstallKind, download: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let InstallKind::AppImage { path } = kind else {
        return Err("not an AppImage install".into());
    };
    std::fs::set_permissions(download, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("{}: {error}", download.display()))?;
    // The running image stays mounted from the old inode, so the swap is safe now.
    std::fs::rename(download, path).map_err(|error| format!("{}: {error}", path.display()))?;
    spawn_after_exit(
        r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$2""#,
        &[path.as_os_str()],
    )
}

#[cfg(target_os = "macos")]
fn stage(kind: &InstallKind, download: &Path) -> Result<(), String> {
    let InstallKind::MacApp { bundle } = kind else {
        return Err("not an app bundle install".into());
    };
    let parent = bundle.parent().ok_or("app bundle has no parent")?;
    let staged = parent.join(".kopuz-update.app");
    let backup = parent.join(".kopuz-previous.app");
    let mount = download.with_extension("mount");
    let _ = std::fs::remove_dir_all(&staged);
    let _ = std::fs::create_dir_all(&mount);

    run("hdiutil", |cmd| {
        cmd.args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(download)
    })?;
    let copied = std::fs::read_dir(&mount)
        .map_err(|error| format!("{}: {error}", mount.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or_else(|| "the disk image holds no app".to_string())
        .and_then(|app| run("ditto", |cmd| cmd.arg(&app).arg(&staged)));
    let _ = run("hdiutil", |cmd| cmd.args(["detach", "-quiet"]).arg(&mount));
    let _ = std::fs::remove_file(download);
    copied?;

    spawn_after_exit(
        r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
rm -rf "$4"
if mv "$2" "$4"; then
  if mv "$3" "$2"; then rm -rf "$4"; else mv "$4" "$2"; fi
fi
exec open "$2""#,
        &[bundle.as_os_str(), staged.as_os_str(), backup.as_os_str()],
    )
}

#[cfg(target_os = "macos")]
fn run(
    program: &str,
    args: impl FnOnce(&mut std::process::Command) -> &mut std::process::Command,
) -> Result<(), String> {
    let mut command = std::process::Command::new(program);
    let status = args(&mut command)
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|error| format!("{program}: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{program} exited with {status}"))
}

/// Runs `script` under `sh` with this process's pid as `$1` and `args` after
/// it, detached from this process's stdio.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_after_exit(script: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(std::process::id().to_string())
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|error| format!("could not start the update helper: {error}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn stage(_kind: &InstallKind, _download: &Path) -> Result<(), String> {
    Err("self-update is not supported here".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn asset(name: &str) -> GithubAsset {
        GithubAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.invalid/{name}"),
        }
    }

    fn release_assets() -> Vec<GithubAsset> {
        [
            "kopuz-0.19.0-1.x86_64.rpm",
            "kopuz-0.19.0-1.x86_64.rpm.sha256",
            "Kopuz_0.19.0_aarch64.dmg",
            "Kopuz_0.19.0_aarch64.dmg.sha256",
            "Kopuz_0.19.0_x64-setup.exe",
            "Kopuz_0.19.0_x64-setup.exe.sha256",
            "Kopuz_0.19.0_x64.msi",
            "Kopuz_0.19.0_x64.msi.sha256",
            "Kopuz_0.19.0_x86_64.AppImage",
            "Kopuz_0.19.0_x86_64.AppImage.sha256",
        ]
        .into_iter()
        .map(asset)
        .collect()
    }

    fn nsis() -> InstallKind {
        InstallKind::Nsis {
            dir: PathBuf::from(r"C:\Users\kyan\AppData\Local\Programs\Kopuz"),
            exe: PathBuf::from(r"C:\Users\kyan\AppData\Local\Programs\Kopuz\kopuz.exe"),
        }
    }

    fn msi() -> InstallKind {
        InstallKind::Msi {
            dir: PathBuf::from(r"C:\Program Files\Kopuz"),
            exe: PathBuf::from(r"C:\Program Files\Kopuz\kopuz.exe"),
        }
    }

    #[test]
    fn newer_versions_are_detected() {
        assert!(is_newer_version("0.18.0", "v0.19.0"));
        assert!(is_newer_version("0.19.0", "0.19.1"));
        assert!(is_newer_version("0.19", "0.19.1"));
        assert!(!is_newer_version("0.19.0", "v0.19.0"));
        assert!(!is_newer_version("0.19.1", "0.19.0"));
        assert!(!is_newer_version("0.19.0", "0.19.0-rc1"));
        assert!(!is_newer_version("0.19.0", "nightly"));
    }

    #[test]
    fn picks_the_asset_for_each_install_type() {
        let assets = release_assets();
        let pick = |kind, arch| select_asset(kind, arch, &assets).map(|plan| plan.asset_name);
        assert_eq!(
            pick(nsis(), "x86_64").as_deref(),
            Some("Kopuz_0.19.0_x64-setup.exe")
        );
        assert_eq!(
            pick(msi(), "x86_64").as_deref(),
            Some("Kopuz_0.19.0_x64.msi")
        );
        let appimage = InstallKind::AppImage {
            path: PathBuf::from("/home/k/Kopuz.AppImage"),
        };
        assert_eq!(
            pick(appimage, "x86_64").as_deref(),
            Some("Kopuz_0.19.0_x86_64.AppImage")
        );
        let mac = InstallKind::MacApp {
            bundle: PathBuf::from("/Applications/Kopuz.app"),
        };
        assert_eq!(
            pick(mac.clone(), "aarch64").as_deref(),
            Some("Kopuz_0.19.0_aarch64.dmg")
        );
        assert_eq!(pick(mac, "x86_64"), None);
        assert_eq!(pick(nsis(), "aarch64"), None);
    }

    #[test]
    fn plan_carries_the_matching_checksum() {
        let plan = select_asset(msi(), "x86_64", &release_assets()).expect("msi plan");
        assert_eq!(
            plan.checksum_url,
            "https://example.invalid/Kopuz_0.19.0_x64.msi.sha256"
        );
    }

    #[test]
    fn an_asset_without_a_checksum_is_not_installed() {
        let assets: Vec<_> = release_assets()
            .into_iter()
            .filter(|asset| asset.name != "Kopuz_0.19.0_x64-setup.exe.sha256")
            .collect();
        assert_eq!(select_asset(nsis(), "x86_64", &assets), None);
    }

    #[test]
    fn checksum_accepts_the_published_digest() {
        let digest = Sha256::digest(b"kopuz installer");
        let published = format!("{}  Kopuz_0.19.0_x64.msi\r\n", hex::encode(digest));
        assert!(checksum_matches(&digest, &published));
        assert!(checksum_matches(
            &digest,
            &format!("\u{feff}{}", hex::encode(digest).to_uppercase())
        ));
    }

    #[test]
    fn checksum_rejects_a_different_file() {
        let published = format!(
            "{}  Kopuz_0.19.0_x64.msi",
            hex::encode(Sha256::digest(b"kopuz installer"))
        );
        assert!(!checksum_matches(
            &Sha256::digest(b"tampered installer"),
            &published
        ));
        let digest = Sha256::digest(b"kopuz installer");
        assert!(!checksum_matches(&digest, ""));
        assert!(!checksum_matches(&digest, "not-hex  file"));
        assert!(!checksum_matches(&digest, &hex::encode(&digest[..16])));
    }

    #[test]
    fn windows_install_type_follows_the_install_layout() {
        let program_files = [
            PathBuf::from(r"C:\Program Files"),
            PathBuf::from(r"C:\Program Files (x86)"),
        ];
        let user = Path::new(r"C:\Users\kyan\AppData\Local\Programs\Kopuz");
        let machine = Path::new(r"C:\PROGRAM FILES\Kopuz");
        let lookalike = Path::new(r"C:\Program Files Extra\Kopuz");
        let portable = Path::new(r"D:\Apps\Kopuz");
        let classify = |dir: &Path, uninstaller| {
            classify_windows_install(dir, &dir.join("kopuz.exe"), uninstaller, &program_files)
        };

        assert!(matches!(
            classify(user, true),
            Some(InstallKind::Nsis { .. })
        ));
        assert!(matches!(
            classify(machine, false),
            Some(InstallKind::Msi { .. })
        ));
        assert_eq!(classify(machine, true), None);
        assert_eq!(classify(lookalike, false), None);
        assert_eq!(classify(portable, false), None);
        assert_eq!(classify(user, false), None);
    }

    #[test]
    fn mac_bundle_is_found_only_inside_an_app() {
        assert_eq!(
            mac_bundle_of(Path::new("/Applications/Kopuz.app/Contents/MacOS/kopuz")),
            Some(PathBuf::from("/Applications/Kopuz.app"))
        );
        assert_eq!(mac_bundle_of(Path::new("/usr/local/bin/kopuz")), None);
        assert_eq!(
            mac_bundle_of(Path::new("/opt/kopuz/target/release/kopuz")),
            None
        );
    }

    #[test]
    fn windows_script_runs_each_installer_silently() {
        let installer = Path::new(r"C:\Temp\kopuz-update\Kopuz_0.19.0_x64-setup.exe");
        let nsis_script = windows_apply_script(&nsis(), installer, 42).expect("nsis script");
        assert!(nsis_script.contains("Wait-Process -Id 42"));
        assert!(nsis_script.contains(r"'/S /D=C:\Users\kyan\AppData\Local\Programs\Kopuz'"));
        assert!(nsis_script.contains(
            r"Start-Process -FilePath 'C:\Users\kyan\AppData\Local\Programs\Kopuz\kopuz.exe'"
        ));

        let msi_script =
            windows_apply_script(&msi(), Path::new(r"C:\Temp\it's.msi"), 7).expect("msi script");
        assert!(msi_script.contains("msiexec.exe"));
        assert!(msi_script.contains(
            r#"'/i "C:\Temp\it''s.msi" /passive /norestart INSTALLDIR="C:\Program Files\Kopuz"'"#
        ));
        let appimage = InstallKind::AppImage {
            path: PathBuf::from("/tmp/Kopuz.AppImage"),
        };
        assert_eq!(windows_apply_script(&appimage, installer, 1), None);
    }
}
