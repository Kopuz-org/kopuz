pub(crate) mod browser;
pub(crate) mod cdp;
#[cfg(not(target_os = "android"))]
pub(crate) mod mozilla;
pub(crate) mod profile;
pub(crate) mod profiles;
pub(crate) mod signin;
pub(crate) mod store;
#[cfg(target_os = "windows")]
pub(crate) mod windows_native;

pub use browser::{detect_default_browser, has_host_spawn, resolve_browser};
pub use profile::{delete_profile, profile_dir};
pub use profiles::{BrowserProfile, browser_profiles};
pub(crate) use signin::launch_signin_and_extract;

pub(crate) use profile::has_cookie;
pub(crate) use store::read_profile_cookies;
pub use store::{Cookie, read_cookies};
