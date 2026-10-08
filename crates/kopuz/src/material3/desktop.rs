use std::path::{Path, PathBuf};
use std::time::SystemTime;

use material_colors::color::Argb;

pub(super) async fn css(seed: Option<Argb>) -> Option<String> {
    #[cfg(target_os = "linux")]
    let (accent, dark) = tokio::time::timeout(std::time::Duration::from_secs(2), appearance())
        .await
        .unwrap_or_default();
    #[cfg(not(target_os = "linux"))]
    let (accent, dark) = (None, None::<bool>);
    let seed = seed.or(accent)?;
    Some(match dark {
        Some(dark) => super::color_css(&super::tonal_roles(seed, dark), dark),
        None => super::tonal_css(seed),
    })
}

#[cfg(target_os = "linux")]
async fn appearance() -> (Option<Argb>, Option<bool>) {
    use ashpd::desktop::settings::{ColorScheme, Settings};

    let Ok(settings) = Settings::new().await else {
        return (None, None);
    };
    let (accent, scheme) = tokio::join!(settings.accent_color(), settings.color_scheme());
    let accent = accent
        .ok()
        .and_then(|color| portal_accent([color.red(), color.green(), color.blue()]));
    let dark = match scheme {
        Ok(ColorScheme::PreferDark) => Some(true),
        Ok(ColorScheme::PreferLight) => Some(false),
        _ => None,
    };
    (accent, dark)
}

#[cfg(target_os = "linux")]
fn portal_accent(rgb: [f64; 3]) -> Option<Argb> {
    if !rgb.iter().all(|c| (0.0..=1.0).contains(c)) {
        return None;
    }
    let [r, g, b] = rgb.map(|c| (c * 255.0).round() as u8);
    Some(Argb::new(255, r, g, b))
}

#[derive(Default)]
pub(super) struct Wallpaper {
    file: Option<(PathBuf, SystemTime, u64)>,
    color: Option<Argb>,
}

impl Wallpaper {
    pub(super) fn seed(&mut self, live_path: &str) -> Option<Argb> {
        self.color(wallpaper_path()).or_else(|| {
            let path = utils::live_theme::resolve_path(live_path);
            let raw = utils::live_theme::read(&path)?;
            let vars = utils::live_theme::parse(&raw, &path)?;
            vars.get("accent")?.parse().ok()
        })
    }

    fn color(&mut self, path: Option<PathBuf>) -> Option<Argb> {
        let file = path.and_then(|path| {
            let meta = std::fs::metadata(&path).ok()?;
            Some((path, meta.modified().ok()?, meta.len()))
        });
        if file != self.file {
            self.color = file.as_ref().and_then(|(path, _, _)| seed_from_file(path));
            self.file = file;
        }
        self.color
    }
}

fn seed_from_file(path: &Path) -> Option<Argb> {
    if std::fs::metadata(path).ok()?.len() > 32 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    utils::color::palette_from_bytes(&bytes)?
        .first()
        .map(super::argb)
}

fn wallpaper_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(path) = wayland_wallpaper() {
        return Some(path);
    }
    local_path(&wallpaper::get().ok()?)
}

fn local_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with("file:") {
        reqwest::Url::parse(value).ok()?.to_file_path().ok()
    } else {
        let path = PathBuf::from(value);
        path.is_absolute().then_some(path)
    }
}

/// Wayland wallpaper daemons that answer a query with the image on screen:
/// the program, its arguments, and what precedes the path on its output line.
#[cfg(target_os = "linux")]
type Probe<'a> = (&'a str, &'a [&'a str], &'a str);

#[cfg(target_os = "linux")]
const WAYLAND_PROBES: &[Probe<'static>] = &[
    ("hyprctl", &["hyprpaper", "listactive"], " = "),
    ("swww", &["query"], "image: "),
    ("awww", &["query"], "image: "),
];

/// A query answers in milliseconds; one stuck on a wedged daemon socket would
/// otherwise hold the colour loop, and the blocking thread under it, forever.
#[cfg(target_os = "linux")]
const PROBE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
fn wayland_wallpaper() -> Option<PathBuf> {
    if let Some(path) = probe_wallpaper(WAYLAND_PROBES, PROBE_DEADLINE) {
        return Some(path);
    }
    let state_dir = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    wallpaper_from_state(&state_dir.join("quickshell/wallpaper"))
}

#[cfg(target_os = "linux")]
fn probe_wallpaper(probes: &[Probe<'_>], deadline: std::time::Duration) -> Option<PathBuf> {
    probes.iter().find_map(|(program, args, separator)| {
        let text = probe_output(program, args, deadline)?;
        text.lines().find_map(|line| {
            let (_, path) = line.split_once(separator)?;
            local_path(path).filter(|path| path.is_file())
        })
    })
}

/// stdout of a successful run, or `None` if it fails or outlives `deadline`,
/// in which case the child is killed and reaped rather than left running.
#[cfg(target_os = "linux")]
fn probe_output(program: &str, args: &[&str], deadline: std::time::Duration) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Drained on its own thread so a chatty child cannot fill the pipe and
    // block before it exits.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout.read_to_end(&mut out).ok().map(|_| out)
    });
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            _ => {
                tracing::debug!(program, "wallpaper probe timed out");
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()??;
    status
        .success()
        .then(|| String::from_utf8_lossy(&out).into_owned())
}

#[cfg(target_os = "linux")]
fn wallpaper_from_state(path: &Path) -> Option<PathBuf> {
    local_path(&std::fs::read_to_string(path).ok()?).filter(|path| path.is_file())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn a_hung_probe_is_killed_and_the_next_source_still_answers() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let image = dir.path().join("wall paper.png");
        std::fs::write(&image, b"").unwrap();
        let hang = format!("echo $$ > '{}'; exec sleep 60", pid_file.display());
        let answer = format!(
            "echo 'DP-1: currently displaying: image: {}'",
            image.display()
        );
        let probes = [
            ("sh", &["-c", hang.as_str()][..], "image: "),
            ("sh", &["-c", answer.as_str()][..], "image: "),
        ];

        let started = std::time::Instant::now();
        let found = probe_wallpaper(&probes, std::time::Duration::from_millis(300));
        assert_eq!(found, Some(image));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));

        let pid = std::fs::read_to_string(&pid_file).unwrap();
        assert!(
            !Path::new("/proc").join(pid.trim()).exists(),
            "the hung probe was left running or unreaped"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wallpaper_change_invalidates_the_cached_palette() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("wallpaper");
        let blue = dir.path().join("blue image.png");
        let red = dir.path().join("red image.png");
        image::RgbImage::from_pixel(32, 32, image::Rgb([40, 80, 210]))
            .save(&blue)
            .unwrap();
        image::RgbImage::from_pixel(32, 32, image::Rgb([210, 40, 60]))
            .save(&red)
            .unwrap();
        let mut cache = Wallpaper::default();
        std::fs::write(&state, blue.to_str().unwrap()).unwrap();
        let first = cache.color(wallpaper_from_state(&state)).unwrap();
        assert_eq!(cache.color(wallpaper_from_state(&state)), Some(first));
        std::fs::write(&state, red.to_str().unwrap()).unwrap();
        let second = cache.color(wallpaper_from_state(&state)).unwrap();
        assert_ne!(first, second);
        std::fs::remove_file(&red).unwrap();
        assert_eq!(cache.color(wallpaper_from_state(&state)), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_accent_rejects_unset_values_and_preserves_srgb() {
        assert_eq!(
            portal_accent([0.207843, 0.517647, 0.894118]),
            Some(Argb::from_u32(0xff3584e4))
        );
        for value in [-1.0, 1.01, f64::NAN, f64::INFINITY] {
            assert_eq!(portal_accent([value, 0.5, 0.5]), None);
        }
    }

    #[test]
    fn wallpaper_paths_keep_spaces_and_reject_non_file_urls() {
        assert_eq!(
            local_path("file:///tmp/my%20wallpaper.png"),
            Some(PathBuf::from("/tmp/my wallpaper.png"))
        );
        assert_eq!(
            local_path("/tmp/my wallpaper.png\n"),
            Some(PathBuf::from("/tmp/my wallpaper.png"))
        );
        assert_eq!(local_path("https://example.com/wallpaper.png"), None);
        assert_eq!(local_path(""), None);
    }
}
