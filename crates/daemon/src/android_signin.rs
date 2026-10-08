//! Android provider authentication through the app's own login activity.

use config::MusicService;

fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.trim().split_once('=')?;
        (key == name && !value.is_empty()).then_some(value)
    })
}

fn credential(service: MusicService, header: &str) -> Option<String> {
    match service {
        MusicService::YtMusic => (cookie_value(header, "SAPISID").is_some()
            && cookie_value(header, "SID").is_some())
        .then(|| header.to_string()),
        MusicService::SoundCloud => cookie_value(header, "oauth_token").map(str::to_string),
        MusicService::AppleMusic => cookie_value(header, "media-user-token").map(str::to_string),
        _ => None,
    }
}

#[cfg(target_os = "android")]
pub async fn sign_in(
    service: MusicService,
    client_id: String,
) -> Result<(String, Option<String>), String> {
    // CookieManager belongs to the app: a second login must not clear the
    // cookies of a flow that is already open.
    static SIGNIN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _lock = SIGNIN
        .try_lock()
        .map_err(|_| "Finish the open sign-in window first")?;
    struct CloseOnDrop;
    impl Drop for CloseOnDrop {
        fn drop(&mut self) {
            player::systemint::login_close();
        }
    }
    let _close = CloseOnDrop;

    if service == MusicService::Spotify {
        let auth = tokio::select! {
            result = server::spotify::auth::authorize_with(client_id, player::systemint::login_open) => result?,
            error = wait_for_close() => return Err(error),
        };
        return Ok((
            server::spotify::auth::pack_token(&auth.access_token, &auth.refresh_token),
            Some(auth.user_id),
        ));
    }

    let (sign_in_url, cookie_url) = match service {
        MusicService::YtMusic => (
            server::ytmusic::isolated_profile::SIGNIN_URL,
            "https://music.youtube.com",
        ),
        MusicService::SoundCloud => ("https://soundcloud.com/signin", "https://soundcloud.com"),
        MusicService::AppleMusic => (
            server::applemusic::signin::SIGNIN_URL,
            "https://music.apple.com",
        ),
        _ => return Err("This source signs in with a username and password".into()),
    };
    player::systemint::login_open(sign_in_url)?;
    let secret = tokio::select! {
        result = tokio::time::timeout(std::time::Duration::from_secs(300), async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                // Opening the activity clears the old jar asynchronously. Do
                // not accept any cookie until that operation has finished.
                if player::systemint::login_is_open()
                    && let Some(header) = player::systemint::login_cookies(cookie_url)
                    && let Some(secret) = credential(service, &header)
                {
                    return secret;
                }
            }
        }) => result.map_err(|_| "Sign-in timed out")?,
        error = wait_for_close() => return Err(error),
    };
    let user = match service {
        MusicService::YtMusic => {
            if !server::provider::validate_ytmusic_cookies(&secret).await {
                return Err(
                    "Sign-in completed but YouTube Music did not accept the session".into(),
                );
            }
            server::ytmusic::derive_user_id(&secret)
        }
        MusicService::SoundCloud => Some(
            server::soundcloud::derive_user_id(&secret)
                .await
                .ok_or("SoundCloud did not accept the sign-in session")?,
        ),
        _ => Some("me".to_string()),
    };
    Ok((secret, user))
}

#[cfg(target_os = "android")]
async fn wait_for_close() -> String {
    let started = tokio::time::Instant::now();
    let mut seen_open = false;
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if player::systemint::login_is_open() {
            seen_open = true;
        } else if seen_open {
            return "The sign-in window was closed".into();
        } else if started.elapsed() >= std::time::Duration::from_secs(10) {
            return "Could not open the sign-in WebView".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_requires_a_complete_session_and_keeps_its_cookie_header() {
        for partial in [
            "SAPISID=one",
            "SID=two",
            "SAPISID=; SID=two",
            "SAPISID=one; SID=",
        ] {
            assert!(credential(MusicService::YtMusic, partial).is_none());
        }
        let header = "SAPISID=one; SID=two; other=three";
        assert_eq!(
            credential(MusicService::YtMusic, header).as_deref(),
            Some(header)
        );
    }

    #[test]
    fn cookie_providers_extract_only_their_own_nonempty_token() {
        let header = "other=ignored; oauth_token=sc=token; media-user-token=am-token";
        assert_eq!(
            credential(MusicService::SoundCloud, header).as_deref(),
            Some("sc=token")
        );
        assert_eq!(
            credential(MusicService::AppleMusic, header).as_deref(),
            Some("am-token")
        );
        assert!(credential(MusicService::AppleMusic, "media-user-token=").is_none());
        assert!(credential(MusicService::SoundCloud, "not_oauth_token=other").is_none());
        assert!(credential(MusicService::Spotify, header).is_none());
    }
}
