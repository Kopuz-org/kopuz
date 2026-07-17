#[cfg(not(target_os = "android"))]
use crate::theme_editor::ThemeEditorPage;
#[cfg(not(target_os = "android"))]
fn theme_editor_section(config: Signal<AppConfig>) -> Element {
    rsx! {
        SettingsSection { title: i18n::t("theme_editor").to_string(),
            div { class: "py-2",
                ThemeEditorPage { config, embedded: true }
            }
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
#[cfg(not(target_os = "android"))]
fn trigger_test_crash() {
    panic!("manual crash trigger from settings (debug build)");
}

#[cfg(not(target_os = "android"))]
fn logs_section(mut config: Signal<AppConfig>) -> Element {
    rsx! {
        SettingsSection { title: i18n::t("logs").to_string(),
            div {
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
                    class: "text-xs text-amber-400/80 pb-3",
                    "{i18n::t(\"tracing_warning\")}"
                }
            }
            div { class: "flex flex-wrap gap-3 py-3",
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
                                && let Err(e) = utils::logs::export_logs(file.path()) {
                                    tracing::warn!(error = %e, "failed to export logs");
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

/// Debug-build-only database panel: reset / load release DB / seed / re-run
/// import / vacuum / info, all against the disposable debug DB with a live
/// pool swap (no restart). English-only by design (dev tool).
#[cfg(target_os = "android")]
fn logs_section(_config: Signal<AppConfig>) -> Element {
    rsx! {}
}

async fn clear_cover_cache() {
    let cover_cache = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.cache_dir().join("covers"))
        .unwrap_or_else(|| std::path::PathBuf::from("./cache/covers"));

    if let Ok(mut entries) = tokio::fs::read_dir(&cover_cache).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }

    let tmp = std::env::temp_dir();
    if let Ok(mut entries) = tokio::fs::read_dir(&tmp).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(name) = entry.file_name().to_str()
                && (name.starts_with("rusic_thumb_") || name.starts_with("rusic_hq_"))
            {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}

#[component]
fn ClearCacheButton() -> Element {
    let mut cleared = use_signal(|| false);
    let mut reextracting = use_signal(|| false);
    let mut trigger_reextract = use_context::<hooks::CoverReextractTrigger>().0;
    rsx! {
        div { class: "flex flex-wrap gap-3 mt-4 items-center",
            button {
                class: "px-4 py-2 rounded-lg bg-red-500/20 hover:bg-red-500/30 text-red-300 text-sm transition-colors flex items-center gap-2",
                onclick: move |_| {
                    reextracting.set(false);
                    spawn(async move {
                        clear_cover_cache().await;
                        cleared.set(true);
                    });
                },
                i { class: "fa-solid fa-trash" }
                "{i18n::t(\"clear_cover_cache\")}"
            }
            button {
                class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors flex items-center gap-2",
                onclick: move |_| {
                    cleared.set(false);
                    *trigger_reextract.write() += 1;
                    reextracting.set(true);
                },
                i { class: "fa-solid fa-arrows-rotate" }
                "{i18n::t(\"force_rescan_photos\")}"
            }
            if cleared() {
                span { class: "text-xs text-emerald-400/80", "{i18n::t(\"cache_cleared\")}" }
            }
            if reextracting() {
                span { class: "text-xs text-emerald-400/80", "{i18n::t(\"rescanning_photos\")}" }
            }
        }
    }
}

use components::settings_items::{
    BackBehaviorSelector, ChannelModeSelector, DeviceChangeBehaviorSelector,
    DiscordPresencePausedSettings, DiscordPresenceSettings, EqualizerPanel, LanguageSelector,
    LastFmSettings, LibreFmSettings, MultiDirectoryPicker, MusicBrainzSettings,
    RadioRegistryDropdown, SampleRateModeSelector, ServerSettings, SettingItem, SettingsSection,
    ThemeSelector, ToggleSetting,
};
use components::settings_popups::{AddRegistryPopup, AddServerPopup, LoginPopup};
use config::{AppConfig, FetchStrategy, MusicService, OfflineQuality};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

#[component]
pub fn Settings(config: Signal<AppConfig>) -> Element {
    let ctrl = use_context::<PlayerController>();
    let crossfade_label = if config.read().crossfade_seconds == 0 {
        i18n::t("crossfade_off")
    } else {
        format!("{}s", config.read().crossfade_seconds)
    };
    let mut show_add_server = use_signal(|| false);
    let mut show_login = use_signal(|| false);

    let server_name = use_signal(String::new);
    let server_url = use_signal(String::new);
    let server_service = use_signal(|| MusicService::Jellyfin);
    let yt_browser = use_signal(|| {
        config
            .peek()
            .server
            .as_ref()
            .and_then(|s| s.yt_browser)
            .unwrap_or(config::Browser::Chrome)
    });
    // Anonymous YT mode for the add-server popup. Defaults to browser sign-in on
    // every platform (Windows cookie decryption is now supported natively).
    let yt_anonymous = use_signal(|| false);

    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);

    let error = use_signal(|| Option::<String>::None);
    let mut login_error = use_signal(|| Option::<String>::None);
    let is_loading = use_signal(|| false);

    let mut show_add_registry = use_signal(|| false);
    let registry_url = use_signal(String::new);
    let registry_error = use_signal(|| Option::<String>::None);
    let registry_loading = use_signal(|| false);
    let mut registry_toggle_error = use_signal(|| Option::<String>::None);

    let handle_add_registry = move |_| {
        crate::settings_actions::add_registry(
            config,
            registry_url,
            registry_error,
            registry_loading,
            show_add_registry,
        );
    };

    let ytmusic_auto_login = move || {
        crate::settings_actions::ytmusic_auto_login(config, yt_browser, error, ctrl.playback_error);
    };

    let handle_add_server = move |_| {
        crate::settings_actions::add_server(
            config,
            server_name,
            server_url,
            server_service,
            yt_browser,
            yt_anonymous,
            error,
            show_add_server,
            show_login,
            ctrl.playback_error,
        );
    };

    let db_for_switch = use_context::<hooks::ReadDb>();
    let handle_switch_server = move |id: String| {
        crate::settings_actions::switch_server(
            config,
            db_for_switch.clone(),
            id,
            yt_browser,
            error,
            show_login,
            ctrl.playback_error,
        );
    };

    let handle_delete_saved = move |id: String| {
        crate::settings_actions::delete_saved(config, id);
    };

    let handle_login = move |_| {
        crate::settings_actions::login_with_password(
            config,
            username,
            password,
            login_error,
            is_loading,
            show_login,
        );
    };

    rsx! {
        div { class: if cfg!(target_os = "android") { "px-4 pt-2 pb-28 w-full" } else { "p-8 w-full" },
            if !cfg!(target_os = "android") {
                h1 { class: "text-3xl font-bold text-white mb-6", "{i18n::t(\"settings\")}" }
            }

            div { class: "space-y-12",
                SettingsSection {
                    title: i18n::t("general").to_string(),
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

                        div { id: "settings-media-servers",
                            SettingItem {
                                title: i18n::t("media_servers").to_string(),
                                control: rsx! {
                                    ServerSettings {
                                        active: config.read().server.clone(),
                                        active_source_id: config
                                            .read()
                                            .active_source
                                            .server_id()
                                            .map(String::from),
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
                        SettingItem {
                            title: i18n::t("auto_check_updates").to_string(),
                            control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().auto_check_updates,
                                    on_change: move |val| config.write().auto_check_updates = val,
                                }
                            }
                        }
                        if cfg!(not(target_os = "android")) {
                            SettingItem {
                                title: i18n::t("minimize_to_tray").to_string(),
                                control: rsx! {
                                    ToggleSetting {
                                        enabled: config.read().minimize_to_tray,
                                        on_change: move |val| config.write().minimize_to_tray = val,
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("show_source_toggle").to_string(),
                                control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().show_source_toggle,
                                    on_change: move |val| config.write().show_source_toggle = val,
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
                                                    "vaxry" => config::UiStyle::Vaxry,
                                                    _ => config::UiStyle::Normal,
                                                };
                                            },
                                            option {
                                                value: "normal",
                                                selected: current_style == config::UiStyle::Normal,
                                                "{i18n::t(\"ui_normal\")}"
                                            }
                                            option {
                                                value: "vaxry",
                                                selected: current_style == config::UiStyle::Vaxry,
                                                "{i18n::t(\"ui_vaxry\")}"
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
                }
                SettingsSection {
                    title: i18n::t("connectivity").to_string(),
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
                                    SettingItem {
                                        title: i18n::t("discord_presence_source").to_string(),
                                        control: rsx! {
                                            ToggleSetting {
                                                enabled: config.read().discord_presence_source.unwrap_or(true),
                                                on_change: move |val| config.write().discord_presence_source = Some(val),
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
                                SettingItem {
                                    title: i18n::t("librefm").to_string(),
                                    control: rsx! {
                                        LibreFmSettings {
                                            session_key: config.read().librefm_session_key.clone(),

                                            on_session_key_save: move |value: String| {
                                                config.write().librefm_session_key = value;
                                            },
                                        }
                                    }
                                }
                }

                if config.read().server.is_some() {
                    SettingsSection {
                        title: i18n::t("offline_downloads").to_string(),
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

                SettingsSection {
                    title: i18n::t("metadata").to_string(),
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
                            title: i18n::t("enable_musixmatch_lyrics").to_string(),
                            control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().enable_musixmatch_lyrics,
                                    on_change: move |val| config.write().enable_musixmatch_lyrics = val,
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
                    ClearCacheButton {}
                }

                SettingsSection {
                    title: i18n::t("player_settings").to_string(),
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
                                        ctrl.player.peek().set_channel_mode(mode);
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("device_change_behavior").to_string(),
                            control: rsx! {
                                DeviceChangeBehaviorSelector {
                                    current: config.read().device_change_behavior,
                                    on_change: move |behavior| {
                                        config.write().device_change_behavior = behavior;
                                        ctrl.player.peek().set_device_change_behavior(behavior);
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("sample_rate_mode").to_string(),
                            control: rsx! {
                                SampleRateModeSelector {
                                    current: config.read().sample_rate_mode,
                                    on_change: move |mode| {
                                        config.write().sample_rate_mode = mode;
                                        ctrl.player.peek().set_sample_rate_mode(mode);
                                    }
                                }
                            }
                        }
                        div { class: "py-2",
                            p { class: "text-white font-medium mb-3", "{i18n::t(\"equalizer\")}" }
                            EqualizerPanel {
                                current: config.read().equalizer.clone(),
                                on_preview: move |equalizer: config::EqualizerSettings| {
                                    ctrl.player.peek().set_equalizer(equalizer);
                                },
                                on_commit: move |equalizer: config::EqualizerSettings| {
                                    config.write().equalizer = equalizer.clone();
                                    ctrl.player.peek().set_equalizer(equalizer);
                                }
                            }
                        }
                }

                {logs_section(config)}

                {hooks::debug_db_section()}

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
