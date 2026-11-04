pub(crate) mod browser;
pub(crate) mod cdp;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) mod mozilla;
pub(crate) mod profile;
pub mod profiles;
pub(crate) mod signin;
pub(crate) mod store;
#[cfg(target_os = "windows")]
pub(crate) mod windows_native;

pub use browser::{detect_default_browser, has_host_spawn, resolve_browser};
pub use profile::{delete_profile, profile_dir};
pub use profiles::{BrowserProfile, ProfileError, browser_profiles, scratch_dir};
pub(crate) use signin::launch_signin_and_extract;

pub(crate) use profile::has_cookie;
pub(crate) use store::read_profile_cookies;
pub use store::{Cookie, read_cookies};
