//! The user's own browser profiles, and the cookies in one of them. Listing
//! reads `Local State` and `profiles.ini`; checking a profile for a session
//! reads the host and name columns of a copy of its cookie store, which are
//! never encrypted; importing reads a copy of the store through the browser
//! itself, started headless on the copy, or decrypts it. Nothing here starts,
//! signals or locks the user's browser or writes to its profile, so all of it
//! works while the browser is open.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use config::{Browser, BrowserEngine};

use super::store::{Cookie, host_matches_domain};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserProfile {
    pub browser: Browser,
    /// The profile directory: the one holding the cookie store.
    pub dir: PathBuf,
    pub name: String,
    /// The account the browser itself is signed in to, where it says.
    pub email: Option<String>,
}

impl BrowserProfile {
    /// Stable across listings, so a client can name the one it picked.
    pub fn id(&self) -> String {
        format!("{}:{}", self.browser.id(), self.dir.display())
    }
}

/// Chromium data directories under `~/Library/Application Support` (macOS)
/// or the XDG config directory (Linux), and Firefox ones the same way.
fn data_dirs(browser: Browser, mac: bool) -> &'static [&'static str] {
    match (browser, mac) {
        (Browser::Helium, _) => &["net.imput.helium"],
        (Browser::Chrome, false) => &["google-chrome"],
        (Browser::Chrome, true) => &["Google/Chrome"],
        (Browser::Chromium, false) => &["chromium"],
        (Browser::Chromium, true) => &["Chromium"],
        (Browser::Brave, _) => &["BraveSoftware/Brave-Browser"],
        (Browser::Vivaldi, false) => &["vivaldi"],
        (Browser::Vivaldi, true) => &["Vivaldi"],
        (Browser::Edge, false) => &["microsoft-edge"],
        (Browser::Edge, true) => &["Microsoft Edge"],
        (Browser::Firefox, false) => &["mozilla/firefox"],
        (Browser::Firefox, true) => &["Firefox"],
        (Browser::LibreWolf, false) => &["librewolf/librewolf"],
        (Browser::LibreWolf, true) => &["librewolf"],
        (Browser::Zen, _) => &["zen"],
        (Browser::Floorp, false) => &["floorp"],
        (Browser::Floorp, true) => &["Floorp"],
    }
}

/// Firefox and its forks still default to a dot directory in `$HOME` on Linux.
fn home_dirs(browser: Browser) -> &'static [&'static str] {
    match browser {
        Browser::Firefox => &[".mozilla/firefox"],
        Browser::LibreWolf => &[".librewolf"],
        Browser::Zen => &[".zen"],
        Browser::Floorp => &[".floorp"],
        _ => &[],
    }
}

/// Chromium keeps its `User Data` under `%LOCALAPPDATA%`, Firefox and its
/// forks their `profiles.ini` under `%APPDATA%`.
fn windows_roots(browser: Browser, local: &Path, roaming: &Path) -> Vec<PathBuf> {
    let (base, dir) = match browser {
        Browser::Helium => (local, r"imput\Helium\User Data"),
        Browser::Chrome => (local, r"Google\Chrome\User Data"),
        Browser::Chromium => (local, r"Chromium\User Data"),
        Browser::Brave => (local, r"BraveSoftware\Brave-Browser\User Data"),
        Browser::Vivaldi => (local, r"Vivaldi\User Data"),
        Browser::Edge => (local, r"Microsoft\Edge\User Data"),
        Browser::Firefox => (roaming, r"Mozilla\Firefox"),
        Browser::LibreWolf => (roaming, "librewolf"),
        Browser::Zen => (roaming, "zen"),
        Browser::Floorp => (roaming, "Floorp"),
    };
    vec![base.join(dir)]
}

fn mac_roots(browser: Browser, home: &Path) -> Vec<PathBuf> {
    let support = home.join("Library/Application Support");
    data_dirs(browser, true)
        .iter()
        .map(|d| support.join(d))
        .collect()
}

/// The native locations, then the same inside a Flatpak's `~/.var/app`.
fn linux_roots(browser: Browser, home: &Path, config: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = data_dirs(browser, false)
        .iter()
        .map(|d| config.join(d))
        .collect();
    roots.extend(home_dirs(browser).iter().map(|d| home.join(d)));
    for id in super::browser::browser_flatpak_ids(browser) {
        let app = home.join(".var/app").join(id);
        roots.extend(
            data_dirs(browser, false)
                .iter()
                .map(|d| app.join("config").join(d)),
        );
        roots.extend(home_dirs(browser).iter().map(|d| app.join(d)));
    }
    roots
}

/// Every place `browser` may keep its profiles on this machine.
fn roots(browser: Browser) -> Vec<PathBuf> {
    let Some(dirs) = directories::BaseDirs::new() else {
        return Vec::new();
    };
    if cfg!(target_os = "windows") {
        windows_roots(browser, dirs.data_local_dir(), dirs.data_dir())
    } else if cfg!(target_os = "macos") {
        mac_roots(browser, dirs.home_dir())
    } else {
        linux_roots(browser, dirs.home_dir(), dirs.config_dir())
    }
}

/// Every browser profile on this machine that has a cookie store.
pub fn list() -> Vec<BrowserProfile> {
    Browser::ALL
        .iter()
        .copied()
        .flat_map(|browser| {
            roots(browser)
                .into_iter()
                .flat_map(move |root| match browser.engine() {
                    BrowserEngine::Chromium => chromium_profiles(browser, &root),
                    BrowserEngine::Gecko => gecko_profiles(browser, &root),
                })
        })
        .collect()
}

fn chromium_store(dir: &Path) -> Option<PathBuf> {
    [dir.join("Network").join("Cookies"), dir.join("Cookies")]
        .into_iter()
        .find(|p| p.is_file())
}

/// The profiles `Local State` knows about, in the browser's own order, that
/// still have a cookie store.
fn chromium_profiles(browser: Browser, root: &Path) -> Vec<BrowserProfile> {
    let Ok(text) = std::fs::read_to_string(root.join("Local State")) else {
        return Vec::new();
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(cache) = state["profile"]["info_cache"].as_object() else {
        return Vec::new();
    };
    let order: Vec<&str> = state["profile"]["profiles_order"]
        .as_array()
        .map(|o| o.iter().filter_map(|k| k.as_str()).collect())
        .unwrap_or_default();
    let mut keys: Vec<&String> = cache.keys().collect();
    keys.sort_by_key(|k| {
        (
            order.iter().position(|o| o == k).unwrap_or(usize::MAX),
            k.as_str() != "Default",
            k.as_str(),
        )
    });
    keys.into_iter()
        .filter(|key| chromium_store(&root.join(key)).is_some())
        .map(|key| {
            let info = &cache[key.as_str()];
            let text = |field: &str| {
                info[field]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            BrowserProfile {
                browser,
                dir: root.join(key),
                name: text("name").unwrap_or_else(|| key.clone()),
                email: text("user_name"),
            }
        })
        .collect()
}

/// `[Profile*]` sections of `profiles.ini` whose directory has a cookie
/// store, the default one first.
fn gecko_profiles(browser: Browser, root: &Path) -> Vec<BrowserProfile> {
    #[derive(Default)]
    struct Section {
        name: Option<String>,
        path: Option<String>,
        relative: bool,
        default: bool,
    }
    let Ok(text) = std::fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    let mut found: Vec<(bool, BrowserProfile)> = Vec::new();
    let mut finish = |section: Option<Section>| {
        let Some(Section {
            name: Some(name),
            path: Some(path),
            relative,
            default,
        }) = section
        else {
            return;
        };
        let dir = if relative {
            root.join(&path)
        } else {
            PathBuf::from(&path)
        };
        if dir.join("cookies.sqlite").is_file() {
            found.push((
                default,
                BrowserProfile {
                    browser,
                    dir,
                    name,
                    email: None,
                },
            ));
        }
    };
    let mut section: Option<Section> = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            finish(section.take());
            if line.starts_with("[Profile") {
                section = Some(Section {
                    relative: true,
                    ..Section::default()
                });
            }
            continue;
        }
        let (Some(current), Some((key, value))) = (section.as_mut(), line.split_once('=')) else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Name" => current.name = Some(value.to_owned()),
            "Path" => current.path = Some(value.to_owned()),
            "IsRelative" => current.relative = value == "1",
            "Default" => current.default = value == "1",
            _ => {}
        }
    }
    finish(section.take());
    found.sort_by_key(|(default, _)| !default);
    found.into_iter().map(|(_, profile)| profile).collect()
}

/// A private directory under `scratch` for one read, removed with its
/// contents when dropped: it ends up holding a copy of someone's cookies.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(parent: &Path) -> Result<Self, String> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = parent.join(format!(
            "{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&dir)
            .map_err(|e| format!("create {}: {e}", dir.display()))?;
        Ok(Self { dir })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Copy a SQLite store and whatever of its journal exists to `to`. The `-shm`
/// stays behind: SQLite rebuilds it from the log.
fn copy_store(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)?;
    for suffix in ["-wal", "-journal"] {
        let journal = with_suffix(from, suffix);
        if journal.is_file() {
            std::fs::copy(&journal, with_suffix(to, suffix))?;
        }
    }
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// A Gecko `expiry`, which moved from seconds to milliseconds, in seconds.
pub(crate) fn gecko_expiry_secs(expiry: i64) -> i64 {
    if expiry > 100_000_000_000 {
        expiry / 1000
    } else {
        expiry
    }
}

/// Seconds since the Unix epoch, now.
pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Whether the rows hold every one of `names` for `domain`, unexpired. The
/// expiry is Chromium's microseconds since 1601 or Gecko's Unix time, and 0
/// is a session cookie, which lives as long as the browser does.
fn holds_all(
    engine: BrowserEngine,
    rows: &[(String, String, i64)],
    domain: &str,
    names: &[&str],
    now: i64,
) -> bool {
    let live = |expiry: i64| match engine {
        BrowserEngine::Chromium => expiry == 0 || expiry > (now + 11_644_473_600) * 1_000_000,
        BrowserEngine::Gecko => expiry == 0 || gecko_expiry_secs(expiry) > now,
    };
    names.iter().all(|wanted| {
        rows.iter().any(|(host, name, expiry)| {
            name == wanted && host_matches_domain(host, domain) && live(*expiry)
        })
    })
}

/// Whether `profile` holds cookies named `names` for `domain`, read from a
/// copy of its store without decrypting anything.
pub async fn holds_cookies(
    profile: &BrowserProfile,
    scratch: &Path,
    domain: &str,
    names: &[&str],
) -> Result<bool, String> {
    use sqlx::{ConnectOptions, Row};

    let (store, query) = match profile.browser.engine() {
        BrowserEngine::Chromium => (
            chromium_store(&profile.dir),
            "SELECT host_key AS host, name, expires_utc AS expiry FROM cookies",
        ),
        BrowserEngine::Gecko => (
            Some(profile.dir.join("cookies.sqlite")).filter(|p| p.is_file()),
            // Partitioned and container cookies carry origin attributes and
            // are not what a YouTube tab is sent.
            "SELECT host, name, expiry FROM moz_cookies WHERE originAttributes = ''",
        ),
    };
    let store = store.ok_or_else(|| format!("no cookie store in {}", profile.dir.display()))?;
    let copy = Scratch::new(scratch)?;
    let db = copy.dir.join("cookies");
    copy_store(&store, &db).map_err(|e| format!("copy {}: {e}", store.display()))?;
    // Read-write, so opening the copy replays the journal copied with it.
    let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db)
        .create_if_missing(false)
        .connect()
        .await
        .map_err(|e| format!("open {}: {e}", store.display()))?;
    let rows: Vec<(String, String, i64)> = sqlx::query(query)
        .fetch_all(&mut conn)
        .await
        .map_err(|e| format!("query {}: {e}", store.display()))?
        .into_iter()
        .map(|row| {
            (
                row.try_get("host").unwrap_or_default(),
                row.try_get("name").unwrap_or_default(),
                row.try_get("expiry").unwrap_or_default(),
            )
        })
        .collect();
    drop(conn);
    Ok(holds_all(
        profile.browser.engine(),
        &rows,
        domain,
        names,
        now_unix(),
    ))
}

/// Every cookie `profile` holds for `domain`. A Chromium profile is copied,
/// its store and `Local State` laid out as a profile of their own under
/// `scratch`, and read as a profile kopuz made itself would be; the copy is
/// deleted afterwards. Firefox's store is unencrypted and read from a
/// snapshot.
pub async fn read_cookies(
    profile: &BrowserProfile,
    scratch: &Path,
    domain: &str,
) -> Result<Vec<Cookie>, String> {
    let browser = profile.browser;
    if browser.engine() == BrowserEngine::Gecko {
        return super::mozilla::read_cookies(browser, &profile.dir, domain).await;
    }
    let store = chromium_store(&profile.dir)
        .ok_or_else(|| format!("no cookie store in {}", profile.dir.display()))?;
    let copy = Scratch::new(scratch)?;
    // Windows Chromium holds its store open exclusively while it runs.
    let locked = |e: std::io::Error| {
        format!(
            "could not copy {}'s cookies ({e}). Close {} and try again.",
            browser.label(),
            browser.label()
        )
    };
    let relative = store
        .strip_prefix(&profile.dir)
        .unwrap_or(Path::new("Cookies"));
    copy_store(&store, &copy.dir.join("Default").join(relative)).map_err(locked)?;
    if let Some(root) = profile.dir.parent() {
        let state = root.join("Local State");
        if state.is_file() {
            std::fs::copy(&state, copy.dir.join("Local State")).map_err(locked)?;
        }
    }
    super::store::read_profile_cookies(
        browser,
        &copy.dir,
        domain,
        &["--profile-directory=Default".to_string()],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    #[test]
    fn chromium_profiles_come_from_local_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        write(
            &root.join("Local State"),
            r#"{"profile":{"profiles_order":["Profile 1","Default"],"info_cache":{
                "Default":{"name":"Personal","user_name":""},
                "Profile 1":{"name":"Work","user_name":"me@example.com"},
                "Profile 2":{"name":"Deleted"}}}}"#,
        );
        write(&root.join("Default/Network/Cookies"), "");
        write(&root.join("Profile 1/Cookies"), "");
        let profiles = chromium_profiles(Browser::Helium, root);
        let names: Vec<_> = profiles
            .iter()
            .map(|p| (p.name.as_str(), p.email.as_deref()))
            .collect();
        assert_eq!(
            names,
            [("Work", Some("me@example.com")), ("Personal", None)]
        );
        assert_eq!(profiles[1].dir, root.join("Default"));
        assert_eq!(
            chromium_store(&profiles[1].dir),
            Some(root.join("Default/Network/Cookies"))
        );
    }

    #[test]
    fn gecko_profiles_come_from_profiles_ini() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let elsewhere = root.join("elsewhere");
        write(
            &root.join("profiles.ini"),
            &format!(
                "[General]\nStartWithLastProfile=1\n\n\
                 [Profile1]\nName=dev\nIsRelative=1\nPath=abc.dev\n\n\
                 [Profile0]\nName=default-release\nIsRelative=1\nPath=xyz.default-release\nDefault=1\n\n\
                 [Profile2]\nName=empty\nIsRelative=1\nPath=empty\n\n\
                 [Profile3]\nName=absolute\nIsRelative=0\nPath={}\n\n\
                 [Install4F96D1932A9F858E]\nDefault=xyz.default-release\n",
                elsewhere.display()
            ),
        );
        write(&root.join("abc.dev/cookies.sqlite"), "");
        write(&root.join("xyz.default-release/cookies.sqlite"), "");
        write(&elsewhere.join("cookies.sqlite"), "");
        let profiles = gecko_profiles(Browser::Firefox, root);
        let names: Vec<_> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["default-release", "dev", "absolute"]);
        assert_eq!(profiles[2].dir, elsewhere);
    }

    #[test]
    fn roots_cover_each_platform_layout() {
        let home = Path::new("/home/u");
        let config = Path::new("/home/u/.config");
        assert_eq!(
            linux_roots(Browser::Helium, home, config),
            [home.join(".config/net.imput.helium")]
        );
        let firefox = linux_roots(Browser::Firefox, home, config);
        assert!(firefox.contains(&home.join(".config/mozilla/firefox")));
        assert!(firefox.contains(&home.join(".mozilla/firefox")));
        assert!(firefox.contains(&home.join(".var/app/org.mozilla.firefox/.mozilla/firefox")));
        assert_eq!(
            mac_roots(Browser::Chrome, Path::new("/Users/u")),
            [PathBuf::from(
                "/Users/u/Library/Application Support/Google/Chrome"
            )]
        );
        let local = Path::new("C:/Users/u/AppData/Local");
        let roaming = Path::new("C:/Users/u/AppData/Roaming");
        assert_eq!(
            windows_roots(Browser::Edge, local, roaming),
            [local.join(r"Microsoft\Edge\User Data")]
        );
        assert_eq!(
            windows_roots(Browser::Firefox, local, roaming),
            [roaming.join(r"Mozilla\Firefox")]
        );
    }

    #[test]
    fn a_session_needs_every_cookie_unexpired_on_the_domain() {
        let now = 1_800_000_000;
        let chrome_time = |unix: i64| (unix + 11_644_473_600) * 1_000_000;
        let row = |host: &str, name: &str, expiry: i64| (host.into(), name.into(), expiry);
        let names = ["SID", "SAPISID"];

        let signed_in = [
            row(".youtube.com", "SID", chrome_time(now + 60)),
            row(".youtube.com", "SAPISID", 0),
            row(".google.com", "SID", chrome_time(now + 60)),
        ];
        assert!(holds_all(
            BrowserEngine::Chromium,
            &signed_in,
            "youtube.com",
            &names,
            now
        ));

        let google_only = [
            row(".google.com", "SID", chrome_time(now + 60)),
            row(".google.com", "SAPISID", chrome_time(now + 60)),
        ];
        assert!(!holds_all(
            BrowserEngine::Chromium,
            &google_only,
            "youtube.com",
            &names,
            now
        ));

        let expired = [
            row(".youtube.com", "SID", chrome_time(now - 60)),
            row(".youtube.com", "SAPISID", chrome_time(now + 60)),
        ];
        assert!(!holds_all(
            BrowserEngine::Chromium,
            &expired,
            "youtube.com",
            &names,
            now
        ));

        let gecko_seconds = [
            row(".youtube.com", "SID", now + 60),
            row(".youtube.com", "SAPISID", now + 60),
        ];
        assert!(holds_all(
            BrowserEngine::Gecko,
            &gecko_seconds,
            "youtube.com",
            &names,
            now
        ));
        let gecko_millis = [
            row(".youtube.com", "SID", (now + 60) * 1000),
            row(".youtube.com", "SAPISID", (now - 60) * 1000),
        ];
        assert!(!holds_all(
            BrowserEngine::Gecko,
            &gecko_millis,
            "youtube.com",
            &names,
            now
        ));
    }

    #[tokio::test]
    async fn a_session_is_found_in_a_copy_of_the_store() {
        use sqlx::{ConnectOptions, Executor};

        let dir = tempfile::tempdir().expect("tempdir");
        let profile_dir = dir.path().join("profile");
        std::fs::create_dir_all(&profile_dir).expect("mkdir");
        let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(profile_dir.join("cookies.sqlite"))
            .create_if_missing(true)
            .connect()
            .await
            .expect("create store");
        conn.execute(
            "CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, expiry INTEGER, originAttributes TEXT)",
        )
        .await
        .expect("create table");
        for (name, attributes) in [
            ("SID", ""),
            ("SAPISID", ""),
            ("LOGIN_INFO", "^partitionKey=%28https%2Cexample.com%29"),
        ] {
            sqlx::query("INSERT INTO moz_cookies VALUES ('.youtube.com', ?1, 'v', 0, ?2)")
                .bind(name)
                .bind(attributes)
                .execute(&mut conn)
                .await
                .expect("insert");
        }
        drop(conn);
        let profile = BrowserProfile {
            browser: Browser::Firefox,
            dir: profile_dir,
            name: "default".into(),
            email: None,
        };
        let scratch = dir.path().join("scratch");
        let found = holds_cookies(&profile, &scratch, "youtube.com", &["SID", "SAPISID"])
            .await
            .expect("read");
        assert!(found);
        let missing = holds_cookies(&profile, &scratch, "youtube.com", &["SID", "LOGIN_INFO"])
            .await
            .expect("read");
        assert!(!missing);
        let left: Vec<_> = std::fs::read_dir(&scratch).expect("scratch").collect();
        assert!(left.is_empty(), "copy left behind");
    }
}
