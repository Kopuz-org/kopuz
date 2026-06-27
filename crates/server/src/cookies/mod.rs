pub(crate) mod browser;
pub(crate) mod profile;
pub(crate) mod signin;
pub(crate) mod store;
#[cfg(target_os = "windows")]
pub(crate) mod windows_native;

pub use profile::{delete_profile, profile_dir};
pub use signin::launch_signin_and_extract;

pub(crate) use profile::has_cookie;
pub(crate) use store::read_cookies;
