pub(crate) mod browser;
pub(crate) mod profile;
pub(crate) mod signin;
pub(crate) mod store;
#[cfg(target_os = "windows")]
pub(crate) mod windows_native;

pub use profile::{delete_profile, profile_dir};
pub use signin::launch_signin_and_extract;

/// Pick an installed Chromium-family browser for sign-in, honoring `preferred`
/// but demoting Brave (its fingerprint shields make TIDAL's DataDome device
/// check fail with `UNSUPPORTED_OS`). Returns `None` if none are installed.
pub async fn first_available_browser(preferred: config::Browser) -> Option<config::Browser> {
    let mut order: Vec<config::Browser> = std::iter::once(preferred)
        .chain(
            config::Browser::ALL
                .iter()
                .copied()
                .filter(|b| *b != preferred),
        )
        .collect();
    // Stable sort keeps the preference order but sinks Brave below the rest.
    order.sort_by_key(|b| (*b == config::Browser::Brave) as u8);
    for b in order {
        if browser::find_browser_bin(b).await.is_some() {
            return Some(b);
        }
    }
    None
}

pub(crate) use profile::has_cookie;
pub(crate) use store::read_cookies;
