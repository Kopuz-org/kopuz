#[cfg(not(target_os = "android"))]
use crate::theme_editor::ThemeEditorPage;
use ::server::provider::ProviderClient;

#[cfg(not(target_os = "android"))]
fn theme_editor_section(config: Signal<AppConfig>) -> Element {
    rsx! {
        section {
            h2 {
                class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                "{i18n::t(\"theme_editor\")}"
            }
            ThemeEditorPage { config, embedded: true }
        }
    }
}

#[cfg(target_os = "android")]
fn theme_editor_section(_config: Signal<AppConfig>) -> Element {
    rsx! {}
}

// Desktop-only: open the logs folder in the OS file manager, or export a
// bundle (latest.log + newest crash report) via a save dialog. rfd is
// excluded on Android and utils::logs is filesystem-only, so the rest get a
// stub.
// Declared `-> ()` so the panic coerces here; calling it from the onclick
// keeps the handler's return type `()` (a bare `panic!` in the closure infers
// `!`, which the event-handler trait rejects).
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn trigger_test_crash() {
    panic!("manual crash trigger from settings (debug build)");
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn logs_section(mut config: Signal<AppConfig>) -> Element {
    rsx! {
        section {
            h2 {
                class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                "{i18n::t(\"logs\")}"
            }
            div { class: "space-y-4",
                SettingItem {
                    title: i18n::t("enable_tracing").to_string(),
                    control: rsx! {
                        ToggleSetting {
                            enabled: config.read().tracing_enabled,
                            on_change: move |v| {
                                config.write().tracing_enabled = v;
                            },
                        }
                    },
                }
                p {
                    class: "text-xs text-amber-400/80 -mt-2",
                    "{i18n::t(\"tracing_warning\")}"
                }
            }
            div { class: "flex flex-wrap gap-3 mt-4",
                button {
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors flex items-center gap-2",
                    onclick: move |_| {
                        if let Err(e) = utils::logs::open_log_dir() {
                            tracing::warn!(error = %e, "failed to open logs folder");
                        }
                    },
                    i { class: "fa-solid fa-folder-open" }
                    "{i18n::t(\"open_logs_folder\")}"
                }
                button {
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors flex items-center gap-2",
                    onclick: move |_| {
                        spawn(async move {
                            if let Some(file) = rfd::AsyncFileDialog::new()
                                .set_file_name("kopuz-logs.txt")
                                .save_file()
                                .await
                            {
                                if let Err(e) = utils::logs::export_logs(file.path()) {
                                    tracing::warn!(error = %e, "failed to export logs");
                                }
                            }
                        });
                    },
                    i { class: "fa-solid fa-file-export" }
                    "{i18n::t(\"export_logs\")}"
                }
                // Debug builds only: deliberately panic to exercise the crash
                // hook / crash-report path. English-only by design (dev tool).
                if cfg!(debug_assertions) {
                    button {
                        class: "px-4 py-2 rounded-lg bg-red-500/20 hover:bg-red-500/30 text-red-300 text-sm transition-colors flex items-center gap-2",
                        onclick: move |_| trigger_test_crash(),
                        i { class: "fa-solid fa-bomb" }
                        "Trigger crash (debug)"
                    }
                }
            }
        }
    }
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn logs_section(_config: Signal<AppConfig>) -> Element {
    rsx! {}
}
use components::settings_items::{
    BackBehaviorSelector, ChannelModeSelector, DiscordPresencePausedSettings,
    DiscordPresenceSettings, EqualizerPanel, LanguageSelector, LastFmSettings,
    MultiDirectoryPicker, MusicBrainzSettings, RadioRegistryDropdown, ServerSettings, SettingItem,
    ThemeSelector, ToggleSetting,
};
use components::settings_popups::{AddRegistryPopup, AddServerPopup, LoginPopup};
use config::{AppConfig, ArtistPhotoSource, Browser, FetchStrategy, MusicService, OfflineQuality};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use tracing::Instrument;

async fn validate(cookies: &str) -> bool {
    ::server::ytmusic::YouTubeMusicClient::with_cookies(cookies.to_string())
        .validate_cookies()
        .await
        .is_ok()
}

async fn try_resume(seed: Option<String>) -> Option<String> {
    if let Some(c) = &seed
        && validate(c).await
    {
        return seed;
    }
    if let Some(c) = &seed
        && let Ok(Some(rotated)) =
            ::server::ytmusic::verify_session_keepalive::tick(c).await
        && validate(&rotated).await
    {
        return Some(rotated);
    }
    None
}

async fn ensure_signed_in(
    config_cookies: Option<String>,
    browser: Browser,
    server_id: &str,
) -> Result<String, String> {
    if let Some(c) = try_resume(config_cookies).await {
        return Ok(c);
    }

    let profile = ::server::ytmusic::isolated_profile::profile_dir(server_id);
    if profile.is_dir() {
        let from_profile = ::server::ytmusic::cookies::extract_from(browser, &profile)
            .await
            .ok();
        if let Some(c) = try_resume(from_profile).await {
            return Ok(c);
        }
    }

    let cookies = ::server::ytmusic::isolated_profile::launch_signin_and_extract(
        browser,
        server_id,
        std::time::Duration::from_secs(300),
    )
    .await?;
    if !validate(&cookies).await {
        return Err("Sign-in completed but YT validation still failed".to_string());
    }
    Ok(cookies)
}

#[component]
pub fn Settings(config: Signal<AppConfig>) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let crossfade_label = if config.read().crossfade_seconds == 0 {
        i18n::t("crossfade_off")
    } else {
        format!("{}s", config.read().crossfade_seconds)
    };
    let mut show_add_server = use_signal(|| false);
    let mut show_login = use_signal(|| false);

    let mut server_name = use_signal(|| String::new());
    let mut server_url = use_signal(|| String::new());
    let mut server_service = use_signal(|| MusicService::Jellyfin);
    let yt_browser = use_signal(|| {
        config
            .peek()
            .server
            .as_ref()
            .and_then(|s| s.yt_browser)
            .unwrap_or(config::Browser::Chrome)
    });
    // Anonymous YT mode for the add-server popup. Defaults to true on
    // Windows (browser sign-in disabled there) so the popup opens
    // pre-selected on the only working method.
    let yt_anonymous = use_signal(|| cfg!(target_os = "windows"));

    let mut username = use_signal(|| String::new());
    let mut password = use_signal(|| String::new());

    let mut error = use_signal(|| Option::<String>::None);
    let mut login_error = use_signal(|| Option::<String>::None);
    let mut is_loading = use_signal(|| false);

    let mut show_add_registry = use_signal(|| false);
    let mut registry_url = use_signal(|| String::new());
    let mut registry_error = use_signal(|| Option::<String>::None);
    let mut registry_loading = use_signal(|| false);
    let mut registry_toggle_error = use_signal(|| Option::<String>::None);

    let handle_add_registry = move |_| {
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

        spawn(async move {
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
                Err(e) => {
                    registry_error.set(Some(i18n::t_with(
                        "radio_registry_import_failed",
                        &[("error", e.to_string())],
                    )));
                }
            }
            registry_loading.set(false);
        }.instrument(tracing::info_span!("radio.import_registry")));
    };

    let ytmusic_auto_login = move || {
        // Prefer the browser already saved on the active server entry
        // (set during a previous successful sign-in); fall back to the
        // settings popup's selector for first-time setup.
        let (browser, existing, server_id) = {
            let cfg = config.peek();
            let srv = cfg.server.as_ref();
            (
                srv.and_then(|s| s.yt_browser).unwrap_or(*yt_browser.peek()),
                srv.and_then(|s| s.access_token.clone()).filter(|t| !t.is_empty()),
                srv.and_then(|s| s.id.clone()).unwrap_or_default(),
            )
        };
        let mut report = move |msg: String| {
            error.set(Some(msg.clone()));
            ctrl.playback_error.set(Some(msg));
        };
        spawn(async move {
            if let Err(e) =
                server::ytmusic::YouTubeMusicClient::new().check_botguard_available().await
            {
                report(format!(
                    "YouTube Music needs the rustypipe-botguard helper. {e}"
                ));
                return;
            }

            let cookies = match ensure_signed_in(existing, browser, &server_id).await {
                Ok(c) => c,
                Err(e) => {
                    report(format!("YT Music sign-in failed ({browser}): {e}"));
                    return;
                }
            };

            let yt_user_id =
                ::server::ytmusic::derive_user_id(&cookies).unwrap_or_else(|| "me".to_string());
            {
                let mut cfg = config.write();
                let saved_id = cfg.server.as_ref().and_then(|s| s.id.clone());
                if let Some(srv) = cfg.server.as_mut() {
                    srv.access_token = Some(cookies);
                    srv.user_id = Some(yt_user_id);
                    srv.yt_browser = Some(browser);
                }
                if let Some(id) = saved_id
                    && let Some(saved) = cfg.servers.iter_mut().find(|s| s.id == id)
                {
                    saved.yt_browser = Some(browser);
                }
            }
            error.set(None);
        });
    };

    let handle_add_server = move |_| {
        let selected_service = server_service();
        let is_ytmusic = selected_service == MusicService::YtMusic;

        if !is_ytmusic && !server_url().starts_with("http") {
            error.set(Some(i18n::t("invalid_server_url").to_string()));
            return;
        }

        // Snapshot the synchronous inputs so the async block doesn't have
        // to re-read signals (which it could, but this keeps the data
        // flow obvious).
        let name_input = server_name();
        let url_input = server_url();

        spawn(async move {
            // For YT Music we refuse to even create the server entry if
            // the botguard helper isn't installed — otherwise the user
            // gets a dead-on-arrival server they can't actually play
            // from. The probe is cheap (~ms) and happens before any
            // mutation of the config.
            if is_ytmusic {
                if let Err(msg) =
                    ::server::ytmusic::botguard::check_available().await
                {
                    error.set(Some(format!(
                        "Can't add YouTube Music server: {msg}"
                    )));
                    return;
                }
            }

            let display_name = if name_input.is_empty() {
                format!("Local {}", selected_service.display_name())
            } else {
                name_input
            };

            let effective_url = if is_ytmusic {
                "https://music.youtube.com".to_string()
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
                // Mark anonymous mode at the server level. Empty access
                // token + yt_anonymous=true is what get_stream /
                // discover etc. read as "no cookies, public surfaces
                // only".
                new_server.access_token = Some(String::new());
            }

            let saved = config::SavedServer {
                id: new_server.id.clone().unwrap_or_default(),
                name: new_server.name.clone(),
                url: new_server.url.clone(),
                service: new_server.service,
                yt_browser: (is_ytmusic && !is_anon).then(|| *yt_browser.peek()),
                yt_anonymous: is_anon,
            };
            {
                let mut cfg = config.write();
                cfg.add_saved_server(saved);
                cfg.server = Some(new_server);
            }

            server_name.set(String::new());
            server_url.set(String::new());
            server_service.set(MusicService::Jellyfin);
            error.set(None);
            show_add_server.set(false);

            if is_ytmusic && !is_anon {
                ytmusic_auto_login();
            } else if !is_ytmusic {
                show_login.set(true);
            }
            // Anonymous YT needs no further setup — the server entry
            // is already active and playable.
        }.instrument(tracing::info_span!("yt.anon_setup")));
    };

    let handle_switch_server = move |id: String| {
        let server = {
            let cfg = config.read();
            cfg.find_saved_server(&id).cloned()
        };
        if let Some(saved) = server {
            let is_ytmusic = saved.service == MusicService::YtMusic;
            let is_anon = is_ytmusic && saved.yt_anonymous;
            let active = config::MusicServer {
                name: saved.name,
                url: saved.url,
                service: saved.service,
                // Anonymous YT keeps an empty (non-None) token so the
                // backend treats it as anon rather than "needs sign-in".
                access_token: is_anon.then(String::new),
                user_id: None,
                id: Some(saved.id),
                // Carry the saved browser choice over so the sign-in
                // launch hits the binary the user picked, not whatever
                // the popup's default selector happens to be.
                yt_browser: saved.yt_browser,
                yt_anonymous: is_anon,
            };
            config.write().server = Some(active);
            if is_ytmusic && !is_anon {
                ytmusic_auto_login();
            } else if !is_ytmusic {
                show_login.set(true);
            }
            // Anonymous YT is immediately active — no sign-in launch.
        }
    };

    let handle_delete_saved = move |id: String| {
        let was_ytmusic = config
            .peek()
            .find_saved_server(&id)
            .map(|s| s.service == MusicService::YtMusic)
            .unwrap_or(false);
        config.write().remove_saved_server(&id);
        if was_ytmusic {
            let _ = ::server::ytmusic::isolated_profile::delete_profile(&id);
        }
    };

    let handle_login = move |_| {
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
                    Err(e) => {
                        login_error.set(Some(i18n::t_with(
                            "login_failed",
                            &[("error", e.to_string())],
                        )));
                    }
                }
            });
        }
    };

    rsx! {
        div { class: if cfg!(target_os = "android") { "px-4 pt-2 pb-28 w-full" } else { "p-8 w-full" },
            if !cfg!(target_os = "android") {
                h1 { class: "text-3xl font-bold text-white mb-6", "{i18n::t(\"settings\")}" }
            }

            div { class: "space-y-8",
                section {
                    h2 {
                        class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                        "{i18n::t(\"general\")}"
                    }

                    div { class: "space-y-4",
                        SettingItem {
                            title: i18n::t("language").to_string(),
                            control: rsx! {
                                LanguageSelector {
                                    current_language: config.read().language.clone(),
                                    on_change: move |lang: String| {
                                        config.write().language = lang.clone();
                                        i18n::set_locale(&lang);
                                    }
                                }
                            }
                        }

                        SettingItem {
                            title: i18n::t("appearance").to_string(),
                            control: rsx! {
                                ThemeSelector {
                                    current_theme: config.read().theme.clone(),
                                    on_change: move |theme| {
                                        config.write().theme = theme;
                                    }
                                }
                            }
                        }

                        if !cfg!(target_arch = "wasm32") {
                            SettingItem {
                                title: i18n::t("music_directory").to_string(),
                                    control: rsx! {
                                    MultiDirectoryPicker {
                                        current_paths: config.read().music_directory.clone(),
                                        on_add: move |path| {
                                            let mut config = config.write();
                                            if !config.music_directory.contains(&path) {
                                                config.music_directory.push(path);
                                            }
                                        },
                                        on_remove: move |index| {
                                            let mut config = config.write();
                                            if index < config.music_directory.len() {
                                                config.music_directory.remove(index);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        RadioRegistryDropdown {
                            registries: config.read().radio_registries.clone(),
                            error: registry_toggle_error,
                            on_toggle: move |index: usize| {
                                let (is_enabling, url) = {
                                    let cfg = config.read();
                                    let entry = cfg.radio_registries.get(index);
                                    (
                                        entry.map(|e| !e.enabled).unwrap_or(false),
                                        entry.map(|e| e.url.clone()).unwrap_or_default(),
                                    )
                                };

                                if is_enabling && !url.is_empty() {
                                    registry_toggle_error.set(None);
                                    spawn(async move {
                                        let mut temp_registry = radio::registry::StationRegistry::new();
                                        match temp_registry.import_registry(&url).await {
                                            Ok(_) => {
                                                let mut cfg = config.write();
                                                if let Some(entry) = cfg
                                                .radio_registries
                                                .iter_mut()
                                                .find(|entry| entry.url == url)
                                                {
                                                    entry.enabled = true;
                                                }
                                                registry_toggle_error.set(None);
                                            }
                                            Err(e) => {
                                                registry_toggle_error.set(Some(i18n::t_with("radio_registry_enable_failed", &[("error", e.to_string())])));
                                            }
                                        }
                                    });
                                } else {
                                    let mut cfg = config.write();
                                    if let Some(entry) = cfg.radio_registries.get_mut(index) {
                                        entry.enabled = false;
                                    }
                                    registry_toggle_error.set(None);
                                }
                            },
                            on_add: move |_| show_add_registry.set(true),
                            on_delete: move |index: usize| {
                                let mut cfg = config.write();
                                if index < cfg.radio_registries.len()
                                    && !cfg.radio_registries[index].is_default
                                {
                                    cfg.radio_registries.remove(index);
                                }
                            }
                        }

                        SettingItem {
                            title: i18n::t("media_servers").to_string(),
                            control: rsx! {
                                ServerSettings {
                                    active: config.read().server.clone(),
                                    servers: config.read().servers.clone(),
                                    on_add: move |_| show_add_server.set(true),
                                    on_delete: handle_delete_saved,
                                    on_switch: handle_switch_server,
                                    on_login: move |_| {
                                        let is_ytmusic = config
                                            .read()
                                            .server
                                            .as_ref()
                                            .map(|s| s.service == MusicService::YtMusic)
                                            .unwrap_or(false);
                                        if is_ytmusic {
                                            ytmusic_auto_login();
                                        } else {
                                            show_login.set(true);
                                        }
                                    },
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("reduce_animations").to_string(),
                            control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().reduce_animations,
                                    on_change: move |val| config.write().reduce_animations = val,
                                }
                            }
                        }
                        if !cfg!(target_arch = "wasm32") {
                            SettingItem {
                                title: i18n::t("auto_check_updates").to_string(),
                                control: rsx! {
                                    ToggleSetting {
                                        enabled: config.read().auto_check_updates,
                                        on_change: move |val| config.write().auto_check_updates = val,
                                    }
                                }
                            }
                        }
                        if !cfg!(target_arch = "wasm32") {
                            SettingItem {
                                title: i18n::t("show_source_toggle").to_string(),
                                    control: rsx! {
                                    ToggleSetting {
                                        enabled: config.read().show_source_toggle,
                                        on_change: move |val| config.write().show_source_toggle = val,
                                    }
                                }
                            }
                        }
                        if cfg!(any(target_os = "linux", target_os = "windows")) {
                            SettingItem {
                                title: i18n::t("titlebar_mode").to_string(),
                                control: rsx! {
                                    {
                                        let current_mode = config.read().titlebar_mode;
                                        rsx! {
                                            select {
                                                class: "bg-stone-800 text-white rounded-lg px-3 py-2 text-sm border border-white/10 focus:outline-none focus:border-indigo-500",
                                                onchange: move |evt| {
                                                    config.write().titlebar_mode = match evt.value().as_str() {
                                                        "system" => config::TitlebarMode::System,
                                                        "off" => config::TitlebarMode::Off,
                                                        _ => config::TitlebarMode::Custom,
                                                    };
                                                },
                                                option {
                                                    value: "custom",
                                                    selected: current_mode == config::TitlebarMode::Custom,
                                                    "{i18n::t(\"titlebar_custom\")}"
                                                }
                                                option {
                                                    value: "system",
                                                    selected: current_mode == config::TitlebarMode::System,
                                                    "{i18n::t(\"titlebar_system\")}"
                                                }
                                                option {
                                                    value: "off",
                                                    selected: current_mode == config::TitlebarMode::Off,
                                                    "{i18n::t(\"titlebar_off\")}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("ui_style").to_string(),
                            control: rsx! {
                                {
                                    let current_style = config.read().ui_style;
                                    rsx! {
                                        select {
                                            class: "bg-stone-800 text-white rounded-lg px-3 py-2 text-sm border border-white/10 focus:outline-none focus:border-indigo-500",
                                            onchange: move |evt| {
                                                config.write().ui_style = match evt.value().as_str() {
                                                    "modern" => config::UiStyle::Modern,
                                                    _ => config::UiStyle::Normal,
                                                };
                                            },
                                            option {
                                                value: "normal",
                                                selected: current_style == config::UiStyle::Normal,
                                                "{i18n::t(\"ui_normal\")}"
                                            }
                                            option {
                                                value: "modern",
                                                selected: current_style == config::UiStyle::Modern,
                                                "{i18n::t(\"ui_modern\")}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("player_bar_position").to_string(),
                            control: rsx! {
                                {
                                    let current_position = config.read().player_bar_position;
                                    rsx! {
                                        select {
                                            class: "bg-stone-800 text-white rounded-lg px-3 py-2 text-sm border border-white/10 focus:outline-none focus:border-indigo-500",
                                            onchange: move |evt| {
                                                config.write().player_bar_position = match evt.value().as_str() {
                                                    "top" => config::PlayerBarPosition::Top,
                                                    _ => config::PlayerBarPosition::Bottom,
                                                };
                                            },
                                            option {
                                                value: "bottom",
                                                selected: current_position == config::PlayerBarPosition::Bottom,
                                                "{i18n::t(\"position_bottom\")}"
                                            }
                                            option {
                                                value: "top",
                                                selected: current_position == config::PlayerBarPosition::Top,
                                                "{i18n::t(\"position_top\")}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("back_behavior").to_string(),
                            control: rsx! {
                                BackBehaviorSelector {
                                    current: config.read().back_behavior,
                                    on_change: move |val| config.write().back_behavior = val,
                                }
                            }
                        }
                        if !cfg!(target_arch = "wasm32") {
                            section {
                                h2 {
                                    class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                                    "{i18n::t(\"connectivity\")}"
                                }
                                div {
                                    class: "space-y-4",
                                    if !cfg!(target_os = "android") {
                                        SettingItem {
                                            title: i18n::t("discord_presence").to_string(),
                                            control: rsx! {
                                                DiscordPresenceSettings {
                                                    enabled: config.read().discord_presence.unwrap_or(true),
                                                    on_change: move |val| config.write().discord_presence = Some(val),
                                                }
                                            }
                                        }
                                        SettingItem {
                                            title: i18n::t("discord_presence_paused").to_string(),
                                            control: rsx! {
                                                DiscordPresencePausedSettings {
                                                    enabled: config.read().discord_presence_paused.unwrap_or(true),
                                                    on_change: move |val| config.write().discord_presence_paused = Some(val),
                                                }
                                            }
                                        }
                                    }
                                    SettingItem {
                                        title: i18n::t("listenbrainz").to_string(),
                                        control: rsx! {
                                            MusicBrainzSettings {
                                                current: config.read().musicbrainz_token.clone(),
                                                on_save: move |token: String| {
                                                    config.write().musicbrainz_token = token;
                                                },
                                            }
                                        }
                                    }
                                    SettingItem {
                                        title: i18n::t("lastfm").to_string(),
                                        control: rsx! {
                                            LastFmSettings {
                                                api_key: config.read().lastfm_api_key.clone(),
                                                api_secret: config.read().lastfm_api_secret.clone(),
                                                session_key: config.read().lastfm_session_key.clone(),

                                                on_api_key_save: move |value: String| {
                                                    config.write().lastfm_api_key = value;
                                                },

                                                on_api_secret_save: move |value: String| {
                                                    config.write().lastfm_api_secret = value;
                                                },

                                                on_session_key_save: move |value: String| {
                                                    config.write().lastfm_session_key = value;
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if config.read().server.is_some() {
                    section {
                        h2 {
                            class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                            "{i18n::t(\"offline_downloads\")}"
                        }
                        div { class: "space-y-4",
                            SettingItem {
                                title: i18n::t("download_quality").to_string(),
                                control: rsx! {
                                    select {
                                        class: "bg-stone-800 text-white rounded-lg px-3 py-2 text-sm border border-white/10 focus:outline-none focus:border-indigo-500",
                                        onchange: move |evt| {
                                            config.write().offline_quality = OfflineQuality::from_value_str(&evt.value());
                                        },
                                        for q in OfflineQuality::ALL {
                                            option {
                                                value: q.value_str(),
                                                selected: *q == config.read().offline_quality,
                                                "{q.label()}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                section {
                    h2 {
                        class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                        "{i18n::t(\"metadata\")}"
                    }
                    div { class: "space-y-4",
                        SettingItem {
                            title: i18n::t("auto_fetch_covers").to_string(),
                            control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().auto_fetch_covers,
                                    on_change: move |val| config.write().auto_fetch_covers = val,
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("prefer_local_lyrics").to_string(),
                            control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().prefer_local_lyrics,
                                    on_change: move |val| config.write().prefer_local_lyrics = val,
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("cover_fetch_strategy").to_string(),
                            control: rsx! {
                                {
                                    let current = config.read().cover_fetch_strategy;
                                    rsx! {
                                        select {
                                            class: "bg-stone-800 text-white rounded-lg px-3 py-2 text-sm border border-white/10 focus:outline-none focus:border-indigo-500",
                                            onchange: move |evt| {
                                                config.write().cover_fetch_strategy = match evt.value().as_str() {
                                                    "lastfm_first" => FetchStrategy::LastFmFirst,
                                                    "musicbrainz_only" => FetchStrategy::MusicBrainzOnly,
                                                    "lastfm_only" => FetchStrategy::LastFmOnly,
                                                    _ => FetchStrategy::MusicBrainzFirst,
                                                };
                                            },
                                            option {
                                                value: "musicbrainz_first",
                                                selected: current == FetchStrategy::MusicBrainzFirst,
                                                "{i18n::t(\"musicbrainz_first\")}"
                                            }
                                            option {
                                                value: "lastfm_first",
                                                selected: current == FetchStrategy::LastFmFirst,
                                                "{i18n::t(\"lastfm_first\")}"
                                            }
                                            option {
                                                value: "musicbrainz_only",
                                                selected: current == FetchStrategy::MusicBrainzOnly,
                                                "{i18n::t(\"musicbrainz_only\")}"
                                            }
                                            option {
                                                value: "lastfm_only",
                                                selected: current == FetchStrategy::LastFmOnly,
                                                "{i18n::t(\"lastfm_only\")}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("artist_photo_source").to_string(),
                            control: rsx! {
                                {
                                    let current = config.read().artist_photo_source;
                                    rsx! {
                                        select {
                                            class: "bg-stone-800 text-white rounded-lg px-3 py-2 text-sm border border-white/10 focus:outline-none focus:border-indigo-500",
                                            onchange: move |evt| {
                                                config.write().artist_photo_source = match evt.value().as_str() {
                                                    "artist_photo" => ArtistPhotoSource::ArtistPhoto,
                                                    _ => ArtistPhotoSource::AlbumCover,
                                                };
                                            },
                                            option {
                                                value: "album_cover",
                                                selected: current == ArtistPhotoSource::AlbumCover,
                                                "{i18n::t(\"album_cover\")}"
                                            }
                                            option {
                                                value: "artist_photo",
                                                selected: current == ArtistPhotoSource::ArtistPhoto,
                                                "{i18n::t(\"artist_photo\")}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                section {
                    h2 {
                        class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                        "{i18n::t(\"player_settings\")}"
                    }

                    div { class: "space-y-4",
                        SettingItem {
                            title: i18n::t("crossfade").to_string(),
                            control: rsx! {
                                div { class: "flex items-center gap-3 min-w-[220px]",
                                    input {
                                        r#type: "range",
                                        min: "0",
                                        max: "12",
                                        step: "1",
                                        value: format!("{}", config.read().crossfade_seconds),
                                        class: "w-40",
                                        style: "accent-color: var(--color-indigo-500);",
                                        oninput: move |evt| {
                                            if let Ok(value) = evt.value().parse::<u8>() {
                                                config.write().crossfade_seconds = value.min(12);
                                            }
                                        }
                                    }
                                    span {
                                        class: "text-xs font-mono text-white/80 w-16 text-right",
                                        "{crossfade_label}"
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("volume_scroll_step").to_string(),
                            control: rsx! {
                                div { class: "flex items-center gap-3 min-w-[220px]",
                                    input {
                                        r#type: "range",
                                        min: "1",
                                        max: "50",
                                        step: "1",
                                        value: format!("{}", (config.read().volume_scroll_step * 100.0).round() as i32),
                                        class: "w-40",
                                        style: "accent-color: var(--color-indigo-500);",
                                        oninput: move |evt| {
                                            if let Ok(pct) = evt.value().parse::<i32>() {
                                                let clamped = pct.clamp(1, 50);
                                                config.write().volume_scroll_step = clamped as f32 / 100.0;
                                            }
                                        }
                                    }
                                    span {
                                        class: "text-xs font-mono text-white/80 w-16 text-right",
                                        "{(config.read().volume_scroll_step * 100.0).round() as i32}%"
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("channel_mode").to_string(),
                            control: rsx! {
                                ChannelModeSelector {
                                    current: config.read().channel_mode,
                                    on_change: move |mode| {
                                        config.write().channel_mode = mode;
                                        ctrl.player.write().set_channel_mode(mode);
                                    }
                                }
                            }
                        }
                        div { class: "py-2",
                            p { class: "text-white font-medium mb-3", "{i18n::t(\"equalizer\")}" }
                            EqualizerPanel {
                                current: config.read().equalizer.clone(),
                                on_preview: move |equalizer: config::EqualizerSettings| {
                                    ctrl.player.write().set_equalizer(equalizer);
                                },
                                on_commit: move |equalizer: config::EqualizerSettings| {
                                    config.write().equalizer = equalizer.clone();
                                    ctrl.player.write().set_equalizer(equalizer);
                                }
                            }
                        }
                    }
                }

                {logs_section(config)}

                {theme_editor_section(config)}



                if show_add_server() {
                    AddServerPopup {
                        server_name,
                        server_url,
                        server_service,
                        yt_browser,
                        yt_anonymous,
                        error,
                        on_close: move |_| show_add_server.set(false),
                        on_save: handle_add_server
                    }
                }

                if show_add_registry() {
                    AddRegistryPopup {
                        registry_url,
                        error: registry_error,
                        loading: registry_loading,
                        on_close: move |_| show_add_registry.set(false),
                        on_save: handle_add_registry
                    }
                }

                if show_login() {
                    LoginPopup {
                        username,
                        password,
                        service_name: config
                            .read()
                            .server
                            .as_ref()
                            .map(|server| server.service.display_name().to_string())
                            .unwrap_or_else(|| i18n::t("server").to_string()),
                        error: login_error,
                        loading: is_loading,
                        on_close: move |_| {
                            show_login.set(false);
                            username.set(String::new());
                            password.set(String::new());
                            login_error.set(None);
                        },
                        on_save: handle_login
                    }
                }
            }
        }
    }
}
