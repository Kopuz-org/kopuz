mod desktop_tools;
mod navigation;
mod sections;

use desktop_tools::{logs_section, theme_editor_section};
use navigation::{SettingsCategory, SettingsNavigation};
use sections::{ConnectivitySection, DownloadsSection, MetadataSection, PlayerSection};

use components::settings_items::{
    AppSelect, BackBehaviorSelector, LanguageSelector, LocalSourceSettings, RadioRegistryDropdown,
    ServerSettings, SettingItem, SettingsSection, ThemeSelector, ToggleSetting,
};
use components::settings_popups::{
    AddLocalSourcePopup, AddRegistryPopup, AddServerPopup, LoginPopup,
};
use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

#[component]
fn BuildInfoCard() -> Element {
    let build_summary = utils::build_info::summary();
    let copy_value = serde_json::to_string(&build_summary).unwrap_or_else(|error| {
        tracing::warn!(%error, "failed to encode build information for clipboard");
        "\"\"".to_string()
    });

    rsx! {
        aside { class: "mt-4 rounded-xl border border-white/10 bg-black/25 px-5 py-3 flex items-center justify-between gap-5",
            div { class: "min-w-0 flex flex-col gap-1",
                span { class: "text-sm font-medium text-white/80", "Kopuz {utils::build_info::VERSION}" }
                code { class: "text-xs text-white/45 break-all", "{utils::build_info::COMMIT}" }
            }
            button {
                r#type: "button",
                class: "p-2 rounded text-white/35 hover:text-white hover:bg-white/10 transition-colors shrink-0",
                title: "{build_summary}",
                aria_label: "{build_summary}",
                onclick: move |_| {
                    let js = format!(
                        "navigator.clipboard.writeText({copy_value}).catch((e) => console.error('clipboard writeText failed', e));"
                    );
                    let _ = dioxus::document::eval(&js);
                },
                i { class: "fa-solid fa-copy" }
            }
        }
    }
}

#[component]
pub fn Settings(config: Signal<AppConfig>) -> Element {
    let ctrl = use_context::<PlayerController>();
    let spotify_browsers = use_hook(|| {
        ::server::spotify::host::available_browsers()
            .into_iter()
            .map(|b| (b.id.to_string(), b.label.to_string()))
            .collect::<Vec<_>>()
    });
    let mut show_add_server = use_signal(|| false);
    let mut show_add_local_source = use_signal(|| false);
    let mut show_login = use_signal(|| false);

    let mut local_source_name = use_signal(String::new);
    let mut local_source_directories = use_signal(Vec::<std::path::PathBuf>::new);
    let mut local_source_error = use_signal(|| Option::<String>::None);

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
    let yt_anonymous = use_signal(|| false);

    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);

    let error = use_signal(|| Option::<String>::None);
    let mut login_error = use_signal(|| Option::<String>::None);
    let is_loading = use_signal(|| false);
    let mut active_category = use_signal(|| SettingsCategory::General);
    let settings_anchor = try_consume_context::<components::source_switcher::SettingsAnchor>();

    use_effect(move || {
        if settings_anchor.is_some_and(|components::source_switcher::SettingsAnchor(anchor)| {
            anchor.read().as_deref() == Some("settings-media-servers")
        }) {
            active_category.set(SettingsCategory::Library);
        }
    });

    let mut show_add_registry = use_signal(|| false);
    let registry_url = use_signal(String::new);
    let registry_error = use_signal(|| Option::<String>::None);
    let registry_loading = use_signal(|| false);
    let mut registry_toggle_error = use_signal(|| Option::<String>::None);

    let host_access = use_signal(|| false);

    use_effect(move || {
        spawn(async move {
            crate::settings_actions::ensure_host_access(host_access).await;
        });
    });

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
    let db_for_local_switch = db_for_switch.clone();
    let handle_switch_local = move |source: config::Source| {
        let db = db_for_local_switch.clone();
        spawn(async move {
            hooks::source_switch::apply_source_switch(config, db, source).await;
        });
    };
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
        div { class: if cfg!(target_os = "android") { "px-3 pt-2 pb-6 w-full max-w-7xl mx-auto" } else if config.read().settings_layout == config::SettingsLayout::TopBar { "settings-page settings-layout-topbar px-6 py-7 w-full max-w-7xl mx-auto" } else { "settings-page settings-layout-cd px-6 py-7 w-full max-w-7xl mx-auto" },
            if !cfg!(target_os = "android") {
                h1 { class: "text-2xl font-semibold tracking-tight text-white mb-5 px-1", "{i18n::t(\"settings\")}" }
            }

            if try_consume_context::<config::store::FileLayers>().is_some_and(|layers| !layers.locked_keys.is_empty()) {
                aside { class: "mb-4 rounded-xl border border-white/10 bg-white/5 px-5 py-3 flex items-center gap-3",
                    i { class: "fa-solid fa-lock text-white/40 text-sm shrink-0" }
                    p { class: "text-sm text-white/70", "{i18n::t(\"settings_managed_notice\")}" }
                }
            }

            div { class: "settings-workspace",
                SettingsNavigation {
                    selected: active_category(),
                    on_select: move |category| {
                        active_category.set(category);
                        let _ = document::eval(
                            "requestAnimationFrame(() => document.getElementById('settings-category-content')?.scrollIntoView({ block: 'start' }))"
                        );
                    },
                }
                main { id: "settings-category-content", class: "settings-category-content",
                if matches!(active_category(), SettingsCategory::General | SettingsCategory::Customization | SettingsCategory::Library) {
                    SettingsSection {
                    title: match active_category() {
                        SettingsCategory::Customization => i18n::t("appearance").to_string(),
                        SettingsCategory::Library => i18n::t("library").to_string(),
                        _ => i18n::t("general").to_string(),
                    },
                    if active_category() == SettingsCategory::Customization {
                        SettingItem {
                            title: i18n::t("language").to_string(),
                            config_key: "language",
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
                    }

                    if active_category() == SettingsCategory::Customization {
                        SettingItem {
                            title: i18n::t("appearance").to_string(),
                            config_key: "theme",
                            control: rsx! {
                                ThemeSelector {
                                    current_theme: config.read().theme.clone(),
                                    on_change: move |theme| {
                                        config.write().theme = theme;
                                    }
                                }
                            }
                        }

                        if cfg!(not(target_os = "android"))
                            && config.read().theme == utils::live_theme::THEME_ID
                        {
                            SettingItem {
                                title: i18n::t("live_theme_file").to_string(),
                                config_key: "live_theme_path",
                                control: rsx! {
                                    div { class: "flex items-center gap-2",
                                        span {
                                            class: "text-xs text-white/50 font-mono max-w-[220px] truncate",
                                            "{utils::live_theme::resolve_path(&config.read().live_theme_path).display()}"
                                        }
                                        if !config.read().live_theme_path.is_empty() {
                                            button {
                                                class: "px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-red-300 text-sm transition-colors",
                                                onclick: move |_| config.write().live_theme_path = String::new(),
                                                "{i18n::t(\"remove\")}"
                                            }
                                        }
                                        button {
                                            class: "px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                                            onclick: move |_| {
                                                #[cfg(not(target_os = "android"))]
                                                spawn(async move {
                                                    if let Some(file) = rfd::AsyncFileDialog::new()
                                                        .add_filter("JSON", &["json"])
                                                        .pick_file()
                                                        .await
                                                    {
                                                        config.write().live_theme_path =
                                                            file.path().display().to_string();
                                                    }
                                                });
                                            },
                                            "{i18n::t(\"choose_palette\")}"
                                        }
                                    }
                                }
                            }
                        }

                        SettingItem {
                            title: i18n::t("cover_art_background").to_string(),
                            config_key: "cover_art_background",
                            control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().cover_art_background,
                                    on_change: move |val| config.write().cover_art_background = val,
                                }
                            }
                        }
                        if cfg!(not(target_os = "android")) {
                            SettingItem {
                                title: i18n::t("custom_background").to_string(),
                                config_key: "custom_background_path",
                                control: rsx! {
                                    div { class: "flex items-center gap-2",
                                        if !config.read().custom_background_path.is_empty() {
                                            span {
                                                class: "text-xs text-white/50 font-mono max-w-[220px] truncate",
                                                "{config.read().custom_background_path}"
                                            }
                                            button {
                                                class: "px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-red-300 text-sm transition-colors",
                                                onclick: move |_| config.write().custom_background_path = String::new(),
                                                "{i18n::t(\"remove\")}"
                                            }
                                        }
                                        button {
                                            class: "px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                                            onclick: move |_| {
                                                #[cfg(not(target_os = "android"))]
                                                spawn(async move {
                                                    if let Some(file) = rfd::AsyncFileDialog::new()
                                                        .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif", "bmp"])
                                                        .pick_file()
                                                        .await
                                                    {
                                                        config.write().custom_background_path =
                                                            file.path().display().to_string();
                                                    }
                                                });
                                            },
                                            "{i18n::t(\"choose_image\")}"
                                        }
                                    }
                                }
                            }
                        }
                        if cfg!(not(target_os = "android")) {
                            SettingItem {
                                title: i18n::t("custom_font").to_string(),
                                config_key: "custom_font_path",
                                control: rsx! {
                                    div { class: "flex items-center gap-2",
                                        if !config.read().custom_font_path.is_empty() {
                                            span {
                                                class: "text-xs text-white/50 font-mono max-w-[220px] truncate",
                                                "{config.read().custom_font_path}"
                                            }
                                            button {
                                                class: "px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-red-300 text-sm transition-colors",
                                                onclick: move |_| config.write().custom_font_path = String::new(),
                                                "{i18n::t(\"remove\")}"
                                            }
                                        }
                                        button {
                                            class: "px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                                            onclick: move |_| {
                                                #[cfg(not(target_os = "android"))]
                                                spawn(async move {
                                                    if let Some(file) = rfd::AsyncFileDialog::new()
                                                        .add_filter("Fonts", &["ttf", "otf", "woff", "woff2"])
                                                        .pick_file()
                                                        .await
                                                    {
                                                        config.write().custom_font_path =
                                                            file.path().display().to_string();
                                                    }
                                                });
                                            },
                                            "{i18n::t(\"choose_font\")}"
                                        }
                                    }
                                }
                            }
                        }
                        if config.read().cover_art_background
                            || !config.read().custom_background_path.is_empty()
                        {
                                SettingItem {
                                    title: i18n::t("cover_art_darkening").to_string(),
                                    config_key: "cover_art_darkening",
                                    control: rsx! {
                                        div { class: "flex items-center gap-3 min-w-[220px]",
                                            input {
                                                r#type: "range",
                                                min: "0",
                                                max: "95",
                                                step: "5",
                                                value: format!("{}", config.read().cover_art_darkening),
                                                class: "w-40",
                                                style: "accent-color: var(--color-indigo-500);",
                                                oninput: move |evt| {
                                                    if let Ok(value) = evt.value().parse::<u8>() {
                                                        config.write().cover_art_darkening = value.min(95);
                                                    }
                                                }
                                            }
                                            span {
                                                class: "text-xs font-mono text-white/80 w-16 text-right",
                                                "{config.read().cover_art_darkening}%"
                                            }
                                        }
                                    }
                                }
                                SettingItem {
                                    title: i18n::t("cover_art_blur").to_string(),
                                    config_key: "cover_art_blur",
                                    control: rsx! {
                                        div { class: "flex items-center gap-3 min-w-[220px]",
                                            input {
                                                r#type: "range",
                                                min: "0",
                                                max: "100",
                                                step: "5",
                                                value: format!("{}", config.read().cover_art_blur),
                                                class: "w-40",
                                                style: "accent-color: var(--color-indigo-500);",
                                                oninput: move |evt| {
                                                    if let Ok(value) = evt.value().parse::<u8>() {
                                                        config.write().cover_art_blur = value.min(100);
                                                    }
                                                }
                                            }
                                            span {
                                                class: "text-xs font-mono text-white/80 w-16 text-right",
                                                "{config.read().cover_art_blur}px"
                                            }
                                        }
                                    }
                                }
                        }
                    }

                    if active_category() == SettingsCategory::Library {
                        SettingItem {
                            title: i18n::t("local_libraries").to_string(),
                            config_key: "music_directory",
                            extra_config_keys: vec!["local_sources"],
                            control: rsx! {
                                LocalSourceSettings {
                                    active_source: config.read().active_source.clone(),
                                    default_directories: config.read().music_directory.clone(),
                                    sources: config.read().local_sources.clone(),
                                    on_add: move |_| show_add_local_source.set(true),
                                    on_delete: move |id: String| config.write().remove_local_source(&id),
                                    on_switch: handle_switch_local,
                                    on_add_folder: move |(source, path): (config::Source, std::path::PathBuf)| {
                                        let mut cfg = config.write();
                                        match source {
                                            config::Source::Local => {
                                                if !cfg.music_directory.contains(&path) {
                                                    cfg.music_directory.push(path);
                                                }
                                            }
                                            config::Source::LocalLibrary(id) => {
                                                if let Some(local) = cfg.local_sources.iter_mut().find(|local| local.id == id)
                                                    && !local.directories.contains(&path)
                                                {
                                                    local.directories.push(path);
                                                }
                                            }
                                            config::Source::Server(_) => {}
                                        }
                                    },
                                    on_remove_folder: move |(source, index): (config::Source, usize)| {
                                        let mut cfg = config.write();
                                        match source {
                                            config::Source::Local => {
                                                if index < cfg.music_directory.len() {
                                                    cfg.music_directory.remove(index);
                                                }
                                            }
                                            config::Source::LocalLibrary(id) => {
                                                if let Some(local) = cfg.local_sources.iter_mut().find(|local| local.id == id)
                                                    && index < local.directories.len()
                                                {
                                                    local.directories.remove(index);
                                                }
                                            }
                                            config::Source::Server(_) => {}
                                        }
                                    },
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
                                        spotify_browsers: spotify_browsers.clone(),
                                        spotify_browser: config.read().spotify_browser.clone(),
                                        on_spotify_browser: move |v: Option<String>| {
                                            config.write().spotify_browser = v;
                                        },
                                        spotify_prefer_active_device: config.read().spotify_prefer_active_device,
                                        on_spotify_prefer_active_device: move |v: bool| {
                                            config.write().spotify_prefer_active_device = v;
                                        },
                                    }
                                }
                            }
                        }
                    }

                    if active_category() == SettingsCategory::Customization {
                        SettingItem {
                            title: i18n::t("reduce_animations").to_string(),
                            config_key: "reduce_animations",
                            control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().reduce_animations,
                                    on_change: move |val| config.write().reduce_animations = val,
                                }
                            }
                        }
                        if cfg!(not(target_os = "android")) {
                            SettingItem {
                                title: i18n::t("fullscreen_use_player_bar").to_string(),
                                config_key: "fullscreen_use_player_bar",
                                control: rsx! {
                                    ToggleSetting {
                                        enabled: config.read().fullscreen_use_player_bar,
                                        on_change: move |val| config.write().fullscreen_use_player_bar = val,
                                    }
                                }
                            }
                        }
                    }
                    if active_category() == SettingsCategory::General {
                        SettingItem {
                            title: i18n::t("auto_check_updates").to_string(),
                            config_key: "auto_check_updates",
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
                                config_key: "minimize_to_tray",
                                control: rsx! {
                                    ToggleSetting {
                                        enabled: config.read().minimize_to_tray,
                                        on_change: move |val| config.write().minimize_to_tray = val,
                                    }
                                }
                            }
                        }
                    }
                    if active_category() == SettingsCategory::Customization {
                        SettingItem {
                            title: i18n::t("show_source_toggle").to_string(),
                            config_key: "show_source_toggle",
                                control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().show_source_toggle,
                                    on_change: move |val| config.write().show_source_toggle = val,
                                }
                            }
                        }
                    }
                    if active_category() == SettingsCategory::Customization {
                        SettingItem {
                            title: i18n::t("show_row_images").to_string(),
                            config_key: "show_row_images",
                            control: rsx! {
                                ToggleSetting {
                                    enabled: config.read().show_row_images,
                                    on_change: move |val| config.write().show_row_images = val,
                                }
                            }
                        }
                    }
                    if active_category() == SettingsCategory::Customization {
                        if cfg!(any(target_os = "linux", target_os = "windows")) {
                            SettingItem {
                                title: i18n::t("titlebar_mode").to_string(),
                                config_key: "titlebar_mode",
                                control: rsx! {
                                    {
                                        let current_mode = config.read().titlebar_mode;
                                        rsx! {
                                            AppSelect {
                                                class: "settings-select",
                                                value: match current_mode { config::TitlebarMode::System => "system", config::TitlebarMode::Off => "off", config::TitlebarMode::Custom => "custom" }.to_string(),
                                                options: vec![("custom".into(), i18n::t("titlebar_custom")), ("system".into(), i18n::t("titlebar_system")), ("off".into(), i18n::t("titlebar_off"))],
                                                on_change: move |value: String| {
                                                    config.write().titlebar_mode = match value.as_str() {
                                                        "system" => config::TitlebarMode::System,
                                                        "off" => config::TitlebarMode::Off,
                                                        _ => config::TitlebarMode::Custom,
                                                    };
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("ui_style").to_string(),
                            config_key: "ui_style",
                            control: rsx! {
                                {
                                    let current_style = config.read().ui_style;
                                    rsx! {
                                        AppSelect {
                                            class: "settings-select",
                                            value: (if current_style == config::UiStyle::Vaxry { "vaxry" } else { "normal" }).to_string(),
                                            options: vec![("normal".into(), i18n::t("ui_normal")), ("vaxry".into(), i18n::t("ui_vaxry"))],
                                            on_change: move |value: String| {
                                                config.write().ui_style = match value.as_str() {
                                                    "vaxry" => config::UiStyle::Vaxry,
                                                    _ => config::UiStyle::Normal,
                                                };
                                            },
                                        }
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("player_bar_position").to_string(),
                            config_key: "player_bar_position",
                            control: rsx! {
                                {
                                    let current_position = config.read().player_bar_position;
                                    rsx! {
                                        AppSelect {
                                            class: "settings-select",
                                            value: (if current_position == config::PlayerBarPosition::Top { "top" } else { "bottom" }).to_string(),
                                            options: vec![("bottom".into(), i18n::t("position_bottom")), ("top".into(), i18n::t("position_top"))],
                                            on_change: move |value: String| {
                                                config.write().player_bar_position = match value.as_str() {
                                                    "top" => config::PlayerBarPosition::Top,
                                                    _ => config::PlayerBarPosition::Bottom,
                                                };
                                            },
                                        }
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("settings_layout").to_string(),
                            config_key: "settings_layout",
                            control: rsx! {
                                {
                                    let current_layout = config.read().settings_layout;
                                    rsx! {
                                        AppSelect {
                                            class: "settings-select",
                                            value: (if current_layout == config::SettingsLayout::TopBar { "topbar" } else { "cd" }).to_string(),
                                            options: vec![("cd".into(), i18n::t("settings_layout_cd")), ("topbar".into(), i18n::t("settings_layout_topbar"))],
                                            on_change: move |value: String| {
                                                config.write().settings_layout = match value.as_str() {
                                                    "topbar" => config::SettingsLayout::TopBar,
                                                    _ => config::SettingsLayout::Cd,
                                                };
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if active_category() == SettingsCategory::General {
                        SettingItem {
                            title: i18n::t("back_behavior").to_string(),
                            config_key: "back_behavior",
                            control: rsx! {
                                BackBehaviorSelector {
                                    current: config.read().back_behavior,
                                    on_change: move |val| config.write().back_behavior = val,
                                }
                            }
                        }
                    }
                }
                }
                if active_category() == SettingsCategory::General {
                    BuildInfoCard {}
                }
                if active_category() == SettingsCategory::Customization {
                    {theme_editor_section(config)}
                }
                if active_category() == SettingsCategory::Connectivity {
                    ConnectivitySection { config }
                }
                if active_category() == SettingsCategory::Downloads {
                    DownloadsSection { config }
                }
                if active_category() == SettingsCategory::Metadata {
                    MetadataSection { config }
                }
                if active_category() == SettingsCategory::Player {
                    PlayerSection { config }
                }
                if active_category() == SettingsCategory::Tools {
                    div { class: "space-y-8",
                        {logs_section(config)}
                        {hooks::debug_db_section()}
                    }
                }
            }
            }

            if show_add_server() {
                AddServerPopup {
                    server_name,
                    server_url,
                    server_service,
                    yt_browser,
                    yt_anonymous,
                    host_access,
                    error,
                    on_close: move |_| show_add_server.set(false),
                    on_save: handle_add_server
                }
            }

            if show_add_local_source() {
                AddLocalSourcePopup {
                    name: local_source_name,
                    directories: local_source_directories,
                    error: local_source_error,
                    on_close: move |_| {
                        show_add_local_source.set(false);
                        local_source_name.set(String::new());
                        local_source_directories.set(Vec::new());
                        local_source_error.set(None);
                    },
                    on_save: move |_| {
                        let name = local_source_name().trim().to_string();
                        if name.is_empty() {
                            local_source_error.set(Some(i18n::t("local_library_name_required").to_string()));
                            return;
                        }
                        let directories = local_source_directories();
                        if directories.is_empty() {
                            local_source_error.set(Some(i18n::t("local_library_folder_required").to_string()));
                            return;
                        }
                        let source = config::SavedLocalSource::new(name, directories);
                        let active = config::Source::LocalLibrary(source.id.clone());
                        {
                            let mut cfg = config.write();
                            cfg.add_local_source(source);
                            cfg.set_active_local_source(active);
                        }
                        show_add_local_source.set(false);
                        local_source_name.set(String::new());
                        local_source_directories.set(Vec::new());
                        local_source_error.set(None);
                    },
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
