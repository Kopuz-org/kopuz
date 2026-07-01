//! `cargo install kopuz` only drops the bare executable into `~/.cargo/bin` —
//! Cargo has no post-install hook, so nothing creates the launcher entry, icon,
//! or app bundle a user would get from a `.deb`/`.dmg`. As a result the
//! freshly-installed app is invisible to the OS application search.
//!
//! To match the packaged experience, the app registers itself on launch: Linux
//! gets a freedesktop `.desktop` entry + hicolor icons, macOS gets a thin `.app`
//! bundle in `~/Applications` (so Spotlight/Launchpad index it). The work is
//! idempotent and cheap on repeat launches — a stamp file records the app
//! version and the executable path it was registered for, and we only redo the
//! work when one of those changes (e.g. after an upgrade or a reinstall to a new
//! location).
#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::io;
use std::path::PathBuf;

const APP_NAME: &str = "Kopuz";
#[cfg(target_os = "linux")]
const APP_ID: &str = "kopuz";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
#[cfg(target_os = "linux")]
const COMMENT: &str = "A modern, lightweight music player built with Rust and Dioxus.";

fn app_data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|dirs| dirs.data_dir().to_path_buf())
}

fn stamp_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join("desktop-integration.stamp"))
}

const STAMP_SCHEMA: u32 = 2;

fn stamp_contents(exe: &std::path::Path) -> String {
    format!("{STAMP_SCHEMA}\n{APP_VERSION}\n{}", exe.display())
}

pub fn sync() {
    std::thread::spawn(|| {
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("desktop integration: cannot resolve current exe: {e}");
                return;
            }
        };

        if is_up_to_date(&exe) {
            return;
        }

        match platform::install(&exe) {
            Ok(()) => {
                write_stamp(&exe);
                tracing::info!("desktop integration: registered launcher entry");
            }
            Err(e) => tracing::warn!("desktop integration: registration failed: {e}"),
        }
    });
}

fn is_up_to_date(exe: &std::path::Path) -> bool {
    let Some(stamp) = stamp_path() else {
        return false;
    };
    let Ok(contents) = std::fs::read_to_string(&stamp) else {
        return false;
    };
    contents == stamp_contents(exe) && platform::primary_artifact_exists()
}

fn write_stamp(exe: &std::path::Path) {
    let Some(stamp) = stamp_path() else { return };
    if let Some(parent) = stamp.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&stamp, stamp_contents(exe)) {
        tracing::warn!("desktop integration: cannot write stamp: {e}");
    }
}
#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    const LOGO: &[u8] = include_bytes!("../assets/logo-512.png");

    fn applications_dir() -> PathBuf {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                home.join(".local/share")
            })
            .join("applications")
    }

    fn desktop_file() -> PathBuf {
        applications_dir().join(format!("{APP_ID}.desktop"))
    }

    fn icon_file() -> Option<PathBuf> {
        app_data_dir().map(|dir| dir.join("icon.png"))
    }

    fn exec_value(exe: &std::path::Path) -> String {
        let s = exe.to_string_lossy();
        if s.contains(|c: char| c.is_whitespace() || "\"'\\$`".contains(c)) {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        } else {
            s.into_owned()
        }
    }

    pub fn primary_artifact_exists() -> bool {
        desktop_file().exists()
    }

    pub fn install(exe: &std::path::Path) -> io::Result<()> {
        let icon = icon_file()
            .ok_or_else(|| io::Error::other("cannot resolve data dir for launcher icon"))?;
        if let Some(parent) = icon.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&icon, LOGO)?;

        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Version=1.0\n\
             Name={APP_NAME}\n\
             GenericName=Music Player\n\
             Comment={COMMENT}\n\
             Exec={exec}\n\
             Icon={icon}\n\
             Terminal=false\n\
             Categories=AudioVideo;Audio;Player;\n\
             Keywords=music;player;audio;jellyfin;subsonic;spotify;\n\
             StartupWMClass={APP_NAME}\n\
             StartupNotify=true\n",
            exec = exec_value(exe),
            icon = icon.display(),
        );

        let desktop = desktop_file();
        if let Some(parent) = desktop.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&desktop, entry)?;

        refresh_caches();
        Ok(())
    }

    fn refresh_caches() {
        let _ = std::process::Command::new("update-desktop-database")
            .arg(applications_dir())
            .status();
        for kbuild in ["kbuildsycoca6", "kbuildsycoca5"] {
            let _ = std::process::Command::new(kbuild).status();
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    const ICNS: &[u8] = include_bytes!("../assets/icon.icns");

    fn bundle_dir() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        home.join("Applications").join(format!("{APP_NAME}.app"))
    }

    pub fn primary_artifact_exists() -> bool {
        let exec = bundle_dir().join("Contents/MacOS").join(APP_NAME);
        match std::fs::symlink_metadata(&exec) {
            Ok(meta) => meta.file_type().is_file(),
            Err(_) => false,
        }
    }

    pub fn install(exe: &std::path::Path) -> io::Result<()> {
        let bundle = bundle_dir();
        let contents = bundle.join("Contents");
        let macos = contents.join("MacOS");
        let resources = contents.join("Resources");
        std::fs::create_dir_all(&macos)?;
        std::fs::create_dir_all(&resources)?;

        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>CFBundleName</key>\n\t<string>{APP_NAME}</string>\n\
             \t<key>CFBundleDisplayName</key>\n\t<string>{APP_NAME}</string>\n\
             \t<key>CFBundleExecutable</key>\n\t<string>{APP_NAME}</string>\n\
             \t<key>CFBundleIdentifier</key>\n\t<string>com.temidaradev.kopuz</string>\n\
             \t<key>CFBundleIconFile</key>\n\t<string>{APP_NAME}</string>\n\
             \t<key>CFBundlePackageType</key>\n\t<string>APPL</string>\n\
             \t<key>CFBundleShortVersionString</key>\n\t<string>{APP_VERSION}</string>\n\
             \t<key>CFBundleVersion</key>\n\t<string>{APP_VERSION}</string>\n\
             \t<key>NSHighResolutionCapable</key>\n\t<true/>\n\
             \t<key>LSMinimumSystemVersion</key>\n\t<string>10.11</string>\n\
             </dict>\n\
             </plist>\n"
        );
        std::fs::write(contents.join("Info.plist"), plist)?;
        std::fs::write(resources.join(format!("{APP_NAME}.icns")), ICNS)?;

        let exec_dst = macos.join(APP_NAME);
        let _ = std::fs::remove_file(&exec_dst);
        std::fs::copy(exe, &exec_dst)?;

        register_with_launch_services(&bundle);
        Ok(())
    }

    fn register_with_launch_services(bundle: &std::path::Path) {
        const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/\
            Frameworks/LaunchServices.framework/Support/lsregister";
        let _ = std::process::Command::new(LSREGISTER)
            .arg("-f")
            .arg(bundle)
            .status();
    }
}
