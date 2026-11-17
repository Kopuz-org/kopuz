//! SoundCloud's browser sign-in and share links. The source itself is a plugin
//! (`plugins/soundcloud`); signing in drives a browser profile on this
//! machine, which is the host's job, and yields the OAuth token the plugin's
//! sessions carry.

use std::time::Duration;

use crate::plugins;

/// The embed player's page. It names a track's public page in its
/// `<link rel="canonical">` and, unlike `api-v2`, asks for no client id.
const EMBED_PLAYER: &str = "https://w.soundcloud.com/player/";

/// A track's public page. Its address is the uploader's and the track's slugs,
/// which the numeric id does not carry, so SoundCloud is asked for it.
pub async fn track_page_url(track_id: &str) -> Option<String> {
    track_page_from(EMBED_PLAYER, track_id).await
}

async fn track_page_from(player: &str, track_id: &str) -> Option<String> {
    if track_id.is_empty() || !track_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let client = utils::http::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let response = match client
        .get(player)
        .query(&[(
            "url",
            format!("https://api.soundcloud.com/tracks/{track_id}"),
        )])
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(error = %error.without_url(), "SoundCloud page lookup failed");
            return None;
        }
    };
    if !response.status().is_success() {
        tracing::debug!(status = %response.status(), "SoundCloud has no page for this track");
        return None;
    }
    canonical_page(&response.text().await.ok()?)
}

/// The page a `<link rel="canonical">` names, when it is a track's.
fn canonical_page(html: &str) -> Option<String> {
    html.split("<link")
        .skip(1)
        .filter_map(|chunk| chunk.split_once('>').map(|(tag, _)| tag))
        .find(|tag| attr(tag, "rel") == Some("canonical"))
        .and_then(|tag| attr(tag, "href"))
        .filter(|href| is_track_page(href))
        .map(str::to_string)
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let rest = &tag[tag.find(&key)? + key.len()..];
    Some(&rest[..rest.find('"')?])
}

/// The link is copied for someone to open, so it is held to SoundCloud's own
/// `/uploader/track` pages whatever the response says.
fn is_track_page(href: &str) -> bool {
    reqwest::Url::parse(href).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("soundcloud.com")
            && url
                .path_segments()
                .is_some_and(|segments| segments.filter(|part| !part.is_empty()).count() >= 2)
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://soundcloud.com/kllhhr/topman-w-izaya-tiji-prod";

    fn player_page(canonical: &str) -> String {
        format!(
            "<html><head><link rel=\"dns-prefetch\" href=\"//api.soundcloud.com\">\n\
             <link rel=\"canonical\" href=\"{canonical}\">\n<title>SoundCloud Widget</title></head></html>"
        )
    }

    /// An embed player that knows track 42 and no other.
    async fn player() -> String {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = server.local_addr().expect("address");
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut stream, _)) = server.accept().await {
                let mut request = [0u8; 4096];
                let read = stream.read(&mut request).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]);
                let known = request.lines().next().is_some_and(|line| {
                    line.contains("url=https%3A%2F%2Fapi.soundcloud.com%2Ftracks%2F42 ")
                });
                let (status, body) = match known {
                    true => ("200 OK", player_page(PAGE)),
                    false => ("404 Not Found", player_page("")),
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{address}/player/")
    }

    #[tokio::test]
    async fn a_track_id_resolves_to_the_page_soundcloud_names_for_it() {
        let player = player().await;
        assert_eq!(track_page_from(&player, "42").await.as_deref(), Some(PAGE));
        assert_eq!(track_page_from(&player, "43").await, None);
        assert_eq!(track_page_from(&player, "42&url=elsewhere").await, None);
        assert_eq!(track_page_from(&player, "").await, None);
    }

    #[test]
    fn only_a_soundcloud_track_page_is_shared() {
        assert_eq!(canonical_page(&player_page(PAGE)).as_deref(), Some(PAGE));
        for other in [
            "",
            "https://soundcloud.com/",
            "https://soundcloud.com/kllhhr",
            "http://soundcloud.com/kllhhr/topman-w-izaya-tiji-prod",
            "https://soundcloud.com.example/kllhhr/topman-w-izaya-tiji-prod",
        ] {
            assert_eq!(canonical_page(&player_page(other)), None, "{other}");
        }
        assert_eq!(canonical_page("<html></html>"), None);
    }
}
