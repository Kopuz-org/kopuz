use crate::theme_editor::ThemeEditorPage;
use ::server::provider::ProviderClient;
use components::settings_items::{
    BackBehaviorSelector, ChannelModeSelector, DiscordPresenceSettings, EqualizerPanel,
    LanguageSelector, LastFmSettings, MultiDirectoryPicker, MusicBrainzSettings, ServerSettings,
    SettingItem, ThemeSelector, ToggleSetting, RadioRegistryDropdown,
};
use components::settings_popups::{AddServerPopup, LoginPopup, AddRegistryPopup};
use config::{AppConfig, ArtistPhotoSource, MusicService, OfflineQuality};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

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
                    registry_error.set(Some(i18n::t_with("radio_registry_import_failed", &[("error", e.to_string())])));
                }
            }
            registry_loading.set(false);
        });
    };

    let handle_add_server = move |_| {
        if !server_url().starts_with("http") {
            error.set(Some(i18n::t("invalid_server_url").to_string()));
            return;
        }

        let selected_service = server_service();

        let new_server = config::MusicServer::new_with_service(
            if server_name().is_empty() {
                format!("Local {}", selected_service.display_name())
            } else {
                server_name()
            },
            server_url(),
            selected_service,
        );

        config.write().server = Some(new_server);

        server_name.set(String::new());
        server_url.set(String::new());
        server_service.set(MusicService::Jellyfin);
        error.set(None);
        show_add_server.set(false);

        show_login.set(true);
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
        div { class: "p-8 max-w-4xl",
            h1 { class: "text-3xl font-bold text-white mb-6", "{i18n::t(\"settings\")}" }

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
                            title: i18n::t("media_server").to_string(),
                            control: rsx! {
                                ServerSettings {
                                    server: config.read().server.clone(),
                                    on_add: move |_| show_add_server.set(true),
                                    on_delete: move |_| config.write().server = None,
                                    on_login: move |_| show_login.set(true),
                                }
                            }
                        }
                        if !cfg!(target_arch = "wasm32") {
                            SettingItem {
                                title: i18n::t("discord_presence").to_string(),
                                    control: rsx! {
                                    DiscordPresenceSettings {
                                        enabled: config.read().discord_presence.unwrap_or(true),
                                        on_change: move |val| config.write().discord_presence = Some(val),
                                    }
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

                section {
                    h2 {
                        class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                        "{i18n::t(\"theme_editor\")}"
                    }
                    ThemeEditorPage { config, embedded: true }
                }



                if show_add_server() {
                    AddServerPopup {
                        server_name,
                        server_url,
                        server_service,
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
