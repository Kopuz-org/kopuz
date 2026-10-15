mod desktop_tools;
mod index;
mod navigation;
mod sections;

use desktop_tools::{logs_section, theme_editor_section};
use index::SettingsIndex;
use navigation::{SettingsCategory, SettingsNavigation};
use sections::{
    ConnectivitySection, DownloadsSection, EqualizerSection, MetadataSection, PlayerSection,
};

use components::settings_items::{
    AppSelect, BackBehaviorSelector, LanguageSelector, RadioRegistryDropdown, SettingItem,
    SettingsGroup, SettingsSection, SourceSettings, ThemeSelector, ToggleSetting,
};
use components::settings_popups::{
    AddRegistryPopup, AddSourcePopup, BrowserSignInPopup, LoginPopup,
};
use components::settings_remote_folders::RemoteFolderSettings;
use config::{
    BackgroundEdit, IntegrationsEdit, LibraryEdit, LyricsEdit, PlaybackEdit, RadioRegistryOp,
    SettingsEdit, SystemEdit, UiEdit,
};
use dioxus::prelude::*;

use crate::DebugPanel;
use hooks::use_player_controller::PlayerController;

/// The sources section, which Manage sources opens Settings at.
const MEDIA_SERVERS_SECTION: &str = "settings-media-servers";

/// The category Settings shows, held at the app root so the remount a language change does keeps it.
#[derive(Clone, Copy)]
pub struct OpenCategory(pub Signal<SettingsCategory>);

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
                class: "app-icon-button p-2 rounded text-white/35 hover:text-white hover:bg-white/10 transition-colors shrink-0",
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

/// Whether a resource has an answer for its current inputs. `finished` only
/// peeks, so it would not re-render when the answer lands.
fn settled<T>(resource: &Resource<T>) -> bool {
    matches!(
        *resource.state().read(),
        UseResourceState::Ready | UseResourceState::Stopped
    )
}

#[component]
pub fn Settings(settings: Signal<config::Settings>) -> Element {
    let ctrl = use_context::<PlayerController>();
    let edits = hooks::settings::use_settings();
    // The sources are the daemon's: it holds their credentials, so the config
    // this page reads never carries them.
    let sources = hooks::sources::use_sources();
    let all_sources = use_memo(move || sources.read().clone().unwrap_or_default());
    let active_server = use_memo(move || {
        all_sources()
            .into_iter()
            .find(|source| source.active && source.needs_network)
    });
    let mut show_add_source = use_signal(|| false);
    let mut show_login = use_signal(|| false);

    let services = hooks::sources::use_services();
    let source_name = use_signal(String::new);
    let source_service = use_signal(String::new);
    let draft_values = use_signal(Vec::<api::FieldValue>::new);
    let draft_secrets = use_signal(Vec::<api::FieldValue>::new);
    let draft_check = use_signal(|| Option::<api::DraftCheck>::None);
    let sessions_for = use_memo(move || {
        if !show_add_source() {
            return None;
        }
        let picked = source_service();
        if !picked.is_empty() {
            return Some(picked);
        }
        services
            .read()
            .as_ref()
            .and_then(|offered| offered.first().map(|service| service.id.clone()))
    });
    let browser_sessions = hooks::sources::use_browser_sessions(sessions_for);
    let importing = use_signal(|| Option::<String>::None);
    // Signing an existing source in again offers the same profiles first.
    let mut show_reauth = use_signal(|| false);
    let reauth_error = use_signal(|| Option::<String>::None);
    let reauth_for = use_memo(move || {
        show_reauth()
            .then(|| active_server().map(|server| server.service.id))
            .flatten()
    });
    let reauth_sessions = hooks::sources::use_browser_sessions(reauth_for);

    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);

    let error = use_signal(|| Option::<String>::None);
    let mut login_error = use_signal(|| Option::<String>::None);
    let is_loading = use_signal(|| false);
    let OpenCategory(mut active_category) = use_context();
    let settings_anchor = try_consume_context::<components::source_switcher::SettingsAnchor>();
    let subpage = try_consume_context::<components::tabbar::SettingsSubpage>()
        .map(|components::tabbar::SettingsSubpage(subpage)| subpage);

    // A section a tab switch has to render before it can be scrolled to; it scrolls itself once mounted.
    let mut pending_section = use_signal(|| None::<&'static str>);
    let mut open_category = move |category: SettingsCategory, section: Option<&'static str>| {
        let shown = *active_category.peek() == category;
        active_category.set(category);
        if cfg!(target_os = "android")
            && let Some(mut subpage) = subpage
        {
            subpage.set(Some(category.title_key()));
        }
        match section {
            Some(section) if !shown => pending_section.set(Some(section)),
            _ => {
                let target = section.unwrap_or("settings-category-content");
                let _ = document::eval(&format!(
                    "requestAnimationFrame(() => document.getElementById('{target}')?.scrollIntoView({{ block: 'start' }}))"
                ));
            }
        }
    };
    let mut scroll_if_pending = move |section: &'static str, mounted: std::rc::Rc<MountedData>| {
        if *pending_section.peek() != Some(section) {
            return;
        }
        pending_section.set(None);
        spawn(async move {
            if let Err(error) = mounted.scroll_to(ScrollBehavior::Instant).await {
                tracing::debug!(%error, section, "scrolling to a settings section failed");
            }
        });
    };

    // The page consumes its own anchor: the app shell cleared it before an already open Settings could read it.
    use_effect(move || {
        let Some(components::source_switcher::SettingsAnchor(mut anchor)) = settings_anchor else {
            return;
        };
        let Some(id) = anchor.read().clone() else {
            return;
        };
        if id == MEDIA_SERVERS_SECTION {
            open_category(SettingsCategory::Library, Some(MEDIA_SERVERS_SECTION));
        }
        anchor.set(None);
    });

    use_effect(move || {
        if cfg!(target_os = "android") && subpage.is_some_and(|subpage| subpage.read().is_none()) {
            let _ = document::eval(
                "requestAnimationFrame(() => document.getElementById('settings-index')?.scrollIntoView({ block: 'start' }))",
            );
        }
    });

    let mut show_add_registry = use_signal(|| false);
    let registry_url = use_signal(String::new);
    let registry_error = use_signal(|| Option::<String>::None);
    let registry_loading = use_signal(|| false);
    let mut registry_toggle_error = use_signal(|| Option::<String>::None);

    let host_access = use_signal(|| true);

    use_effect(move || {
        spawn(async move {
            crate::settings_actions::ensure_host_access(host_access).await;
        });
    });

    let handle_add_registry = move |_| {
        crate::settings_actions::add_registry(
            edits,
            registry_url,
            registry_error,
            registry_loading,
            show_add_registry,
        );
    };

    // Signing in again on an active server: which flow it is belongs to the
    // service, and the daemon runs it.
    let mut sign_in_again = move || match active_server() {
        Some(server) if server.reauth == api::SignInKind::Browser => {
            let mut reauth_error = reauth_error;
            reauth_error.set(None);
            show_reauth.set(true);
        }
        _ => show_login.set(true),
    };

    // A "sign in again" from elsewhere in the app lands here.
    use_effect(move || {
        let mut requested = ctrl.reauth_requested;
        if requested() {
            requested.set(false);
            sign_in_again();
        }
    });

    // A service with no profiles to offer goes straight to the browser.
    use_effect(move || {
        let looked = reauth_sessions.read().clone();
        if show_reauth()
            && settled(&reauth_sessions)
            && reauth_for().is_some()
            && looked.is_some_and(|found| found.is_empty())
        {
            show_reauth.set(false);
            if let Some(server) = active_server() {
                crate::settings_actions::authenticate(server.id, error, ctrl.playback_error);
            }
        }
    });

    // The daemon checks the draft as it is typed, so the form knows what is
    // wrong with it and which sign-in saving it will start.
    use_effect(move || {
        let draft = crate::settings_actions::draft(
            source_name,
            source_service,
            draft_values,
            draft_secrets,
        );
        crate::settings_actions::check_draft(draft, draft_check);
    });

    let handle_add_source = move |_| {
        crate::settings_actions::add_source(
            crate::settings_actions::draft(
                source_name,
                source_service,
                draft_values,
                draft_secrets,
            ),
            source_name,
            draft_values,
            draft_secrets,
            error,
            show_add_source,
            show_login,
            ctrl.playback_error,
        );
    };

    let handle_import_session = move |session: api::BrowserSession| {
        crate::settings_actions::add_source_from_session(
            crate::settings_actions::draft(
                source_name,
                source_service,
                draft_values,
                draft_secrets,
            ),
            session,
            importing,
            source_name,
            draft_values,
            draft_secrets,
            error,
            show_add_source,
            show_login,
            ctrl.playback_error,
        );
    };

    let handle_switch_server = move |id: String| {
        crate::settings_actions::switch_server(id, error, show_login, ctrl.playback_error);
    };

    let handle_delete_saved = move |id: String| {
        crate::settings_actions::delete_saved(id);
    };

    let handle_login = move |_| {
        let Some(server) = active_server() else {
            return;
        };
        crate::settings_actions::login_with_password(
            server.id,
            username,
            password,
            login_error,
            is_loading,
            show_login,
        );
    };

    let is_android = cfg!(target_os = "android");
    let showing_index = is_android && subpage.is_none_or(|subpage| subpage.read().is_none());
    let is_material3 = settings.read().ui.ui_style == config::UiStyle::Material3;

    rsx! {
        div { class: if is_android { "px-3 pt-2 pb-6 w-full max-w-7xl mx-auto" } else if is_material3 || settings.read().ui.settings_layout == config::SettingsLayout::TopBar { "settings-page settings-layout-topbar px-6 py-7 w-full max-w-7xl mx-auto" } else { "settings-page settings-layout-cd px-6 py-7 w-full max-w-7xl mx-auto" },
            if !is_android {
                h1 { class: "text-2xl font-semibold tracking-tight text-white mb-5 px-1", "{i18n::t(\"settings\")}" }
            }

            if let Some(notices) = try_consume_context::<hooks::config_view::LockedKeys>()
                .map(|layers| lock_notices(&layers.0.read()))
                .filter(|notices| !notices.is_empty())
            {
                aside { class: "mb-4 rounded-xl border border-white/10 bg-white/5 px-5 py-3 flex items-center gap-3",
                    i { class: "fa-solid fa-lock text-white/40 text-sm shrink-0" }
                    div { class: "flex flex-col gap-1",
                        for notice in notices {
                            p { class: "text-sm text-white/70", "{notice}" }
                        }
                    }
                }
            }

            if showing_index {
                SettingsIndex {
                    settings,
                    active_source: all_sources().into_iter().find(|source| source.active),
                    on_select: move |category| open_category(category, None),
                }
            } else {
            div { class: "settings-workspace",
                if !is_android {
                    SettingsNavigation {
                        selected: active_category(),
                        on_select: move |category| open_category(category, None),
                    }
                }
                main { id: "settings-category-content", class: "settings-category-content",
                if matches!(active_category(), SettingsCategory::General | SettingsCategory::Customization | SettingsCategory::Library) {
                    SettingsSection {
                    title: active_category().title(),
                    if active_category() == SettingsCategory::Customization {
                        SettingsGroup { label: i18n::t("settings_group_theme") }
                        SettingItem {
                            title: i18n::t("settings_group_theme").to_string(),
                            config_key: "theme",
                            control: rsx! {
                                ThemeSelector {
                                    current_theme: settings.read().ui.theme.clone(),
                                    on_change: move |theme| {
                                        edits.edit(SettingsEdit::Ui(UiEdit::Theme(theme)));
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("language").to_string(),
                            config_key: "language",
                            control: rsx! {
                                LanguageSelector {
                                    current_language: settings.read().ui.language.clone(),
                                    on_change: move |lang: String| {
                                        edits.edit(SettingsEdit::Ui(UiEdit::Language(lang)));
                                    }
                                }
                            }
                        }

                        if cfg!(not(target_os = "android"))
                            && settings.read().ui.theme == utils::live_theme::THEME_ID
                        {
                            SettingItem {
                                title: i18n::t("live_theme_file").to_string(),
                                config_key: "live_theme_path",
                                control: rsx! {
                                    div { class: "flex items-center gap-2",
                                        span {
                                            class: "text-xs text-white/50 font-mono max-w-[220px] truncate",
                                            "{utils::live_theme::resolve_path(&settings.read().ui.live_theme_path).display()}"
                                        }
                                        if !settings.read().ui.live_theme_path.is_empty() {
                                            button {
                                                class: "app-button-tonal app-button-danger px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-red-300 text-sm transition-colors",
                                                onclick: move |_| edits.edit(SettingsEdit::Ui(UiEdit::LiveThemePath(String::new()))),
                                                "{i18n::t(\"remove\")}"
                                            }
                                        }
                                        button {
                                            class: "app-button-tonal px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                                            onclick: move |_| {
                                                #[cfg(not(target_os = "android"))]
                                                spawn(async move {
                                                    if let Some(file) = rfd::AsyncFileDialog::new()
                                                        .add_filter("JSON", &["json"])
                                                        .pick_file()
                                                        .await
                                                    {
                                                        edits.edit(SettingsEdit::Ui(UiEdit::LiveThemePath(
                                                            file.path().display().to_string(),
                                                        )));
                                                    }
                                                });
                                            },
                                            "{i18n::t(\"choose_palette\")}"
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
                                        if !settings.read().ui.custom_font_path.is_empty() {
                                            span {
                                                class: "text-xs text-white/50 font-mono max-w-[220px] truncate",
                                                "{settings.read().ui.custom_font_path}"
                                            }
                                            button {
                                                class: "app-button-tonal app-button-danger px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-red-300 text-sm transition-colors",
                                                onclick: move |_| edits.edit(SettingsEdit::Ui(UiEdit::CustomFontPath(String::new()))),
                                                "{i18n::t(\"remove\")}"
                                            }
                                        }
                                        button {
                                            class: "app-button-tonal px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                                            onclick: move |_| {
                                                #[cfg(not(target_os = "android"))]
                                                spawn(async move {
                                                    if let Some(file) = rfd::AsyncFileDialog::new()
                                                        .add_filter("Fonts", &["ttf", "otf", "woff", "woff2"])
                                                        .pick_file()
                                                        .await
                                                    {
                                                        edits.edit(SettingsEdit::Ui(UiEdit::CustomFontPath(
                                                            file.path().display().to_string(),
                                                        )));
                                                    }
                                                });
                                            },
                                            "{i18n::t(\"choose_font\")}"
                                        }
                                    }
                                }
                            }
                        }

                        SettingsGroup { label: i18n::t("settings_group_background") }
                        SettingItem {
                            title: i18n::t("cover_art_background").to_string(),
                            config_key: "cover_art_background",
                            control: rsx! {
                                ToggleSetting {
                                    enabled: settings.read().background.cover_art_background,
                                    on_change: move |val| edits.edit(SettingsEdit::Background(BackgroundEdit::CoverArtBackground(val))),
                                }
                            }
                        }
                        if cfg!(not(target_os = "android")) {
                            SettingItem {
                                title: i18n::t("custom_background").to_string(),
                                config_key: "custom_background_path",
                                control: rsx! {
                                    div { class: "flex items-center gap-2",
                                        if !settings.read().background.custom_background_path.is_empty() {
                                            span {
                                                class: "text-xs text-white/50 font-mono max-w-[220px] truncate",
                                                "{settings.read().background.custom_background_path}"
                                            }
                                            button {
                                                class: "app-button-tonal app-button-danger px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-red-300 text-sm transition-colors",
                                                onclick: move |_| edits.edit(SettingsEdit::Background(BackgroundEdit::CustomBackgroundPath(String::new()))),
                                                "{i18n::t(\"remove\")}"
                                            }
                                        }
                                        button {
                                            class: "app-button-tonal px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                                            onclick: move |_| {
                                                #[cfg(not(target_os = "android"))]
                                                spawn(async move {
                                                    if let Some(file) = rfd::AsyncFileDialog::new()
                                                        .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif", "bmp"])
                                                        .pick_file()
                                                        .await
                                                    {
                                                        edits.edit(SettingsEdit::Background(BackgroundEdit::CustomBackgroundPath(
                                                            file.path().display().to_string(),
                                                        )));
                                                    }
                                                });
                                            },
                                            "{i18n::t(\"choose_image\")}"
                                        }
                                    }
                                }
                            }
                        }
                        if settings.read().background.cover_art_background
                            || !settings.read().background.custom_background_path.is_empty()
                        {
                                SettingItem {
                                    title: i18n::t("cover_art_darkening").to_string(),
                                    config_key: "cover_art_darkening",
                                    nested: true,
                                    control: rsx! {
                                        div { class: "settings-slider flex items-center gap-3 min-w-[220px]",
                                            input {
                                                r#type: "range",
                                                min: "0",
                                                max: "95",
                                                step: "5",
                                                value: format!("{}", settings.read().background.cover_art_darkening),
                                                class: "w-40",
                                                style: components::settings::items::range_style(f64::from(settings.read().background.cover_art_darkening), 0.0, 95.0),
                                                oninput: move |evt| {
                                                    if let Ok(value) = evt.value().parse::<u8>() {
                                                        edits.preview(SettingsEdit::Background(BackgroundEdit::CoverArtDarkening(value.min(95))));
                                                    }
                                                },
                                                onchange: move |evt| {
                                                    if let Ok(value) = evt.value().parse::<u8>() {
                                                        edits.edit(SettingsEdit::Background(BackgroundEdit::CoverArtDarkening(value.min(95))));
                                                    }
                                                }
                                            }
                                            span {
                                                class: "text-xs font-mono text-white/80 w-16 text-right",
                                                "{settings.read().background.cover_art_darkening}%"
                                            }
                                        }
                                    }
                                }
                                SettingItem {
                                    title: i18n::t("cover_art_blur").to_string(),
                                    config_key: "cover_art_blur",
                                    nested: true,
                                    control: rsx! {
                                        div { class: "settings-slider flex items-center gap-3 min-w-[220px]",
                                            input {
                                                r#type: "range",
                                                min: "0",
                                                max: "100",
                                                step: "5",
                                                value: format!("{}", settings.read().background.cover_art_blur),
                                                class: "w-40",
                                                style: components::settings::items::range_style(f64::from(settings.read().background.cover_art_blur), 0.0, 100.0),
                                                oninput: move |evt| {
                                                    if let Ok(value) = evt.value().parse::<u8>() {
                                                        edits.preview(SettingsEdit::Background(BackgroundEdit::CoverArtBlur(value.min(100))));
                                                    }
                                                },
                                                onchange: move |evt| {
                                                    if let Ok(value) = evt.value().parse::<u8>() {
                                                        edits.edit(SettingsEdit::Background(BackgroundEdit::CoverArtBlur(value.min(100))));
                                                    }
                                                }
                                            }
                                            span {
                                                class: "text-xs font-mono text-white/80 w-16 text-right",
                                                "{settings.read().background.cover_art_blur}px"
                                            }
                                        }
                                    }
                                }
                        }

                        SettingsGroup { label: i18n::t("lyrics") }
                        SettingItem {
                            title: i18n::t("lyrics_depth_blur").to_string(),
                            config_key: "lyrics_depth_blur",
                            control: rsx! {
                                ToggleSetting {
                                    enabled: settings.read().lyrics.lyrics_depth_blur,
                                    on_change: move |val| edits.edit(SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlur(val))),
                                }
                            }
                        }
                        if settings.read().lyrics.lyrics_depth_blur {
                            SettingItem {
                                title: i18n::t("lyrics_depth_blur_strength").to_string(),
                                config_key: "lyrics_depth_blur_strength",
                                nested: true,
                                control: rsx! {
                                    div { class: "settings-slider flex items-center gap-3 min-w-[220px]",
                                        input {
                                            r#type: "range",
                                            min: "10",
                                            max: "200",
                                            step: "10",
                                            value: format!("{}", settings.read().lyrics.lyrics_depth_blur_strength),
                                            class: "w-40",
                                            style: components::settings::items::range_style(f64::from(settings.read().lyrics.lyrics_depth_blur_strength), 10.0, 200.0),
                                            oninput: move |evt| {
                                                if let Ok(value) = evt.value().parse::<u8>() {
                                                    edits.preview(SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlurStrength(value.clamp(10, 200))));
                                                }
                                            },
                                            onchange: move |evt| {
                                                if let Ok(value) = evt.value().parse::<u8>() {
                                                    edits.edit(SettingsEdit::Lyrics(LyricsEdit::LyricsDepthBlurStrength(value.clamp(10, 200))));
                                                }
                                            }
                                        }
                                        span {
                                            class: "text-xs font-mono text-white/80 w-16 text-right",
                                            "{settings.read().lyrics.lyrics_depth_blur_strength}%"
                                        }
                                    }
                                }
                            }
                        }

                        SettingsGroup { label: i18n::t("settings_group_interface") }
                        SettingItem {
                            title: i18n::t("reduce_animations").to_string(),
                            config_key: "reduce_animations",
                            control: rsx! {
                                ToggleSetting {
                                    enabled: settings.read().ui.reduce_animations,
                                    on_change: move |val| edits.edit(SettingsEdit::Ui(UiEdit::ReduceAnimations(val))),
                                }
                            }
                        }
                        if cfg!(not(target_os = "android")) {
                            SettingItem {
                                title: i18n::t("fullscreen_use_player_bar").to_string(),
                                config_key: "fullscreen_use_player_bar",
                                control: rsx! {
                                    ToggleSetting {
                                        enabled: settings.read().ui.fullscreen_use_player_bar,
                                        on_change: move |val| edits.edit(SettingsEdit::Ui(UiEdit::FullscreenUsePlayerBar(val))),
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("show_source_toggle").to_string(),
                            config_key: "show_source_toggle",
                                control: rsx! {
                                ToggleSetting {
                                    enabled: settings.read().ui.show_source_toggle,
                                    on_change: move |val| edits.edit(SettingsEdit::Ui(UiEdit::ShowSourceToggle(val))),
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("show_row_images").to_string(),
                            config_key: "show_row_images",
                            control: rsx! {
                                ToggleSetting {
                                    enabled: settings.read().ui.show_row_images,
                                    on_change: move |val| edits.edit(SettingsEdit::Ui(UiEdit::ShowRowImages(val))),
                                }
                            }
                        }
                        if cfg!(any(target_os = "linux", target_os = "windows")) {
                            SettingItem {
                                title: i18n::t("titlebar_mode").to_string(),
                                config_key: "titlebar_mode",
                                control: rsx! {
                                    {
                                        let current_mode = settings.read().ui.titlebar_mode;
                                        rsx! {
                                            AppSelect {
                                                class: "settings-select",
                                                value: match current_mode { config::TitlebarMode::System => "system", config::TitlebarMode::Off => "off", config::TitlebarMode::Custom => "custom" }.to_string(),
                                                options: vec![("custom".into(), i18n::t("titlebar_custom")), ("system".into(), i18n::t("titlebar_system")), ("off".into(), i18n::t("titlebar_off"))],
                                                on_change: move |value: String| {
                                                    edits.edit(SettingsEdit::Ui(UiEdit::TitlebarMode(match value.as_str() {
                                                        "system" => config::TitlebarMode::System,
                                                        "off" => config::TitlebarMode::Off,
                                                        _ => config::TitlebarMode::Custom,
                                                    })));
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
                                    let current_style = settings.read().ui.ui_style;
                                    rsx! {
                                        AppSelect {
                                            class: "settings-select",
                                            value: current_style.as_str().to_string(),
                                            options: vec![("normal".into(), i18n::t("ui_normal")), ("vaxry".into(), i18n::t("ui_vaxry")), ("material3".into(), i18n::t("ui_material3"))],
                                            on_change: move |value: String| {
                                                edits.edit(SettingsEdit::Ui(UiEdit::UiStyle(match value.as_str() {
                                                    "vaxry" => config::UiStyle::Vaxry,
                                                    "material3" => config::UiStyle::Material3,
                                                    _ => config::UiStyle::Normal,
                                                })));
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
                                    let current_position = settings.read().ui.player_bar_position;
                                    rsx! {
                                        AppSelect {
                                            class: "settings-select",
                                            value: (if current_position == config::PlayerBarPosition::Top { "top" } else { "bottom" }).to_string(),
                                            options: vec![("bottom".into(), i18n::t("position_bottom")), ("top".into(), i18n::t("position_top"))],
                                            on_change: move |value: String| {
                                                edits.edit(SettingsEdit::Ui(UiEdit::PlayerBarPosition(match value.as_str() {
                                                    "top" => config::PlayerBarPosition::Top,
                                                    _ => config::PlayerBarPosition::Bottom,
                                                })));
                                            },
                                        }
                                    }
                                }
                            }
                        }
                        if !is_android && !is_material3 {
                            SettingItem {
                                title: i18n::t("settings_layout").to_string(),
                                config_key: "settings_layout",
                                control: rsx! {
                                    {
                                        let current_layout = settings.read().ui.settings_layout;
                                        rsx! {
                                            AppSelect {
                                                class: "settings-select",
                                                value: (if current_layout == config::SettingsLayout::TopBar { "topbar" } else { "cd" }).to_string(),
                                                options: vec![("cd".into(), i18n::t("settings_layout_cd")), ("topbar".into(), i18n::t("settings_layout_topbar"))],
                                                on_change: move |value: String| {
                                                    edits.edit(SettingsEdit::Ui(UiEdit::SettingsLayout(match value.as_str() {
                                                        "topbar" => config::SettingsLayout::TopBar,
                                                        _ => config::SettingsLayout::Cd,
                                                    })));
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if active_category() == SettingsCategory::Library {
                        RadioRegistryDropdown {
                            registries: settings.read().integrations.radio_registries.clone(),
                            error: registry_toggle_error,
                            on_toggle: move |index: usize| {
                                let Some(entry) = settings.read().integrations.radio_registries.get(index).cloned() else {
                                    return;
                                };
                                let (enabling, url) = (!entry.enabled, entry.url.clone());
                                let put = move |enabled: bool| {
                                    edits.edit(SettingsEdit::Integrations(IntegrationsEdit::RadioRegistries(
                                        RadioRegistryOp::Put(config::RegistryEntry { enabled, ..entry.clone() }),
                                    )));
                                };

                                if enabling && !url.is_empty() {
                                    registry_toggle_error.set(None);
                                    let api = hooks::consume_api();
                                    spawn(async move {
                                        match api.validate_radio_registry(url).await {
                                            Ok(_) => {
                                                put(true);
                                                registry_toggle_error.set(None);
                                            }
                                            Err(e) => {
                                                registry_toggle_error.set(Some(i18n::t_with("radio_registry_enable_failed", &[("error", components::forms::error_text(&e).into())])));
                                            }
                                        }
                                    });
                                } else {
                                    put(false);
                                    registry_toggle_error.set(None);
                                }
                            },
                            on_add: move |_| show_add_registry.set(true),
                            on_delete: move |index: usize| {
                                let url = settings
                                    .read()
                                    .integrations
                                    .radio_registries
                                    .get(index)
                                    .filter(|entry| !entry.is_default)
                                    .map(|entry| entry.url.clone());
                                if let Some(url) = url {
                                    edits.edit(SettingsEdit::Integrations(IntegrationsEdit::RadioRegistries(
                                        RadioRegistryOp::Remove { url },
                                    )));
                                }
                            }
                        }

                        SettingItem {
                            title: i18n::t("sync_interval").to_string(),
                            config_key: "sync_interval_minutes",
                            control: rsx! {
                                AppSelect {
                                    class: "settings-select",
                                    value: settings.read().library.sync_interval_minutes.to_string(),
                                    options: sync_interval_options(settings.read().library.sync_interval_minutes),
                                    on_change: move |value: String| {
                                        if let Ok(minutes) = value.parse::<u32>() {
                                            edits.edit(SettingsEdit::Library(LibraryEdit::SyncIntervalMinutes(minutes)));
                                        }
                                    },
                                }
                            }
                        }

                        // Clears the sticky category tabs when Manage sources scrolls here.
                        div { id: MEDIA_SERVERS_SECTION, style: "scroll-margin-top: 5rem;", onmounted: move |event| scroll_if_pending(MEDIA_SERVERS_SECTION, event.data()),
                            SettingItem {
                                title: i18n::t("sources").to_string(),
                                stacked: true,
                                control: rsx! {
                                    SourceSettings {
                                        sources: all_sources(),
                                        on_add: move |_| show_add_source.set(true),
                                        on_delete: handle_delete_saved,
                                        on_switch: handle_switch_server,
                                        on_login: move |_| sign_in_again(),
                                        remote_folders: remote_folder_settings(active_server()),
                                        host_access: host_access(),
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
                                    enabled: settings.read().system.auto_check_updates,
                                    on_change: move |val| edits.edit(SettingsEdit::System(SystemEdit::AutoCheckUpdates(val))),
                                }
                            }
                        }
                        if cfg!(not(target_os = "android")) {
                            SettingItem {
                                title: i18n::t("minimize_to_tray").to_string(),
                                config_key: "minimize_to_tray",
                                control: rsx! {
                                    ToggleSetting {
                                        enabled: settings.read().system.minimize_to_tray,
                                        on_change: move |val| edits.edit(SettingsEdit::System(SystemEdit::MinimizeToTray(val))),
                                    }
                                }
                            }
                        }
                        SettingItem {
                            title: i18n::t("back_behavior").to_string(),
                            config_key: "back_behavior",
                            control: rsx! {
                                BackBehaviorSelector {
                                    current: settings.read().playback.back_behavior,
                                    on_change: move |val| edits.edit(SettingsEdit::Playback(PlaybackEdit::BackBehavior(val))),
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
                    {theme_editor_section(settings)}
                }
                if active_category() == SettingsCategory::Connectivity {
                    ConnectivitySection {}
                }
                if active_category() == SettingsCategory::Downloads {
                    DownloadsSection { settings }
                }
                if active_category() == SettingsCategory::Metadata {
                    MetadataSection { settings }
                }
                if active_category() == SettingsCategory::Player {
                    PlayerSection { settings }
                }
                if active_category() == SettingsCategory::Equalizer {
                    EqualizerSection { settings }
                }
                if active_category() == SettingsCategory::Tools {
                    div { class: "space-y-8",
                        {logs_section(settings)}
                        // The app fills this in debug builds; it is the only
                        // crate that still holds a write-capable database.
                        if let Some(panel) = try_consume_context::<DebugPanel>() {
                            {(panel.0)()}
                        }
                    }
                }
            }
            }
            }

            if show_add_source() {
                AddSourcePopup {
                    services: services.read().clone().unwrap_or_default(),
                    service: source_service,
                    name: source_name,
                    values: draft_values,
                    secrets: draft_secrets,
                    check: draft_check(),
                    host_access: host_access(),
                    error,
                    sessions: settled(&browser_sessions)
                        .then(|| browser_sessions.read().clone())
                        .flatten(),
                    importing: importing(),
                    on_close: move |_| show_add_source.set(false),
                    on_save: handle_add_source,
                    on_import: handle_import_session,
                }
            }

            if show_reauth() {
                BrowserSignInPopup {
                    service_name: active_server()
                        .map(|server| components::forms::text(&server.service.name))
                        .unwrap_or_default(),
                    sessions: settled(&reauth_sessions)
                        .then(|| reauth_sessions.read().clone())
                        .flatten(),
                    importing: importing(),
                    error: reauth_error,
                    on_close: move |_| show_reauth.set(false),
                    on_browser: move |_| {
                        show_reauth.set(false);
                        if let Some(server) = active_server() {
                            crate::settings_actions::authenticate(server.id, error, ctrl.playback_error);
                        }
                    },
                    on_import: move |session: api::BrowserSession| {
                        if let Some(server) = active_server() {
                            crate::settings_actions::reauth_from_session(
                                server.id,
                                session,
                                importing,
                                reauth_error,
                                show_reauth,
                            );
                        }
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
                    service_name: active_server()
                        .map(|server| components::forms::text(&server.service.name))
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

/// One sentence for each kind of layer that pins a setting, so the banner names only what does.
fn lock_notices(locks: &[api::SettingLock]) -> Vec<String> {
    let pinned_by = |kind: fn(&api::LockSource) -> bool| locks.iter().any(|lock| kind(&lock.by));
    [
        (
            pinned_by(|by| matches!(by, api::LockSource::Managed(_))),
            i18n::t("settings_managed_notice"),
        ),
        (
            pinned_by(|by| matches!(by, api::LockSource::DropIn(_))),
            i18n::t("settings_dropin_notice"),
        ),
        (
            pinned_by(|by| matches!(by, api::LockSource::Env(_))),
            i18n::t("settings_env_notice"),
        ),
    ]
    .into_iter()
    .filter_map(|(pinned, notice)| pinned.then_some(notice))
    .collect()
}

const SYNC_INTERVAL_PRESETS: [u32; 6] = [0, 60, 6 * 60, 12 * 60, 24 * 60, 7 * 24 * 60];

fn sync_interval_label(minutes: u32) -> String {
    let (key, count) = match minutes {
        0 => return i18n::t("sync_interval_never").to_string(),
        m if m % (24 * 60) == 0 => ("sync_interval_days", m / (24 * 60)),
        m if m % 60 == 0 => ("sync_interval_hours", m / 60),
        m => ("sync_interval_minutes", m),
    };
    i18n::t_with(key, &[("count", count.into())])
}

/// The presets, plus the current value when a settings file set one that is not among them.
fn sync_interval_options(current: u32) -> Vec<(String, String)> {
    let mut minutes = SYNC_INTERVAL_PRESETS.to_vec();
    if !minutes.contains(&current) {
        minutes.push(current);
        minutes.sort_unstable();
    }
    minutes
        .into_iter()
        .map(|m| (m.to_string(), sync_interval_label(m)))
        .collect()
}

/// The active server's folder picker, or `None` unless it browses a folder
/// tree and is signed in -- the daemon lists the folders, so it needs both.
fn remote_folder_settings(server: Option<api::SourceInfo>) -> Option<RemoteFolderSettings> {
    let server = server?;
    if !server.capabilities.browse_folders || !server.authenticated {
        return None;
    }
    let folders = api::spec_value(&server.settings, "directories")
        .map(api::decode_directories)
        .unwrap_or_default();
    let add_id = server.id.clone();
    let add_folders = folders.clone();
    let remove_id = server.id.clone();
    let remove_folders = folders.clone();

    Some(RemoteFolderSettings {
        source_id: server.id,
        folders,
        on_add: EventHandler::new(move |path: String| {
            let mut next = add_folders.clone();
            if !next.contains(&path) {
                next.push(path);
            }
            set_directories(add_id.clone(), next);
        }),
        on_remove: EventHandler::new(move |index: usize| {
            let mut next = remove_folders.clone();
            if index < next.len() {
                next.remove(index);
                set_directories(remove_id.clone(), next);
            }
        }),
    })
}

fn set_directories(id: String, directories: Vec<String>) {
    let api = hooks::consume_api();
    spawn(async move {
        let folders = api::FieldValue::new("directories", api::encode_directories(&directories));
        if let Err(error) = api.set_source_settings(id, vec![folders]).await {
            tracing::warn!(%error, "setting the source folders failed");
            hooks::toast::toast_api_error(&error);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::lock_notices;

    fn lock(by: api::LockSource) -> api::SettingLock {
        api::SettingLock {
            key: config::SettingKey::Ui(config::UiKey::Theme),
            by,
        }
    }

    #[test]
    fn the_lock_banner_names_only_the_layers_that_pin() {
        let dropin = lock(api::LockSource::DropIn("/x/settings.d/30-c.toml".into()));
        let env = lock(api::LockSource::Env("KOPUZ_CONFIG_THEME".into()));
        let managed = lock(api::LockSource::Managed("/x/settings.toml".into()));

        assert!(lock_notices(&[]).is_empty());
        assert_eq!(
            lock_notices(std::slice::from_ref(&dropin)),
            vec![i18n::t("settings_dropin_notice")]
        );
        assert_eq!(
            lock_notices(&[env.clone(), dropin.clone(), env]),
            vec![
                i18n::t("settings_dropin_notice"),
                i18n::t("settings_env_notice")
            ]
        );
        assert_eq!(
            lock_notices(&[managed]),
            vec![i18n::t("settings_managed_notice")]
        );
    }
}
