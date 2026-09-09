//! What the settings page does when a button is pressed.
//!
//! Adding a server, signing into one, switching between them and deleting one
//! are all daemon calls now. The browser sign-in windows, the cookie
//! scraping, the OAuth loopback and the credentials they produce never come
//! near this process: a form is filled in here, and what comes back is
//! whether the source ended up authenticated.

use config::{AppConfig, Browser, MusicService};
use dioxus::prelude::*;
use tracing::Instrument;

/// Whether the daemon can open a browser for a sign-in. A sandboxed daemon
/// cannot, and the add-server dialog says so instead of failing later.
pub(crate) async fn ensure_host_access(mut host_access: Signal<bool>) -> Option<()> {
    let api = hooks::consume_api();
    host_access.set(api.can_open_browser().await.unwrap_or(true));
    None
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

    let api = hooks::consume_api();
    spawn(
        async move {
            match api.validate_radio_registry(url.clone()).await {
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

type Api = std::sync::Arc<dyn api::KopuzApi>;

/// Run a source's sign-in in the daemon and report the outcome. Every service
/// with a browser flow goes through here: which window opens, what it scrapes
/// and where the secret is kept are the daemon's business.
pub fn authenticate(
    server_id: String,
    error: Signal<Option<String>>,
    playback_error: Signal<Option<String>>,
) {
    let api = hooks::consume_api();
    spawn(
        authenticate_with(api, server_id, error, playback_error)
            .instrument(tracing::info_span!("source.authenticate")),
    );
}

async fn authenticate_with(
    api: Api,
    server_id: String,
    mut error: Signal<Option<String>>,
    mut playback_error: Signal<Option<String>>,
) {
    match api.authenticate_source(server_id).await {
        Ok(_) => error.set(None),
        Err(failure) => {
            let message = i18n::t_with("signin_failed", &[("error", failure.to_string())]);
            error.set(Some(message.clone()));
            playback_error.set(Some(message));
        }
    }
}

/// Make a server the active source and, if the daemon says it has no usable
/// credentials, start whichever sign-in that service uses.
async fn activate(
    api: Api,
    id: String,
    error: Signal<Option<String>>,
    mut show_login: Signal<bool>,
    playback_error: Signal<Option<String>>,
) {
    let source = match api.switch_source(id).await {
        Ok(source) => source,
        Err(failure) => {
            tracing::warn!(%failure, "switching source failed");
            hooks::toast::toast_error(&failure.to_string());
            return;
        }
    };
    // Which sign-in a source takes belongs to the service, and the daemon runs
    // it -- this only asks for whichever it named.
    match source.sign_in {
        api::SignInKind::None => (),
        api::SignInKind::Browser => authenticate_with(api, source.id, error, playback_error).await,
        api::SignInKind::Password => show_login.set(true),
    }
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
    show_login: Signal<bool>,
    playback_error: Signal<Option<String>>,
    apple_music_storefront: Signal<String>,
    apple_music_language: Signal<String>,
    mut apple_music_manual_token: Signal<String>,
    apple_music_use_manual: Signal<bool>,
) {
    let selected_service = server_service();
    let is_ytmusic = selected_service == MusicService::YtMusic;
    let is_apple = selected_service == MusicService::AppleMusic;
    let anonymous = is_ytmusic && yt_anonymous();
    let manual_token = is_apple && *apple_music_use_manual.peek();

    let mut values = vec![
        api::FieldValue::new("url", server_url().trim()),
        api::FieldValue::new("client_id", server_url().trim()),
        api::FieldValue::new("browser", yt_browser().id()),
    ];
    if anonymous {
        values.push(api::FieldValue::new("auth_method", "anonymous"));
    } else if manual_token {
        values.push(api::FieldValue::new("auth_method", "manual"));
    } else {
        values.push(api::FieldValue::new("auth_method", "browser"));
    }
    if is_apple {
        values.push(api::FieldValue::new(
            "storefront",
            apple_music_storefront().trim(),
        ));
        values.push(api::FieldValue::new("language", apple_music_language()));
    }
    let draft = api::ServerDraft {
        id: None,
        name: server_name().trim().to_string(),
        service: selected_service.id().to_string(),
        values,
        secrets: if manual_token {
            vec![api::FieldValue::new(
                "token",
                apple_music_manual_token().trim(),
            )]
        } else {
            Vec::new()
        },
    };
    let api = hooks::consume_api();

    spawn(
        async move {
            // The daemon owns what each service's form needs, so it is what
            // says whether these answers are enough.
            match api.check_server_draft(draft.clone()).await {
                Ok(check) => {
                    if let Some(problem) = check.problems.first() {
                        error.set(Some(components::forms::text(&problem.label)));
                        return;
                    }
                }
                Err(failure) => {
                    error.set(Some(failure.to_string()));
                    return;
                }
            }
            let saved = match api.upsert_server(draft).await {
                Ok(saved) => saved,
                Err(failure) => {
                    error.set(Some(failure.to_string()));
                    return;
                }
            };

            server_name.set(String::new());
            server_url.set(String::new());
            server_service.set(MusicService::Jellyfin);
            // Cleared with the rest of the form: it has been handed over, and
            // a credential left in a live signal is one the next server can
            // pick up.
            apple_music_manual_token.set(String::new());
            error.set(None);
            show_add_server.set(false);

            // A server is added to be used, so it becomes the active source
            // and picks up whichever sign-in it still needs.
            activate(api, saved.id, error, show_login, playback_error).await;
        }
        .instrument(tracing::info_span!("source.add_server")),
    );
}

/// Make a saved server the active source, and pick up the sign-in it needs if
/// the daemon reports it has no usable credentials.
pub fn switch_server(
    id: String,
    error: Signal<Option<String>>,
    show_login: Signal<bool>,
    playback_error: Signal<Option<String>>,
) {
    let api = hooks::consume_api();
    spawn(activate(api, id, error, show_login, playback_error));
}

pub fn delete_saved(id: String) {
    let api = hooks::consume_api();
    spawn(async move {
        if let Err(error) = api.delete_server(id).await {
            tracing::warn!(%error, "deleting a server failed");
            hooks::toast::toast_error(&error.to_string());
        }
    });
}

pub fn login_with_password(
    server_id: String,
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

    is_loading.set(true);
    login_error.set(None);
    let request = api::SourceLoginRequest {
        server_id,
        username: username(),
        password: password(),
    };
    let api = hooks::consume_api();
    spawn(async move {
        let result = api.login_source(request).await;
        is_loading.set(false);
        match result {
            Ok(_) => {
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
