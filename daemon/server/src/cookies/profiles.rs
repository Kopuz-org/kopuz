//! The person's own browser profiles, as opposed to the isolated ones kopuz
//! launches for a sign-in. Listing reads `Local State` and `profiles.ini`
//! only, so it never starts, signals or locks a running browser.

use std::path::{Path, PathBuf};
#[cfg(not(target_os = "android"))]
use std::sync::atomic::{AtomicU64, Ordering};

use config::{Browser, BrowserEngine};

#[cfg(not(target_os = "android"))]
use super::store::{Cookie, host_matches_domain};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserProfile {
    pub browser: Browser,
    /// What [`super::read_cookies`] takes as its profile root.
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

/// Every profile with a cookie store, browser by browser in
/// [`Browser::ALL`] order and each browser's own order within it.
pub fn browser_profiles() -> Vec<BrowserProfile> {
    let Some(dirs) = directories::BaseDirs::new() else {
        return Vec::new();
    };
    Browser::ALL
        .iter()
        .flat_map(|&browser| {
            roots(browser, &dirs)
                .into_iter()
                .flat_map(move |root| match browser.engine() {
                    BrowserEngine::Chromium => chromium_profiles(browser, &root),
                    BrowserEngine::Gecko => gecko_profiles(browser, &root),
                })
        })
        .collect()
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
/// Data directories holding a Chromium `Local State` or a Gecko
/// `profiles.ini`, relative to the platform's application data directory.
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

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android")))]
/// Gecko browsers still default to a dot directory in `$HOME` on Linux.
fn home_dirs(browser: Browser) -> &'static [&'static str] {
    match browser {
        Browser::Firefox => &[".mozilla/firefox"],
        Browser::LibreWolf => &[".librewolf"],
        Browser::Zen => &[".zen"],
        Browser::Floorp => &[".floorp"],
        _ => &[],
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android")))]
fn flatpak_id(browser: Browser) -> Option<&'static str> {
    match browser {
        Browser::Chrome => Some("com.google.Chrome"),
        Browser::Chromium => Some("org.chromium.Chromium"),
        Browser::Brave => Some("com.brave.Browser"),
        Browser::Vivaldi => Some("com.vivaldi.Vivaldi"),
        Browser::Edge => Some("com.microsoft.Edge"),
        Browser::Firefox => Some("org.mozilla.firefox"),
        Browser::LibreWolf => Some("io.gitlab.librewolf-community"),
        Browser::Zen => Some("app.zen_browser.zen"),
        Browser::Floorp => Some("one.ablaze.floorp"),
        Browser::Helium => None,
    }
}

/// Chromium keeps its `User Data` under `%LOCALAPPDATA%`, Gecko its
/// `profiles.ini` under `%APPDATA%`.
#[cfg(target_os = "windows")]
fn roots(browser: Browser, dirs: &directories::BaseDirs) -> Vec<PathBuf> {
    let (base, dir) = match browser {
        Browser::Helium => (dirs.data_local_dir(), r"imput\Helium\User Data"),
        Browser::Chrome => (dirs.data_local_dir(), r"Google\Chrome\User Data"),
        Browser::Chromium => (dirs.data_local_dir(), r"Chromium\User Data"),
        Browser::Brave => (
            dirs.data_local_dir(),
            r"BraveSoftware\Brave-Browser\User Data",
        ),
        Browser::Vivaldi => (dirs.data_local_dir(), r"Vivaldi\User Data"),
        Browser::Edge => (dirs.data_local_dir(), r"Microsoft\Edge\User Data"),
        Browser::Firefox => (dirs.data_dir(), r"Mozilla\Firefox"),
        Browser::LibreWolf => (dirs.data_dir(), "librewolf"),
        Browser::Zen => (dirs.data_dir(), "zen"),
        Browser::Floorp => (dirs.data_dir(), "Floorp"),
    };
    vec![base.join(dir)]
}

#[cfg(target_os = "macos")]
fn roots(browser: Browser, dirs: &directories::BaseDirs) -> Vec<PathBuf> {
    let support = dirs.home_dir().join("Library/Application Support");
    data_dirs(browser, true)
        .iter()
        .map(|d| support.join(d))
        .collect()
}

/// Android has no desktop browser profile to read.
#[cfg(target_os = "android")]
fn roots(_browser: Browser, _dirs: &directories::BaseDirs) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android")))]
fn roots(browser: Browser, dirs: &directories::BaseDirs) -> Vec<PathBuf> {
    let (home, config) = (dirs.home_dir(), dirs.config_dir());
    let mut roots: Vec<PathBuf> = data_dirs(browser, false)
        .iter()
        .map(|d| config.join(d))
        .collect();
    roots.extend(home_dirs(browser).iter().map(|d| home.join(d)));
    if let Some(id) = flatpak_id(browser) {
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

fn chromium_store(dir: &Path) -> Option<PathBuf> {
    [dir.join("Network").join("Cookies"), dir.join("Cookies")]
        .into_iter()
        .find(|p| p.is_file())
}

/// The profiles `Local State` knows about that still have a cookie store, in
/// the browser's own order.
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
    let mut sections = Vec::new();
    let mut current: Option<Section> = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            sections.extend(current.take());
            if line.starts_with("[Profile") {
                current = Some(Section {
                    relative: true,
                    ..Section::default()
                });
            }
            continue;
        }
        let (Some(section), Some((key, value))) = (current.as_mut(), line.split_once('=')) else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Name" => section.name = Some(value.to_string()),
            "Path" => section.path = Some(value.to_string()),
            "IsRelative" => section.relative = value == "1",
            "Default" => section.default = value == "1",
            _ => {}
        }
    }
    sections.extend(current);
    sections.sort_by_key(|s| !s.default);
    sections
        .into_iter()
        .filter_map(|s| {
            let (name, path) = (s.name?, s.path?);
            let dir = if s.relative {
                root.join(path)
            } else {
                PathBuf::from(path)
            };
            dir.join("cookies.sqlite")
                .is_file()
                .then_some(BrowserProfile {
                    browser,
                    dir,
                    name,
                    email: None,
                })
        })
        .collect()
}

#[cfg(not(target_os = "android"))]
/// Removed with its contents when dropped: it holds a copy of someone's cookies.
struct Scratch {
    dir: PathBuf,
}

#[cfg(not(target_os = "android"))]
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

#[cfg(not(target_os = "android"))]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(not(target_os = "android"))]
/// The `-shm` stays behind: SQLite rebuilds it from the log.
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

#[cfg(not(target_os = "android"))]
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(not(target_os = "android"))]
/// A Gecko `expiry`, which moved from seconds to milliseconds, in seconds.
pub(crate) fn gecko_expiry_secs(expiry: i64) -> i64 {
    if expiry > 100_000_000_000 {
        expiry / 1000
    } else {
        expiry
    }
}

#[cfg(not(target_os = "android"))]
pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(not(target_os = "android"))]
/// Expiry is Chromium's microseconds since 1601 or Gecko's Unix time; 0 is a
/// session cookie, which lives as long as the browser does.
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

#[cfg(not(target_os = "android"))]
/// Host and name columns are never encrypted, so this reads a copy without decrypting.
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

#[cfg(not(target_os = "android"))]
/// A Chromium profile's store and `Local State` are copied under `scratch` and
/// read like a profile kopuz made itself. Firefox's store is unencrypted.
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
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn lists_chromium_profiles_from_local_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("Local State"),
            r#"{"profile":{"profiles_order":["Profile 1","Default"],"info_cache":{
                "Default":{"name":"Personal"},
                "Profile 1":{"name":"Work"},
                "Profile 2":{"name":"Deleted"}}}}"#,
        );
        write(&root.join("Default/Network/Cookies"), "");
        write(&root.join("Profile 1/Cookies"), "");
        let names: Vec<String> = chromium_profiles(Browser::Chrome, root)
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Work", "Personal"]);
    }

    #[test]
    fn lists_gecko_profiles_default_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("profiles.ini"),
            "[General]\nStartWithLastProfile=1\n\n\
             [Profile1]\nName=other\nIsRelative=1\nPath=Profiles/b.other\n\n\
             [Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/a.default\nDefault=1\n\n\
             [Profile2]\nName=empty\nIsRelative=1\nPath=Profiles/c.empty\n",
        );
        write(&root.join("Profiles/a.default/cookies.sqlite"), "");
        write(&root.join("Profiles/b.other/cookies.sqlite"), "");
        let profiles = gecko_profiles(Browser::Firefox, root);
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["default-release", "other"]);
        assert_eq!(profiles[0].dir, root.join("Profiles/a.default"));
    }

    #[cfg(not(target_os = "android"))]
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

    #[cfg(not(target_os = "android"))]
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
