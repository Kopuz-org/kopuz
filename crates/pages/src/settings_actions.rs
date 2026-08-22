use ::server::provider::ProviderClient;
use config::{AppConfig, Browser, MusicService};
use dioxus::prelude::*;
use hooks::ReadDb;
use tracing::Instrument;

pub(crate) async fn ensure_host_access(mut host_access: Signal<bool>) -> Option<()> {
    let host = ::server::cookies::has_host_spawn().await;
    host_access.set(host);
    None
}

async fn validate_ytmusic(cookies: &str) -> bool {
    ::server::provider::validate_ytmusic_cookies(cookies).await
}

async fn try_resume_ytmusic(seed: Option<String>) -> Option<String> {
    if let Some(cookies) = &seed
        && validate_ytmusic(cookies).await
    {
        return seed;
    }
    if let Some(cookies) = &seed
        && let Ok(Some(rotated)) = ::server::ytmusic::verify_session_keepalive::tick(cookies).await
        && validate_ytmusic(&rotated).await
    {
        return Some(rotated);
    }
    None
}

/// Poll-driven Android sign-in: opens the in-app login WebView at `signin_url`
/// and waits until `extract` finds what it needs in the cookies the WebView
/// accumulates for `cookie_url`. Mirrors the desktop flow in
/// `server::cookies::signin`, with Android's app-global CookieManager standing
/// in for the spawned browser profile.
#[cfg(target_os = "android")]
async fn webview_signin(
    signin_url: &str,
    cookie_url: &str,
    extract: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    use std::time::{Duration, Instant};
    player::systemint::login_open(signin_url);
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut seen_open = false;
    loop {
        utils::sleep(Duration::from_secs(2)).await;
        if let Some(header) = player::systemint::login_cookies(cookie_url)
            && let Some(value) = extract(&header)
        {
            player::systemint::login_close();
            return Ok(value);
        }
        if player::systemint::login_is_open() {
            seen_open = true;
        } else if seen_open {
            return Err("the sign-in window was closed".to_string());
        }
        if Instant::now() >= deadline {
            player::systemint::login_close();
            return Err("sign-in timed out".to_string());
        }
    }
}

#[cfg(target_os = "android")]
fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.trim().split_once('=')?;
        (key == name && !value.is_empty()).then(|| value.to_string())
    })
}

#[cfg(target_os = "android")]
async fn ensure_ytmusic_signed_in(
    config_cookies: Option<String>,
    _browser: Browser,
    _server_id: &str,
) -> Result<String, String> {
    if let Some(cookies) = try_resume_ytmusic(config_cookies).await {
        return Ok(cookies);
    }
    let cookies = webview_signin(
        ::server::ytmusic::isolated_profile::SIGNIN_URL,
        "https://music.youtube.com",
        |header| {
            (cookie_value(header, "SAPISID").is_some() && cookie_value(header, "SID").is_some())
                .then(|| header.to_string())
        },
    )
    .await?;
    if !validate_ytmusic(&cookies).await {
        return Err("Sign-in completed but YT validation still failed".to_string());
    }
    Ok(cookies)
}

#[cfg(not(target_os = "android"))]
async fn ensure_ytmusic_signed_in(
    config_cookies: Option<String>,
    browser: Browser,
    server_id: &str,
) -> Result<String, String> {
    if let Some(cookies) = try_resume_ytmusic(config_cookies).await {
        return Ok(cookies);
    }

    let profile = ::server::ytmusic::isolated_profile::profile_dir(server_id);
    if profile.is_dir() {
        let from_profile = ::server::ytmusic::cookies::extract_from(browser, &profile)
            .await
            .ok();
        if let Some(cookies) = try_resume_ytmusic(from_profile).await {
            return Ok(cookies);
        }
    }

    let cookies = ::server::ytmusic::isolated_profile::launch_signin_and_extract(
        browser,
        server_id,
        std::time::Duration::from_secs(300),
    )
    .await?;
    if !validate_ytmusic(&cookies).await {
        return Err("Sign-in completed but YT validation still failed".to_string());
    }
    Ok(cookies)
}

pub fn add_registry(
    mut config: Signal<AppConfig>,
    mut registry_url: Signal<String>,
    mut registry_error: Signal<Option<String>>,
    mut registry_loading: Signal<bool>,
    mut show_add_registry: Signal<bool>,
) {
    let url = registry_url().trim().to_string();
    if url.is_empty() {
        registry_error.set(Some(i18n::t("radio_registry_empty_path").to_string()));
        return;
    }

    if config.read().radio_registries.iter().any(|r| r.url == url) {
        registry_error.set(Some(i18n::t("radio_registry_exists").to_string()));
        return;
    }

    registry_loading.set(true);
    registry_error.set(None);

    spawn(
        async move {
            let mut temp_registry = radio::registry::StationRegistry::new();
            match temp_registry.import_registry(&url).await {
                Ok(_) => {
                    let mut current_config = config.write();
                    if !current_config.radio_registries.iter().any(|r| r.url == url) {
                        current_config.radio_registries.push(config::RegistryEntry {
                            url,
                            enabled: true,
                            is_default: false,
                        });
                    }
                    registry_url.set(String::new());
                    registry_error.set(None);
                    show_add_registry.set(false);
                }
                Err(error) => {
                    registry_error.set(Some(i18n::t_with(
                        "radio_registry_import_failed",
                        &[("error", error.to_string())],
                    )));
                }
            }
            registry_loading.set(false);
        }
        .instrument(tracing::info_span!("radio.import_registry")),
    );
}

/// Persist freshly-obtained browser-sign-in credentials onto the active server
/// and mirror the browser choice into its saved entry. Shared by the YT Music
/// and SoundCloud auto-login flows (the only per-service differences are how the
/// token is obtained and how the user id is derived).
fn apply_browser_login(
    mut config: Signal<AppConfig>,
    browser: Browser,
    token: String,
    user_id: String,
) {
    let mut cfg = config.write();
    let saved_id = cfg.server.as_ref().and_then(|server| server.id.clone());
    if let Some(server) = cfg.server.as_mut() {
        server.access_token = Some(token);
        server.user_id = Some(user_id);
        server.yt_browser = Some(browser);
    }
    if let Some(id) = saved_id
        && let Some(saved) = cfg.servers.iter_mut().find(|server| server.id == id)
    {
        saved.yt_browser = Some(browser);
    }
}

/// What actually rendered the sign-in page, for error messages: the chosen
/// desktop browser, or the in-app WebView on Android (where the browser
/// setting is not consulted at all).
fn signin_surface(browser: Browser) -> String {
    if cfg!(target_os = "android") {
        "WebView".to_string()
    } else {
        browser.to_string()
    }
}

/// Surface a browser sign-in failure to both the settings error line and the
/// player error banner.
fn report_signin_failure(
    mut error: Signal<Option<String>>,
    mut playback_error: Signal<Option<String>>,
    msg: String,
) {
    error.set(Some(msg.clone()));
    playback_error.set(Some(msg));
}

pub fn ytmusic_auto_login(
    config: Signal<AppConfig>,
    yt_browser: Signal<Browser>,
    mut error: Signal<Option<String>>,
    playback_error: Signal<Option<String>>,
) {
    let (browser, existing, server_id) = {
        let cfg = config.peek();
        let srv = cfg.server.as_ref();
        (
            srv.and_then(|s| s.yt_browser).unwrap_or(*yt_browser.peek()),
            srv.and_then(|s| s.access_token.clone())
                .filter(|token| !token.is_empty()),
            srv.and_then(|s| s.id.clone()).unwrap_or_default(),
        )
    };
    spawn(async move {
        let cookies = match ensure_ytmusic_signed_in(existing, browser, &server_id).await {
            Ok(cookies) => cookies,
            Err(err) => {
                report_signin_failure(
                    error,
                    playback_error,
                    format!(
                        "YT Music sign-in failed ({}): {err}",
                        signin_surface(browser)
                    ),
                );
                return;
            }
        };
        let yt_user_id =
            ::server::ytmusic::derive_user_id(&cookies).unwrap_or_else(|| "me".to_string());
        apply_browser_login(config, browser, cookies, yt_user_id);
        error.set(None);
    });
}

pub fn soundcloud_auto_login(
    config: Signal<AppConfig>,
    yt_browser: Signal<Browser>,
    mut error: Signal<Option<String>>,
    playback_error: Signal<Option<String>>,
) {
    let (browser, server_id) = {
        let cfg = config.peek();
        let srv = cfg.server.as_ref();
        (
            srv.and_then(|s| s.yt_browser).unwrap_or(*yt_browser.peek()),
            srv.and_then(|s| s.id.clone()).unwrap_or_default(),
        )
    };
    spawn(async move {
        #[cfg(not(target_os = "android"))]
        let signin = ::server::soundcloud::signin::launch_signin_and_extract(
            browser,
            &server_id,
            std::time::Duration::from_secs(300),
        )
        .await;
        #[cfg(target_os = "android")]
        let signin = {
            let _ = &server_id;
            webview_signin(
                "https://soundcloud.com/signin",
                "https://soundcloud.com",
                |header| cookie_value(header, "oauth_token"),
            )
            .await
        };
        let token = match signin {
            Ok(token) => token,
            Err(err) => {
                report_signin_failure(
                    error,
                    playback_error,
                    format!(
                        "SoundCloud sign-in failed ({}): {err}",
                        signin_surface(browser)
                    ),
                );
                return;
            }
        };
        let user_id = ::server::soundcloud::derive_user_id(&token)
            .await
            .unwrap_or_else(|| "me".to_string());
        apply_browser_login(config, browser, token, user_id);
        error.set(None);
    });
}

/// Spotify OAuth (Authorization-Code + PKCE) sign-in: opens the default browser
/// at the consent screen, captures the redirect on a loopback listener, and
/// stores the packed `<access>\n<refresh>` token + user id on the active server.
/// Unlike YT/SoundCloud this is a real redirect flow, not cookie-scraping, so it
/// takes no browser choice.
pub fn spotify_auto_login(
    mut config: Signal<AppConfig>,
    mut error: Signal<Option<String>>,
    playback_error: Signal<Option<String>>,
) {
    let client_id = config
        .peek()
        .server
        .as_ref()
        .map(|s| s.url.clone())
        .unwrap_or_default();
    spawn(async move {
        let auth = match ::server::spotify::auth::launch_signin_and_extract(client_id).await {
            Ok(auth) => auth,
            Err(err) => {
                report_signin_failure(
                    error,
                    playback_error,
                    format!("Spotify sign-in failed: {err}"),
                );
                return;
            }
        };
        let packed = ::server::spotify::auth::pack_token(&auth.access_token, &auth.refresh_token);
        {
            let mut cfg = config.write();
            if let Some(server) = cfg.server.as_mut() {
                server.access_token = Some(packed);
                server.user_id = Some(auth.user_id);
            }
        }
        error.set(None);
    });
}
pub fn applemusic_auto_login(
    mut config: Signal<AppConfig>,
    yt_browser: Signal<Browser>,
    mut error: Signal<Option<String>>,
    mut playback_error: Signal<Option<String>>,
) {
    let (browser, server_id) = {
        let cfg = config.peek();
        let srv = cfg.server.as_ref();
        (
            srv.and_then(|s| s.yt_browser).unwrap_or(*yt_browser.peek()),
            srv.and_then(|s| s.id.clone()).unwrap_or_default(),
        )
    };
    let mut report = move |msg: String| {
        error.set(Some(msg.clone()));
        playback_error.set(Some(msg));
    };
    spawn(async move {
        let token = match ::server::applemusic::signin::launch_signin_and_extract(
            browser,
            &server_id,
            std::time::Duration::from_secs(300),
        )
        .await
        {
            Ok(token) => token,
            Err(err) => {
                report(format!("Apple Music sign-in failed ({browser}): {err}"));
                return;
            }
        };
        {
            let mut cfg = config.write();
            let saved_id = cfg.server.as_ref().and_then(|server| server.id.clone());
            if let Some(server) = cfg.server.as_mut() {
                server.access_token = Some(token);
                server.user_id = Some("me".to_string());
                server.yt_browser = Some(browser);
            }
            if let Some(id) = saved_id
                && let Some(saved) = cfg.servers.iter_mut().find(|server| server.id == id)
            {
                saved.yt_browser = Some(browser);
            }
        }
        error.set(None);
    });
}

#[allow(clippy::too_many_arguments)]
pub fn add_server(
    mut config: Signal<AppConfig>,
    mut server_name: Signal<String>,
    mut server_url: Signal<String>,
    mut server_service: Signal<MusicService>,
    yt_browser: Signal<Browser>,
    yt_anonymous: Signal<bool>,
    mut error: Signal<Option<String>>,
    mut show_add_server: Signal<bool>,
    mut show_login: Signal<bool>,
    playback_error: Signal<Option<String>>,
    apple_music_storefront: Signal<String>,
    apple_music_language: Signal<String>,
    mut apple_music_manual_token: Signal<String>,
    apple_music_use_manual: Signal<bool>,
) {
    let selected_service = server_service();
    let is_ytmusic = selected_service == MusicService::YtMusic;
    let is_soundcloud = selected_service == MusicService::SoundCloud;
    let is_spotify = selected_service == MusicService::Spotify;
    let is_browser_signin = selected_service.uses_browser_signin();

    if server_name().trim().is_empty() {
        error.set(Some(i18n::t("server_name_required").to_string()));
        return;
    }

    if !is_browser_signin && !server_url().starts_with("http") {
        error.set(Some(i18n::t("invalid_server_url").to_string()));
        return;
    }

    if is_spotify && server_url().trim().is_empty() {
        error.set(Some(
            "Enter your Spotify app Client ID (create one at developer.spotify.com)".to_string(),
        ));
        return;
    }

    // Manual mode is the one path that never opens a browser, so an empty field
    // here isn't "sign in later" — it saves a server with no credential and no
    // way to acquire one.
    if selected_service == MusicService::AppleMusic
        && *apple_music_use_manual.peek()
        && apple_music_manual_token().trim().is_empty()
    {
        error.set(Some(
            "Enter your Apple Music media-user-token, or switch to browser sign-in".to_string(),
        ));
        return;
    }

    let name_input = server_name();
    let url_input = server_url();

    spawn(
        async move {
            let display_name = name_input.trim().to_string();

            let effective_url = if is_ytmusic {
                "https://music.youtube.com".to_string()
            } else if is_soundcloud {
                "https://soundcloud.com".to_string()
            } else if selected_service == MusicService::AppleMusic {
                "https://music.apple.com".to_string()
            } else if is_spotify {
                url_input.trim().to_string()
            } else {
                url_input
            };

            let mut new_server = config::MusicServer::new_with_service(
                display_name,
                effective_url,
                selected_service,
            );
            let is_anon = is_ytmusic && *yt_anonymous.peek();
            new_server.yt_anonymous = is_anon;
            if is_anon {
                new_server.access_token = Some(String::new());
            }
            new_server.yt_browser = (is_browser_signin && !is_anon).then(|| *yt_browser.peek());
            // Apple Music: set storefront, language, and optionally manual token.
            // The token only applies in manual mode — the signal keeps its value
            // between saves, so reading it unconditionally would attach the
            // previous server's token to one that is about to sign in through
            // the browser, and leave it there if that sign-in fails.
            if selected_service == MusicService::AppleMusic {
                new_server.apple_music_storefront = apple_music_storefront();
                new_server.apple_music_language = apple_music_language();
                if *apple_music_use_manual.peek() {
                    let manual = apple_music_manual_token();
                    if !manual.is_empty() {
                        new_server.access_token = Some(manual);
                        new_server.user_id = Some("me".to_string());
                    }
                }
            }
            let saved = config::SavedServer::from_music_server(&new_server);
            {
                let mut cfg = config.write();
                cfg.add_saved_server(saved);
                cfg.set_active_server_snapshot(new_server);
            }

            server_name.set(String::new());
            server_url.set(String::new());
            server_service.set(MusicService::Jellyfin);
            // Cleared with the rest of the form: it has been persisted, and a
            // credential left in a live signal is one the next server can pick up.
            apple_music_manual_token.set(String::new());
            error.set(None);
            show_add_server.set(false);

            if is_ytmusic && !is_anon {
                ytmusic_auto_login(config, yt_browser, error, playback_error);
            } else if is_soundcloud {
                soundcloud_auto_login(config, yt_browser, error, playback_error);
            } else if selected_service == MusicService::AppleMusic
                && !*apple_music_use_manual.peek()
            {
                applemusic_auto_login(config, yt_browser, error, playback_error);
            } else if is_spotify {
                spotify_auto_login(config, error, playback_error);
            } else if !is_browser_signin {
                show_login.set(true);
            }
        }
        .instrument(tracing::info_span!("source.add_server")),
    );
}

pub fn switch_server(
    config: Signal<AppConfig>,
    db: ReadDb,
    id: String,
    yt_browser: Signal<Browser>,
    error: Signal<Option<String>>,
    mut show_login: Signal<bool>,
    playback_error: Signal<Option<String>>,
) {
    spawn(async move {
        let Some(service) = config.peek().find_saved_server(&id).map(|s| s.service) else {
            return;
        };

        let usable =
            hooks::source_switch::apply_source_switch(config, db, config::Source::Server(id)).await;
        if usable {
            return;
        }

        match service {
            MusicService::YtMusic => ytmusic_auto_login(config, yt_browser, error, playback_error),
            MusicService::SoundCloud => {
                soundcloud_auto_login(config, yt_browser, error, playback_error)
            }
            MusicService::AppleMusic => {
                applemusic_auto_login(config, yt_browser, error, playback_error)
            }
            MusicService::Spotify => spotify_auto_login(config, error, playback_error),
            _ => show_login.set(true),
        }
    });
}

pub fn delete_saved(mut config: Signal<AppConfig>, id: String) {
    let service = config
        .peek()
        .find_saved_server(&id)
        .map(|server| server.service);
    config.write().remove_saved_server(&id);
    match service {
        Some(MusicService::YtMusic) => {
            let _ = ::server::ytmusic::isolated_profile::delete_profile(&id);
        }
        Some(MusicService::SoundCloud) => {
            let _ = ::server::soundcloud::signin::delete_profile(&id);
        }
        Some(MusicService::AppleMusic) => {
            let _ = ::server::applemusic::signin::delete_profile(&id);
        }
        _ => {}
    }
}

pub fn login_with_password(
    mut config: Signal<AppConfig>,
    mut username: Signal<String>,
    mut password: Signal<String>,
    mut login_error: Signal<Option<String>>,
    mut is_loading: Signal<bool>,
    mut show_login: Signal<bool>,
) {
    if username().is_empty() || password().is_empty() {
        login_error.set(Some(i18n::t("username_and_password_required").to_string()));
        return;
    }

    if let Some(server) = &config.read().server {
        let service = server.service;
        let server_url = server.url.clone();
        let device_id = config.read().device_id.clone();
        let user = username();
        let pass = password();

        is_loading.set(true);
        login_error.set(None);

        spawn(async move {
            let remote = ProviderClient::new(service, server_url, device_id);
            let result = remote.login(&user, &pass).await;

            is_loading.set(false);

            match result {
                Ok(session) => {
                    if let Some(server) = config.write().server.as_mut() {
                        server.access_token = Some(session.access_token);
                        server.user_id = Some(session.user_id);
                    }
                    username.set(String::new());
                    password.set(String::new());
                    login_error.set(None);
                    show_login.set(false);
                }
                Err(error) => {
                    login_error.set(Some(i18n::t_with(
                        "login_failed",
                        &[("error", error.to_string())],
                    )));
                }
            }
        });
    }
}
