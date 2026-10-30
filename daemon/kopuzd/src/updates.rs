//! Release checks and self-update for the packaged Kopuz app, which ships this daemon beside it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::{ApiError, ApiEvent, AvailableUpdate, UpdateStatus};
use config::flavor::Flavor;
use daemon::SessionHandle;

const RELEASES_URL: &str = "https://api.github.com/repos/Kopuz-org/kopuz/releases";
/// Nightlies come out daily and only the last ten are kept, so one page always holds the newest.
const NIGHTLY_PAGE: &str = "?per_page=30";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
// A daemon that starts before the network would otherwise offer nothing for a whole CHECK_EVERY.
const RETRY_EVERY: Duration = Duration::from_secs(10 * 60);
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
// Long enough for the Ready event to reach the app before the socket closes.
const EXIT_GRACE: Duration = Duration::from_secs(1);

/// `KOPUZ_UPDATE_PRETEND_VERSION` exercises the updater against the published release; release CI never sets it.
const CURRENT_VERSION: &str = match option_env!("KOPUZ_UPDATE_PRETEND_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The installers of this daemon's own app; Kopuz Legacy's `Kopuz-Legacy-…` and the other flavour's never match.
const fn asset_prefix(flavor: Flavor) -> &'static str {
    match flavor {
        Flavor::Stable => "Kopuz_",
        Flavor::Nightly => "Kopuz-Nightly_",
    }
}

pub struct Updater {
    session: SessionHandle,
    install: Option<InstallKind>,
    state: Mutex<State>,
    exit: tokio::sync::Notify,
}

#[derive(Default)]
struct State {
    status: UpdateStatus,
    plan: Option<InstallPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InstallPlan {
    kind: InstallKind,
    asset_name: String,
    asset_url: String,
    checksum_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "each platform builds only its own install kinds")
)]
enum InstallKind {
    /// The per-user NSIS installer, which leaves `uninstall.exe` beside the binaries.
    Nsis {
        dir: PathBuf,
    },
    Msi {
        dir: PathBuf,
    },
    AppImage {
        path: PathBuf,
    },
    MacApp {
        bundle: PathBuf,
    },
}

#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(serde::Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

impl Updater {
    pub fn new(session: SessionHandle) -> Arc<Self> {
        Arc::new(Self {
            session,
            install: detect_install(),
            state: Mutex::default(),
            exit: tokio::sync::Notify::new(),
        })
    }

    pub fn status(&self) -> UpdateStatus {
        self.lock().status.clone()
    }

    /// Check now and then every few hours while `auto_check_updates` is on.
    pub fn spawn_checks(self: &Arc<Self>, config: Arc<daemon::ConfigService>) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let mut wait = CHECK_EVERY;
                if config.snapshot().await.system.auto_check_updates
                    && let Err(error) = this.check().await
                {
                    tracing::debug!(%error, "update check failed");
                    wait = RETRY_EVERY;
                }
                tokio::time::sleep(wait).await;
            }
        });
    }

    pub async fn check(&self) -> Result<UpdateStatus, ApiError> {
        let flavor = Flavor::current();
        let (status, plan) = match fetch_newest(flavor).await? {
            Some(release) if is_newer_version(CURRENT_VERSION, &release.tag_name) => {
                let plan = self.install.clone().and_then(|kind| {
                    select_asset(flavor, kind, std::env::consts::ARCH, &release.assets)
                });
                let available = AvailableUpdate {
                    version: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
                    release_url: release.html_url,
                    installable: plan.is_some(),
                };
                (UpdateStatus::Available(available), plan)
            }
            _ => (UpdateStatus::Current, None),
        };
        let mut state = self.lock();
        // A download or a staged update outranks whatever a check finds.
        if matches!(
            state.status,
            UpdateStatus::Downloading { .. } | UpdateStatus::Ready { .. }
        ) {
            return Ok(state.status.clone());
        }
        state.plan = plan;
        Ok(self.publish(&mut state, status))
    }

    /// Start the download; progress and the outcome arrive as `Update` events.
    pub fn install(self: &Arc<Self>, app_pid: Option<u32>) -> Result<(), ApiError> {
        let (update, plan) = {
            let mut state = self.lock();
            let update = match &state.status {
                UpdateStatus::Available(update) | UpdateStatus::Failed(update) => update.clone(),
                UpdateStatus::Downloading { .. } | UpdateStatus::Ready { .. } => return Ok(()),
                UpdateStatus::Current => {
                    return Err(ApiError::new(
                        api::ErrorCode::Conflict,
                        "there is no update to install",
                    ));
                }
            };
            let plan = state
                .plan
                .clone()
                .ok_or_else(|| ApiError::unsupported("installing updates for this install"))?;
            let downloading = UpdateStatus::Downloading {
                version: update.version.clone(),
                done: 0,
                total: None,
            };
            self.publish(&mut state, downloading);
            (update, plan)
        };
        let this = self.clone();
        tokio::spawn(async move {
            let outcome = this
                .download_and_stage(&update.version, &plan, app_pid)
                .await;
            let staged = outcome.is_ok();
            let status = match outcome {
                Ok(()) => UpdateStatus::Ready {
                    version: update.version,
                },
                Err(error) => {
                    tracing::error!(%error, asset = %plan.asset_name, "update failed");
                    UpdateStatus::Failed(update)
                }
            };
            this.publish(&mut this.lock(), status);
            if staged {
                tokio::time::sleep(EXIT_GRACE).await;
                this.exit.notify_one();
            }
        });
        Ok(())
    }

    /// Resolves once a staged update needs this daemon gone.
    pub async fn exit_requested(&self) {
        self.exit.notified().await;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn publish(&self, state: &mut State, status: UpdateStatus) -> UpdateStatus {
        if state.status != status {
            state.status = status.clone();
            self.session.emit_event(ApiEvent::Update(status.clone()));
        }
        status
    }

    async fn download_and_stage(
        &self,
        version: &str,
        plan: &InstallPlan,
        app_pid: Option<u32>,
    ) -> Result<(), String> {
        let client = http_client(Duration::from_secs(30 * 60))?;
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
        let pids: Vec<u32> = std::iter::once(std::process::id()).chain(app_pid).collect();
        let kind = plan.kind.clone();
        let staging = target.clone();
        let staged = match self.fetch(&client, version, plan, &target, &checksum).await {
            Ok(()) => tokio::task::spawn_blocking(move || stage(&kind, &staging, &pids))
                .await
                .map_err(|error| format!("staging: {error}"))
                .flatten(),
            Err(error) => Err(error),
        };
        // A retry starts a fresh download, so a half-written or unstaged one is only clutter.
        if staged.is_err() {
            let _ = std::fs::remove_file(&target);
        }
        staged
    }

    /// Stream the asset into `target`, reporting progress, and check it against `checksum`.
    async fn fetch(
        &self,
        client: &reqwest::Client,
        version: &str,
        plan: &InstallPlan,
        target: &Path,
        checksum: &str,
    ) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        use std::io::Write;

        let mut response = client
            .get(&plan.asset_url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| format!("download: {error}"))?;
        let total = response.content_length();
        let mut file = std::fs::File::create(target)
            .map_err(|error| format!("{}: {error}", target.display()))?;
        let mut hasher = Sha256::new();
        let mut done = 0u64;
        let mut told = Instant::now();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("download: {error}"))?
        {
            hasher.update(&chunk);
            file.write_all(&chunk)
                .map_err(|error| format!("{}: {error}", target.display()))?;
            done += chunk.len() as u64;
            if told.elapsed() >= PROGRESS_EVERY {
                told = Instant::now();
                let progress = UpdateStatus::Downloading {
                    version: version.to_string(),
                    done,
                    total,
                };
                self.publish(&mut self.lock(), progress);
            }
        }
        file.sync_all()
            .map_err(|error| format!("{}: {error}", target.display()))?;
        drop(file);

        match checksum_matches(&hasher.finalize(), checksum) {
            true => Ok(()),
            false => Err(format!("{} does not match its checksum", plan.asset_name)),
        }
    }
}

fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    utils::http::builder()
        .user_agent(format!("kopuz/{}", env!("CARGO_PKG_VERSION")))
        .timeout(timeout)
        .build()
        .map_err(|error| format!("could not build the HTTP client: {error}"))
}

/// The newest release this flavour updates to: GitHub's latest for Kopuz, the newest nightly pre-release for Kopuz Nightly.
async fn fetch_newest(flavor: Flavor) -> Result<Option<GithubRelease>, ApiError> {
    let unreachable = |error: reqwest::Error| {
        ApiError::new(
            api::ErrorCode::SourceUnreachable,
            format!("release check: {error}"),
        )
    };
    let url = match flavor {
        Flavor::Stable => format!("{RELEASES_URL}/latest"),
        Flavor::Nightly => format!("{RELEASES_URL}{NIGHTLY_PAGE}"),
    };
    let response = http_client(Duration::from_secs(8))
        .map_err(ApiError::internal)?
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(unreachable)?;
    match flavor {
        Flavor::Stable => response.json().await.map(Some).map_err(unreachable),
        Flavor::Nightly => response
            .json::<Vec<GithubRelease>>()
            .await
            .map(newest_nightly)
            .map_err(unreachable),
    }
}

fn newest_nightly(releases: Vec<GithubRelease>) -> Option<GithubRelease> {
    releases
        .into_iter()
        .filter(|release| release.prerelease && release.tag_name.contains("-nightly."))
        .filter_map(|release| Some((version_of(&release.tag_name)?, release)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, release)| release)
}

fn version_of(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches(['v', 'V'])).ok()
}

/// Semver order, so `0.68.0-nightly.20261029` follows `…20261028` and `0.68.0` follows both.
fn is_newer_version(current: &str, candidate: &str) -> bool {
    matches!(
        (version_of(current), version_of(candidate)),
        (Some(current), Some(candidate)) if candidate > current
    )
}

/// No published checksum, no self-update.
fn select_asset(
    flavor: Flavor,
    kind: InstallKind,
    arch: &str,
    assets: &[GithubAsset],
) -> Option<InstallPlan> {
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
    let prefix = asset_prefix(flavor);
    let asset = assets
        .iter()
        .find(|asset| asset.name.starts_with(prefix) && asset.name.ends_with(&suffix))?;
    let checksum_name = format!("{}.sha256", asset.name);
    let checksum = assets.iter().find(|asset| asset.name == checksum_name)?;
    Some(InstallPlan {
        kind,
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
    })
}

/// `checksum_file` is a `sha256sum`-style line (`<hex>  <name>`).
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

/// Only an installer we ship, in a place this user can write: anything a package manager owns gets `None`.
fn detect_install() -> Option<InstallKind> {
    if cfg!(debug_assertions) {
        return None;
    }
    detect_platform_install(&std::env::current_exe().ok()?)
}

#[cfg(target_os = "windows")]
fn detect_platform_install(exe: &Path) -> Option<InstallKind> {
    let dir = exe.parent()?;
    let program_files: Vec<PathBuf> = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    classify_windows_install(dir, dir.join("uninstall.exe").is_file(), &program_files)
}

#[cfg(any(target_os = "windows", test))]
fn classify_windows_install(
    dir: &Path,
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
    let dir = dir.to_path_buf();
    match (has_uninstaller, under_program_files) {
        // The NSIS installer runs unelevated, so it cannot update a copy someone placed under Program Files.
        (true, false) => Some(InstallKind::Nsis { dir }),
        (false, true) => Some(InstallKind::Msi { dir }),
        _ => None,
    }
}

/// The app sets `APPIMAGE` and the daemon it starts inherits it; one started any other way is no AppImage install.
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
fn stage(kind: &InstallKind, download: &Path, pids: &[u32]) -> Result<(), String> {
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
    let Some(script) = windows_apply_script(Flavor::current(), kind, &installer, pids) else {
        let _ = std::fs::remove_file(&installer);
        return Err("not a Windows install".into());
    };
    let encoded: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let started = std::process::Command::new("powershell.exe")
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
        .map_err(|error| format!("could not start the installer helper: {error}"));
    if started.is_err() {
        let _ = std::fs::remove_file(&installer);
    }
    started
}

/// A failed or cancelled install still starts the old copy again.
#[cfg(any(target_os = "windows", test))]
fn windows_apply_script(
    flavor: Flavor,
    kind: &InstallKind,
    installer: &Path,
    pids: &[u32],
) -> Option<String> {
    let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let (InstallKind::Nsis { dir } | InstallKind::Msi { dir }) = kind else {
        return None;
    };
    let app = dir
        .join(format!("{}.exe", flavor.binary("kopuz")))
        .display()
        .to_string();
    let dir = dir.display().to_string();
    let installer = installer.display().to_string();
    let pids = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    // Not `-Wait`: on Windows PowerShell that also waits for whatever the installer leaves running.
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
        "Wait-Process -Id {pids} -Timeout 60 -ErrorAction SilentlyContinue\n\
         {run}\n\
         Remove-Item -LiteralPath {} -ErrorAction SilentlyContinue\n\
         Start-Process -FilePath {}\n",
        quote(&installer),
        quote(&app),
    ))
}

#[cfg(target_os = "linux")]
fn stage(kind: &InstallKind, download: &Path, pids: &[u32]) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let InstallKind::AppImage { path } = kind else {
        return Err("not an AppImage install".into());
    };
    std::fs::set_permissions(download, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("{}: {error}", download.display()))?;
    // The running image stays mounted from the old inode, so the swap is safe now.
    std::fs::rename(download, path).map_err(|error| format!("{}: {error}", path.display()))?;
    spawn_after_exit(pids, r#"exec "$1""#, &[path.as_os_str()])
}

#[cfg(target_os = "macos")]
fn stage(kind: &InstallKind, download: &Path, pids: &[u32]) -> Result<(), String> {
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
        pids,
        r#"rm -rf "$3"
if mv "$1" "$3"; then
  if mv "$2" "$1"; then rm -rf "$3"; else mv "$3" "$1"; fi
fi
exec open "$1""#,
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

/// Runs `script` under `sh` once every pid in `pids` has exited, with `args` as `$1…`.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_after_exit(pids: &[u32], script: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let pids = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    // A zombie still answers `kill -0`, and an app's launcher may be slow to reap it.
    let wait = format!(
        "for pid in {pids}; do\n  while :; do case \"$(ps -o stat= -p \"$pid\" 2>/dev/null)\" in ''|Z*) break ;; esac; sleep 0.2; done\ndone\n{script}"
    );
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(wait)
        .arg("sh")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Its own group, so the signal that stops this daemon leaves the helper running.
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    command
        .spawn()
        .map(drop)
        .map_err(|error| format!("could not start the update helper: {error}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn stage(_kind: &InstallKind, _download: &Path, _pids: &[u32]) -> Result<(), String> {
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
            "kopuz_1.0.0-1_amd64.deb",
            "kopuz_1.0.0-1_amd64.deb.sha256",
            "Kopuz_1.0.0_aarch64.dmg",
            "Kopuz_1.0.0_aarch64.dmg.sha256",
            "Kopuz_1.0.0_x64-setup.exe",
            "Kopuz_1.0.0_x64-setup.exe.sha256",
            "Kopuz_1.0.0_x64.msi",
            "Kopuz_1.0.0_x64.msi.sha256",
            "Kopuz_1.0.0_x86_64.AppImage",
            "Kopuz_1.0.0_x86_64.AppImage.sha256",
            "Kopuz-Legacy-1.0.0-aarch64.dmg",
            "Kopuz-Legacy-1.0.0-aarch64.dmg.sha256",
            "Kopuz-Legacy-1.0.0-x64-setup.exe",
            "Kopuz-Legacy-1.0.0-x64-setup.exe.sha256",
            "Kopuz-Legacy-1.0.0-x64.msi",
            "Kopuz-Legacy-1.0.0-x64.msi.sha256",
            "Kopuz-Legacy-1.0.0-x86_64.AppImage",
            "Kopuz-Legacy-1.0.0-x86_64.AppImage.sha256",
        ]
        .into_iter()
        .map(asset)
        .collect()
    }

    fn nsis() -> InstallKind {
        InstallKind::Nsis {
            dir: PathBuf::from(r"C:\Users\kyan\AppData\Local\Kopuz"),
        }
    }

    fn msi() -> InstallKind {
        InstallKind::Msi {
            dir: PathBuf::from(r"C:\Program Files\Kopuz"),
        }
    }

    #[test]
    fn newer_versions_are_detected() {
        assert!(is_newer_version("0.67.0", "v1.0.0"));
        assert!(is_newer_version("1.0.0", "1.0.1"));
        assert!(!is_newer_version("1.0.0", "v1.0.0"));
        assert!(!is_newer_version("1.0.1", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "nightly"));
    }

    #[test]
    fn nightlies_order_by_date_and_fall_behind_their_release() {
        assert!(is_newer_version("0.67.0", "0.68.0-nightly.20261028"));
        assert!(is_newer_version(
            "0.68.0-nightly.20261028",
            "v0.68.0-nightly.20261029"
        ));
        assert!(is_newer_version(
            "0.68.0-nightly.20261028",
            "0.68.0-nightly.20261028.2"
        ));
        assert!(is_newer_version("0.68.0-nightly.20261028", "0.68.0"));
        assert!(!is_newer_version("0.68.0", "0.68.0-nightly.20261030"));
    }

    fn release(tag: &str, prerelease: bool) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_string(),
            html_url: String::new(),
            prerelease,
            assets: Vec::new(),
        }
    }

    #[test]
    fn the_newest_nightly_is_picked_whatever_order_github_lists_them_in() {
        let newest = newest_nightly(vec![
            release("v0.68.0-nightly.20261028", true),
            release("v0.68.0-nightly.20261030", true),
            release("v0.68.0-nightly.20261029.2", true),
            release("v0.67.0", false),
            release("v0.69.0-nightly.20261031", false),
        ]);
        assert_eq!(
            newest.map(|release| release.tag_name).as_deref(),
            Some("v0.68.0-nightly.20261030")
        );
        assert!(newest_nightly(vec![release("v0.67.0", false)]).is_none());
    }

    #[test]
    fn picks_the_gpui_asset_for_each_install_type_and_never_the_legacy_one() {
        let assets = release_assets();
        let pick = |kind, arch| {
            select_asset(Flavor::Stable, kind, arch, &assets).map(|plan| plan.asset_name)
        };
        assert_eq!(
            pick(nsis(), "x86_64").as_deref(),
            Some("Kopuz_1.0.0_x64-setup.exe")
        );
        assert_eq!(
            pick(msi(), "x86_64").as_deref(),
            Some("Kopuz_1.0.0_x64.msi")
        );
        let appimage = InstallKind::AppImage {
            path: PathBuf::from("/home/k/Kopuz.AppImage"),
        };
        assert_eq!(
            pick(appimage, "x86_64").as_deref(),
            Some("Kopuz_1.0.0_x86_64.AppImage")
        );
        let mac = InstallKind::MacApp {
            bundle: PathBuf::from("/Applications/Kopuz.app"),
        };
        assert_eq!(
            pick(mac.clone(), "aarch64").as_deref(),
            Some("Kopuz_1.0.0_aarch64.dmg")
        );
        assert_eq!(pick(mac, "x86_64"), None);
        assert_eq!(pick(nsis(), "aarch64"), None);
    }

    #[test]
    fn an_asset_without_a_checksum_is_not_installed() {
        let assets: Vec<_> = release_assets()
            .into_iter()
            .filter(|asset| asset.name != "Kopuz_1.0.0_x64-setup.exe.sha256")
            .collect();
        assert_eq!(
            select_asset(Flavor::Stable, nsis(), "x86_64", &assets),
            None
        );
        let plan = select_asset(Flavor::Stable, msi(), "x86_64", &assets).expect("msi plan");
        assert_eq!(
            plan.checksum_url,
            "https://example.invalid/Kopuz_1.0.0_x64.msi.sha256"
        );
    }

    #[test]
    fn each_flavour_installs_only_its_own_app() {
        let assets: Vec<_> = [
            "Kopuz_0.68.0_x86_64.AppImage",
            "Kopuz_0.68.0_x86_64.AppImage.sha256",
            "Kopuz-Nightly_0.68.0-nightly.20261028_x86_64.AppImage",
            "Kopuz-Nightly_0.68.0-nightly.20261028_x86_64.AppImage.sha256",
        ]
        .into_iter()
        .map(asset)
        .collect();
        let appimage = || InstallKind::AppImage {
            path: PathBuf::from("/home/k/Kopuz.AppImage"),
        };
        let pick = |flavor| {
            select_asset(flavor, appimage(), "x86_64", &assets).map(|plan| plan.asset_name)
        };
        assert_eq!(
            pick(Flavor::Stable).as_deref(),
            Some("Kopuz_0.68.0_x86_64.AppImage")
        );
        assert_eq!(
            pick(Flavor::Nightly).as_deref(),
            Some("Kopuz-Nightly_0.68.0-nightly.20261028_x86_64.AppImage")
        );
    }

    #[test]
    fn checksum_accepts_only_the_published_digest() {
        let digest = Sha256::digest(b"kopuz installer");
        let published = format!("{}  Kopuz_1.0.0_x64.msi\r\n", hex::encode(digest));
        assert!(checksum_matches(&digest, &published));
        assert!(checksum_matches(
            &digest,
            &format!("\u{feff}{}", hex::encode(digest).to_uppercase())
        ));
        assert!(!checksum_matches(
            &Sha256::digest(b"tampered installer"),
            &published
        ));
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
        let user = Path::new(r"C:\Users\kyan\AppData\Local\Kopuz");
        let machine = Path::new(r"C:\PROGRAM FILES\Kopuz");
        let lookalike = Path::new(r"C:\Program Files Extra\Kopuz");
        let portable = Path::new(r"D:\Apps\Kopuz");
        let classify =
            |dir: &Path, uninstaller| classify_windows_install(dir, uninstaller, &program_files);

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
            mac_bundle_of(Path::new("/Applications/Kopuz.app/Contents/MacOS/kopuzd")),
            Some(PathBuf::from("/Applications/Kopuz.app"))
        );
        assert_eq!(mac_bundle_of(Path::new("/usr/local/bin/kopuzd")), None);
    }

    #[test]
    fn windows_script_waits_for_both_and_starts_the_app() {
        let installer = Path::new(r"C:\Temp\kopuz-update\Kopuz_1.0.0_x64-setup.exe");
        let nsis_script = windows_apply_script(Flavor::Stable, &nsis(), installer, &[42, 7])
            .expect("nsis script");
        assert!(nsis_script.contains("Wait-Process -Id 42,7"));
        assert!(nsis_script.contains(r"'/S /D=C:\Users\kyan\AppData\Local\Kopuz'"));
        // The host's separator joins `kopuz.exe` on, so only Windows itself writes a backslash there.
        let relaunch = nsis_script.lines().last().expect("a relaunch line");
        assert!(
            relaunch.starts_with(r"Start-Process -FilePath 'C:\Users\kyan\AppData\Local\Kopuz")
        );
        assert!(relaunch.ends_with("kopuz.exe'"));

        let msi_script =
            windows_apply_script(Flavor::Stable, &msi(), Path::new(r"C:\Temp\it's.msi"), &[1])
                .expect("msi script");
        assert!(msi_script.contains(
            r#"'/i "C:\Temp\it''s.msi" /passive /norestart INSTALLDIR="C:\Program Files\Kopuz"'"#
        ));
        let appimage = InstallKind::AppImage {
            path: PathBuf::from("/tmp/Kopuz.AppImage"),
        };
        assert_eq!(
            windows_apply_script(Flavor::Stable, &appimage, installer, &[1]),
            None
        );
    }

    #[test]
    fn windows_script_starts_nightly_by_its_own_binary() {
        let installer = Path::new(r"C:\Temp\kopuz-update\Kopuz-Nightly_1.0.0_x64-setup.exe");
        let script =
            windows_apply_script(Flavor::Nightly, &nsis(), installer, &[1]).expect("nsis script");
        let relaunch = script.lines().last().expect("a relaunch line");
        assert!(relaunch.ends_with("kopuz-nightly.exe'"), "{relaunch}");
    }
}
