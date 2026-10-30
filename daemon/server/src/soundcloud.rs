//! SoundCloud's browser sign-in. The source itself is a plugin
//! (`plugins/soundcloud`); signing in drives a browser profile on this
//! machine, which is the host's job, and yields the OAuth token the plugin's
//! sessions carry.

use crate::plugins;

/// The signed-in account's numeric id, which is the source's `user_id`.
pub async fn derive_user_id(token: &str) -> Option<String> {
    let connection = plugin_host::wit::Connection {
        service: config::MusicService::SoundCloud.id().to_string(),
        url: String::new(),
        token: token.to_string(),
        user_id: String::new(),
        device_id: String::new(),
        account: None,
        folders: Vec::new(),
    };
    match plugins::session(connection)?.user_id().await {
        Ok(Ok(id)) => id,
        Ok(Err(error)) => {
            tracing::debug!(%error, "SoundCloud user id lookup failed");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "SoundCloud plugin failed");
            None
        }
    }
}

/// One-time SoundCloud sign-in via an isolated browser profile, reusing the
/// shared [`crate::cookies`] machinery. We extract the single `oauth_token`
/// cookie the web app sends as `Authorization: OAuth <token>`.
pub mod signin {
    use std::time::Duration;

    use config::Browser;

    use crate::cookies;

    const PROFILE_PREFIX: &str = "sc-profile";

    pub fn delete_profile(server_id: &str) -> std::io::Result<()> {
        cookies::delete_profile(PROFILE_PREFIX, server_id)
    }

    #[tracing::instrument(name = "sc.signin", skip(server_id, signin_timeout), fields(browser = %browser))]
    pub async fn launch_signin_and_extract(
        browser: Browser,
        server_id: &str,
        signin_timeout: Duration,
    ) -> Result<String, String> {
        cookies::launch_signin_and_extract(
            browser,
            server_id,
            PROFILE_PREFIX,
            "https://soundcloud.com/signin",
            "soundcloud.com",
            signin_timeout,
            |cookies| {
                cookies
                    .iter()
                    .find(|c| c.name == "oauth_token" && !c.value.is_empty())
                    .map(|c| c.value.clone())
            },
        )
        .await
    }
}
