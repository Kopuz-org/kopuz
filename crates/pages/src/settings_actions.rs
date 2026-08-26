use std::sync::Arc;

use api::KopuzApi;
use config::{Browser, MusicService};
use dioxus::prelude::*;

pub(crate) async fn ensure_host_access(mut host_access: Signal<bool>) -> Option<()> {
    let host = ::server::cookies::has_host_spawn().await;
    host_access.set(host);
    None
}

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

#[cfg(not(target_os = "android"))]
async fn authenticate_source(
    api: Arc<dyn KopuzApi>,
    server_id: String,
    _service: MusicService,
    _browser: Browser,
    _client_id: String,
) -> Result<(), api::ApiError> {
    api.authenticate_source(server_id).await.map(|_| ())
}

#[cfg(target_os = "android")]
async fn authenticate_source(
    api: Arc<dyn KopuzApi>,
    server_id: String,
    service: MusicService,
    browser: Browser,
    client_id: String,
) -> Result<(), api::ApiError> {
    let (secret, user_id) = match service {
        MusicService::YtMusic => {
            let cookies = webview_signin(
                ::server::ytmusic::isolated_profile::SIGNIN_URL,
                "https://music.youtube.com",
                |header| {
                    (cookie_value(header, "SAPISID").is_some()
                        && cookie_value(header, "SID").is_some())
                    .then(|| header.to_string())
                },
            )
            .await
            .map_err(api::ApiError::internal)?;
            let user_id =
                ::server::ytmusic::derive_user_id(&cookies).unwrap_or_else(|| "me".to_string());
            (cookies, user_id)
        }
        MusicService::SoundCloud => {
            let token = webview_signin(
                "https://soundcloud.com/signin",
                "https://soundcloud.com",
                |header| cookie_value(header, "oauth_token"),
            )
            .await
            .map_err(api::ApiError::internal)?;
            let user_id = ::server::soundcloud::derive_user_id(&token)
                .await
                .unwrap_or_else(|| "me".to_string());
            (token, user_id)
        }
        MusicService::AppleMusic => {
            let token = webview_signin(
                ::server::applemusic::signin::SIGNIN_URL,
                "https://music.apple.com",
                |header| cookie_value(header, ::server::applemusic::signin::TOKEN_COOKIE),
            )
            .await
            .map_err(api::ApiError::internal)?;
            (token, "me".to_string())
        }
        MusicService::Spotify => {
            let auth = ::server::spotify::auth::launch_signin_and_extract(client_id)
                .await
                .map_err(api::ApiError::internal)?;
            (
                ::server::spotify::auth::pack_token(&auth.access_token, &auth.refresh_token),
                auth.user_id,
            )
        }
        _ => {
            return Err(api::ApiError::unsupported(
                "this source uses username and password authentication",
            ));
        }
    };
    api.provision_credentials(api::CredentialProvision {
        server_id,
        secret,
        user_id: Some(user_id),
        browser: Some(browser.id().to_string()),
    })
    .await
    .map(|_| ())
}

fn report_signin_failure(
    mut error: Signal<Option<String>>,
    mut playback_error: Signal<Option<String>>,
    message: String,
) {
    error.set(Some(message.clone()));
    playback_error.set(Some(message));
}

pub fn add_registry(
    mut registry_url: Signal<String>,
    mut registry_error: Signal<Option<String>>,
    mut registry_loading: Signal<bool>,
    mut show_add_registry: Signal<bool>,
) {
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let url = registry_url().trim().to_string();
    if url.is_empty() {
        registry_error.set(Some(i18n::t("radio_registry_empty_path").to_string()));
        return;
    }
    registry_loading.set(true);
    registry_error.set(None);
    spawn(async move {
        match api.add_radio_registry(url).await {
            Ok(()) => {
                registry_url.set(String::new());
                registry_error.set(None);
                show_add_registry.set(false);
            }
            Err(error) => registry_error.set(Some(i18n::t_with(
                "radio_registry_import_failed",
                &[("error", error.to_string())],
            ))),
        }
        registry_loading.set(false);
    });
}

fn browser_login(
    yt_browser: Signal<Browser>,
    mut error: Signal<Option<String>>,
    playback_error: Signal<Option<String>>,
    expected: MusicService,
) {
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let sources = consume_context::<Signal<Vec<api::SourceInfo>>>();
    let Some(server) = sources
        .peek()
        .iter()
        .find(|source| {
            source.active && source.service == Some(hooks::music_service_to_api(expected))
        })
        .cloned()
    else {
        return;
    };
    let browser = server
        .browser
        .as_deref()
        .and_then(Browser::from_id)
        .unwrap_or(*yt_browser.peek());
    spawn(async move {
        match authenticate_source(
            api,
            server.id,
            expected,
            browser,
            server.url.unwrap_or_default(),
        )
        .await
        {
            Ok(()) => error.set(None),
            Err(auth_error) => report_signin_failure(
                error,
                playback_error,
                format!("{} sign-in failed: {auth_error}", expected.display_name()),
            ),
        }
    });
}

pub fn ytmusic_auto_login(
    yt_browser: Signal<Browser>,
    error: Signal<Option<String>>,
    playback_error: Signal<Option<String>>,
) {
    browser_login(yt_browser, error, playback_error, MusicService::YtMusic);
}

pub fn applemusic_auto_login(
    yt_browser: Signal<Browser>,
    error: Signal<Option<String>>,
    playback_error: Signal<Option<String>>,
) {
    browser_login(yt_browser, error, playback_error, MusicService::AppleMusic);
}

#[allow(clippy::too_many_arguments)]
pub fn add_server(
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
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let service = server_service();
    let browser_signin = service.uses_browser_signin();
    let storefront = apple_music_storefront().trim().to_string();
    if server_name().trim().is_empty() {
        error.set(Some(i18n::t("server_name_required").to_string()));
        return;
    }
    if !browser_signin && !server_url().starts_with("http") {
        error.set(Some(i18n::t("invalid_server_url").to_string()));
        return;
    }
    if service == MusicService::Spotify && server_url().trim().is_empty() {
        error.set(Some(
            "Enter your Spotify app Client ID (create one at developer.spotify.com)".to_string(),
        ));
        return;
    }
    let manual_apple = service == MusicService::AppleMusic && *apple_music_use_manual.peek();
    if manual_apple && apple_music_manual_token().trim().is_empty() {
        error.set(Some(
            "Enter your Apple Music media-user-token, or switch to browser sign-in".to_string(),
        ));
        return;
    }

    if service == MusicService::AppleMusic && storefront.is_empty() {
        error.set(Some("Enter an Apple Music storefront ID".to_string()));
        return;
    }

    let name = server_name().trim().to_string();
    let url = match service {
        MusicService::YtMusic => "https://music.youtube.com".to_string(),
        MusicService::SoundCloud => "https://soundcloud.com".to_string(),
        MusicService::AppleMusic => "https://music.apple.com".to_string(),
        _ => server_url().trim_end_matches('/').to_string(),
    };
    let browser = *yt_browser.peek();
    let anonymous = service == MusicService::YtMusic && *yt_anonymous.peek();
    let language = apple_music_language();
    let manual_token = apple_music_manual_token();
    spawn(async move {
        let source = match api
            .upsert_server(api::ServerDraft {
                id: None,
                name,
                url: url.clone(),
                service: hooks::music_service_to_api(service),
                browser: (browser_signin && !anonymous).then(|| browser.id().to_string()),
                anonymous,
                storefront: (service == MusicService::AppleMusic).then_some(storefront),
                language: (service == MusicService::AppleMusic).then_some(language),
            })
            .await
        {
            Ok(source) => source,
            Err(api_error) => {
                error.set(Some(api_error.to_string()));
                return;
            }
        };
        if let Err(api_error) = api.switch_source(source.id.clone()).await {
            error.set(Some(api_error.to_string()));
            return;
        }
        let auth_result = if manual_apple {
            api.provision_credentials(api::CredentialProvision {
                server_id: source.id.clone(),
                secret: manual_token,
                user_id: Some("me".to_string()),
                browser: Some(browser.id().to_string()),
            })
            .await
            .map(|_| ())
        } else if browser_signin && !anonymous {
            authenticate_source(api.clone(), source.id, service, browser, url).await
        } else {
            Ok(())
        };
        if let Err(auth_error) = auth_result {
            report_signin_failure(error, playback_error, auth_error.to_string());
            return;
        }

        server_name.set(String::new());
        server_url.set(String::new());
        server_service.set(MusicService::Jellyfin);
        apple_music_manual_token.set(String::new());
        error.set(None);
        show_add_server.set(false);
        if !browser_signin {
            show_login.set(true);
        }
    });
}

pub fn switch_server(
    api: Arc<dyn KopuzApi>,
    source: api::SourceInfo,
    yt_browser: Signal<Browser>,
    mut error: Signal<Option<String>>,
    mut show_login: Signal<bool>,
    playback_error: Signal<Option<String>>,
) {
    let Some(service) = source.service.and_then(hooks::music_service_from_api) else {
        return;
    };
    let id = source.id;
    let browser = source
        .browser
        .as_deref()
        .and_then(Browser::from_id)
        .unwrap_or(*yt_browser.peek());
    let url = source.url.unwrap_or_default();
    spawn(async move {
        match api.switch_source(id.clone()).await {
            Ok(source) if source.authenticated => return,
            Ok(_) => {}
            Err(switch_error) => {
                error.set(Some(switch_error.to_string()));
                return;
            }
        }
        if service.uses_browser_signin() {
            if let Err(auth_error) = authenticate_source(api, id, service, browser, url).await {
                report_signin_failure(error, playback_error, auth_error.to_string());
            }
        } else {
            show_login.set(true);
        }
    });
}

pub fn delete_saved(id: String) {
    let api = consume_context::<Arc<dyn KopuzApi>>();
    spawn(async move {
        if let Err(error) = api.delete_server(id).await {
            tracing::warn!(%error, "could not delete media server");
        }
    });
}

pub fn login_with_password(
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
    let sources = consume_context::<Signal<Vec<api::SourceInfo>>>();
    let Some(server_id) = sources
        .peek()
        .iter()
        .find(|source| source.active && source.kind == api::SourceKind::Server)
        .map(|source| source.id.clone())
    else {
        return;
    };
    let api = consume_context::<Arc<dyn KopuzApi>>();
    let user = username();
    let pass = password();
    is_loading.set(true);
    login_error.set(None);
    spawn(async move {
        let result = api
            .login_source(api::SourceLoginRequest {
                server_id,
                username: user,
                password: pass,
            })
            .await;
        is_loading.set(false);
        match result {
            Ok(_) => {
                username.set(String::new());
                password.set(String::new());
                login_error.set(None);
                show_login.set(false);
            }
            Err(error) => login_error.set(Some(i18n::t_with(
                "login_failed",
                &[("error", error.to_string())],
            ))),
        }
    });
}
