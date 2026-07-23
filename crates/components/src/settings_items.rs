use config::{
    AppConfig, BackBehavior, ChannelMode, DeviceChangeBehavior, EqPreset,
    EqualizerSettings as EqualizerConfig, SampleRateMode, SavedServer,
};
use dioxus::prelude::*;
#[cfg(not(target_os = "android"))]
use rfd::AsyncFileDialog;
use scrobble::lastfm;
use scrobble::librefm;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::Instrument;

static APP_SELECT_ID: AtomicUsize = AtomicUsize::new(0);

#[component]
pub fn SettingItem(title: String, control: Element) -> Element {
    rsx! {
        div { class: "settings-row flex items-center justify-between gap-5 px-5 py-2.5",
            p { class: "min-w-0 text-sm text-white/90 font-medium", "{title}" }
            {control}
        }
    }
}

#[component]
pub fn SettingsSection(title: String, children: Element) -> Element {
    let mut expanded = use_signal(|| true);

    rsx! {
        section { class: "settings-section rounded-xl overflow-visible",
            button {
                r#type: "button",
                class: "settings-section-header w-full flex items-center justify-between gap-3 px-5 py-3 rounded-t-xl text-left",
                aria_expanded: expanded(),
                onclick: move |_| expanded.toggle(),
                h2 { class: "text-xs font-semibold uppercase tracking-wider text-white/65",
                    "{title}"
                }
                i { class: if expanded() { "fa-solid fa-chevron-up text-[10px] text-white/40" } else { "fa-solid fa-chevron-down text-[10px] text-white/40" } }
            }
            if expanded() {
                div { class: "settings-section-body divide-y divide-white/[0.07]", {children} }
            }
        }
    }
}

#[component]
pub fn AppSelect(
    value: String,
    options: Vec<(String, String)>,
    on_change: EventHandler<String>,
    #[props(default)] class: String,
) -> Element {
    let mut open = use_signal(|| false);
    let instance_id = use_hook(|| APP_SELECT_ID.fetch_add(1, Ordering::Relaxed));
    let trigger_id = format!("app-select-trigger-{instance_id}");
    let menu_id = format!("app-select-menu-{instance_id}");
    let selected_index = options
        .iter()
        .position(|(option_value, _)| option_value == &value)
        .unwrap_or(0);
    let mut active_index = use_signal(|| selected_index);
    let mut typeahead = use_signal(String::new);
    let mut typeahead_at = use_signal(std::time::Instant::now);
    use_effect(move || {
        if open() {
            let index = active_index();
            document::eval(&format!(
                "document.getElementById('app-select-option-{instance_id}-{index}')?.scrollIntoView({{block:'nearest'}})"
            ));
        }
    });
    let selected_label = options
        .iter()
        .find(|(option_value, _)| option_value == &value)
        .map(|(_, label)| label.as_str())
        .unwrap_or(value.as_str());
    let open_class = if open() { "z-[70]" } else { "z-0" };
    let active_option_id = format!("app-select-option-{instance_id}-{}", active_index());
    let keyboard_options = options.clone();
    let keyboard_trigger_id = trigger_id.clone();

    rsx! {
        div { class: "app-select relative {open_class} {class}",
            button {
                id: "{trigger_id}",
                r#type: "button",
                role: "combobox",
                class: "app-select-trigger relative z-[1] w-full",
                aria_haspopup: "listbox",
                aria_expanded: open(),
                aria_controls: "{menu_id}",
                aria_activedescendant: if open() { Some(active_option_id.as_str()) } else { None },
                onclick: move |_| {
                    if !open() {
                        active_index.set(selected_index);
                    }
                    open.toggle();
                },
                onkeydown: move |event| {
                    let option_count = keyboard_options.len();
                    if option_count == 0 {
                        return;
                    }

                    let move_active = |next: usize, mut active_index: Signal<usize>| {
                        active_index.set(next);
                    };

                    match event.key() {
                        Key::Escape if open() => {
                            event.prevent_default();
                            open.set(false);
                        }
                        Key::Tab if open() => open.set(false),
                        Key::ArrowDown => {
                            event.prevent_default();
                            if open() {
                                move_active((active_index() + 1) % option_count, active_index);
                            } else {
                                active_index.set(selected_index);
                                open.set(true);
                            }
                        }
                        Key::ArrowUp => {
                            event.prevent_default();
                            if open() {
                                move_active((active_index() + option_count - 1) % option_count, active_index);
                            } else {
                                active_index.set(selected_index);
                                open.set(true);
                            }
                        }
                        Key::Enter => {
                            event.prevent_default();
                            if open() {
                                if let Some((option_value, _)) = keyboard_options.get(active_index()) {
                                    on_change.call(option_value.clone());
                                }
                                open.set(false);
                            } else {
                                active_index.set(selected_index);
                                open.set(true);
                            }
                        }
                        Key::Character(character) if character == " " => {
                            event.prevent_default();
                            if open() {
                                if let Some((option_value, _)) = keyboard_options.get(active_index()) {
                                    on_change.call(option_value.clone());
                                }
                                open.set(false);
                            } else {
                                active_index.set(selected_index);
                                open.set(true);
                            }
                        }
                        Key::Character(character) if !character.chars().any(char::is_control) => {
                            let now = std::time::Instant::now();
                            let mut query = if now.duration_since(*typeahead_at.peek())
                                > std::time::Duration::from_millis(700)
                            {
                                String::new()
                            } else {
                                typeahead.peek().clone()
                            };
                            query.push_str(&character.to_lowercase());
                            typeahead.set(query.clone());
                            typeahead_at.set(now);
                            if let Some(index) = keyboard_options.iter().position(|(_, label)| {
                                label.to_lowercase().starts_with(&query)
                            }) {
                                event.prevent_default();
                                move_active(index, active_index);
                                if !open() {
                                    open.set(true);
                                }
                            }
                        }
                        _ => {}
                    }
                },
                span { class: "truncate", "{selected_label}" }
                svg {
                    class: if open() { "app-select-chevron rotate-180" } else { "app-select-chevron" },
                    view_box: "0 0 16 16",
                    fill: "none",
                    path {
                        d: "m4 6 4 4 4-4",
                        stroke: "currentColor",
                        stroke_width: "1.5",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                    }
                }
            }
            if open() {
                button {
                    r#type: "button",
                    class: "fixed inset-0 z-0 cursor-default",
                    aria_label: "Close menu",
                    onclick: move |_| {
                        open.set(false);
                        document::eval(&format!("document.getElementById('{keyboard_trigger_id}')?.focus()"));
                    },
                    onwheel: move |event| {
                        event.prevent_default();
                        event.stop_propagation();
                    },
                }
                div {
                    id: "{menu_id}",
                    role: "listbox",
                    aria_labelledby: "{trigger_id}",
                    class: "app-select-menu",
                    onwheel: move |event| event.stop_propagation(),
                    for (index, (option_value, label)) in options.iter().enumerate() {
                        {
                            let option_value = option_value.clone();
                            let option_trigger_id = trigger_id.clone();
                            let is_selected = option_value == value;
                            let is_active = index == active_index();
                            let option_class = match (is_selected, is_active) {
                                (true, true) => "app-select-option app-select-option-selected app-select-option-active",
                                (true, false) => "app-select-option app-select-option-selected",
                                (false, true) => "app-select-option app-select-option-active",
                                (false, false) => "app-select-option",
                            };
                            rsx! {
                                button {
                                    id: "app-select-option-{instance_id}-{index}",
                                    r#type: "button",
                                    role: "option",
                                    tabindex: "-1",
                                    aria_selected: is_selected,
                                    class: "{option_class}",
                                    onclick: move |_| {
                                        on_change.call(option_value.clone());
                                        open.set(false);
                                        document::eval(&format!("document.getElementById('{option_trigger_id}')?.focus()"));
                                    },
                                    span { class: "min-w-0 whitespace-normal", "{label}" }
                                    if is_selected {
                                        span { class: "app-select-check", "✓" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn LanguageSelector(current_language: String, on_change: EventHandler<String>) -> Element {
    let options = i18n::available_languages()
        .iter()
        .map(|(code, name)| ((*code).to_string(), (*name).to_string()))
        .collect();
    rsx! {
        AppSelect {
            value: current_language,
            options,
            on_change,
            class: "settings-select",
        }
    }
}

#[component]
pub fn ThemeSelector(current_theme: String, on_change: EventHandler<String>) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let mut custom: Vec<(String, String)> = config
        .read()
        .custom_themes
        .iter()
        .map(|(id, ct)| (id.clone(), ct.name.clone()))
        .collect();
    custom.sort_by(|a, b| a.1.cmp(&b.1));
    let mut options = vec![
        ("album-art".into(), i18n::t("album_art_gradient")),
        ("default".into(), i18n::t("default_theme")),
        ("gruvbox".into(), i18n::t("gruvbox_material")),
        ("gruvbox-classic".into(), i18n::t("gruvbox_classic")),
        ("gruvbox-dark-soft".into(), i18n::t("gruvbox_dark_soft")),
        ("dracula".into(), i18n::t("dracula")),
        ("nord".into(), i18n::t("nord")),
        ("catppuccin".into(), i18n::t("catppuccin_mocha")),
        ("ef-night".into(), i18n::t("ef_night")),
        ("ayu-dark".into(), i18n::t("ayu_dark")),
        ("ayu-mirage".into(), i18n::t("ayu_mirage")),
        ("vague".into(), i18n::t("vague")),
        ("onedarkpro".into(), i18n::t("one_dark_pro")),
        ("osmium".into(), i18n::t("osmium")),
        ("kanagawa-dragon".into(), i18n::t("kanagawa_dragon")),
        ("everforest".into(), i18n::t("everforest")),
        ("rosepine".into(), i18n::t("rosepine")),
        ("kettek16".into(), "kettek16".into()),
        ("default-light".into(), i18n::t("default_light")),
        ("catppuccin-latte".into(), i18n::t("catppuccin_latte")),
        ("rosepine-dawn".into(), i18n::t("rosepine_dawn")),
        ("everforest-light".into(), i18n::t("everforest_light")),
        ("ayu-light".into(), i18n::t("ayu_light")),
        ("one-light".into(), i18n::t("one_light")),
        ("gruvbox-light".into(), i18n::t("gruvbox_light_soft")),
    ];
    options.extend(custom);

    rsx! {
        AppSelect {
            value: current_theme,
            options,
            on_change,
            class: "settings-select",
        }
    }
}

#[component]
pub fn MultiDirectoryPicker(
    current_paths: Vec<std::path::PathBuf>,
    on_add: EventHandler<std::path::PathBuf>,
    on_remove: EventHandler<usize>,
) -> Element {
    let add_text = i18n::t("add_folder");
    let remove_text = i18n::t("remove");
    let no_folders_text = i18n::t("no_music_folders");

    rsx! {
        div { class: "flex flex-col gap-2 w-full",
            if current_paths.is_empty() {
                p { class: "text-xs text-slate-500 italic", "{no_folders_text}" }
            }
            for (i, path) in current_paths.iter().enumerate() {
                {
                    let display = path.display().to_string();
                    let row_key = format!("{i}-{display}");
                    rsx! {
                        div { key: "{row_key}",
                            class: "flex items-center justify-between gap-3 bg-white/5 p-2 rounded w-full",
                            span {
                                class: "text-xs text-slate-400 font-mono truncate flex-1",
                                "{display}"
                            }
                            button {
                                onclick: move |_| {
                                    on_remove.call(i);
                                },
                                class: "text-red-400 hover:text-red-300 text-xs px-2 py-0.5 rounded transition-colors shrink-0",
                                "{remove_text}"
                            }
                        }
                    }
                }
            }
            AddFolderButton { on_add, add_text }
        }
    }
}

#[cfg(not(target_os = "android"))]
#[component]
fn AddFolderButton(on_add: EventHandler<std::path::PathBuf>, add_text: String) -> Element {
    rsx! {
        button {
            onclick: move |_| {
                spawn(async move {
                    if let Some(handle) = AsyncFileDialog::new().pick_folder().await {
                        on_add.call(handle.path().to_path_buf());
                    }
                });
            },
            class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
            "{add_text}"
        }
    }
}

// Android has no native folder dialog (rfd doesn't work), so request storage permission
// and auto-detect the system Music directory via JNI, falling back to common paths.
#[cfg(target_os = "android")]
#[component]
fn AddFolderButton(on_add: EventHandler<std::path::PathBuf>, add_text: String) -> Element {
    rsx! {
        button {
            onclick: move |_| {
                player::systemint::request_permissions();
                let mut paths = Vec::new();
                if let Some(android_music) = player::systemint::get_android_music_dir() {
                    paths.push(std::path::PathBuf::from(android_music));
                }
                paths.push(std::path::PathBuf::from("/storage/emulated/0/Music"));
                paths.push(std::path::PathBuf::from("/sdcard/Music"));
                if let Ok(home) = std::env::var("HOME") {
                    paths.push(std::path::PathBuf::from(home).join("Music"));
                }
                for path in paths {
                    if path.exists() {
                        on_add.call(path);
                        break;
                    }
                }
            },
            class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
            "{add_text}"
        }
    }
}

#[component]
pub fn ServerSettings(
    /// The active source's server id (`None` ⇒ Local) — the authoritative "which
    /// server is active", reactive to the sidebar source switcher too.
    active_source_id: Option<String>,
    servers: Vec<SavedServer>,
    on_add: EventHandler<()>,
    on_delete: EventHandler<String>,
    on_switch: EventHandler<String>,
    on_login: EventHandler<()>,
) -> Element {
    let login_text = i18n::t("login");
    let delete_text = i18n::t("delete");
    let switch_text = i18n::t("switch_to_server");
    let active_text = i18n::t("active_server");
    let conn = hooks::source_switch::use_connection_status();

    rsx! {
        div { class: "flex flex-col gap-2 w-full",
            if servers.is_empty() {
                p { class: "text-xs text-white/50 italic", "{i18n::t(\"no_saved_servers\")}" }
            }
            for srv in servers.iter().cloned() {
                {
                    let id = srv.id.clone();
                    let is_active = active_source_id.as_deref() == Some(srv.id.as_str());
                    let id_switch = id.clone();
                    let id_delete = id.clone();
                    rsx! {
                        div { key: "{srv.id}",
                            class: "flex items-center justify-between gap-4 bg-white/5 p-2 rounded w-full",
                            div { class: "min-w-0 flex-1",
                                div { class: "flex items-center gap-2",
                                    p { class: "text-sm font-medium text-white truncate", "{srv.name}" }
                                    if is_active {
                                        span { class: "text-[10px] px-2 py-0.5 rounded bg-indigo-500/30 text-indigo-200",
                                            "{active_text}"
                                        }
                                    }
                                }
                                p { class: "text-xs text-white/60", "{i18n::t_with(\"service\", &[(\"name\", srv.service.display_name().to_string())])}" }
                                p { class: "text-xs text-white/60 truncate", "{srv.url}" }
                                if is_active {
                                    match conn() {
                                        hooks::source_switch::ConnStatus::Online => rsx! {
                                            p { class: "text-xs mt-1", style: "color:#3fb950", "{i18n::t(\"connected\")}" }
                                        },
                                        hooks::source_switch::ConnStatus::Connecting => rsx! {
                                            p { class: "text-xs mt-1", style: "color:#d8a23a", "{i18n::t(\"connecting\")}" }
                                        },
                                        hooks::source_switch::ConnStatus::Offline => rsx! {
                                            div { class: "flex items-center gap-2 mt-1",
                                                p { class: "text-xs", style: "color:#e5534b", "{i18n::t(\"disconnected\")}" }
                                                button {
                                                    onclick: move |_| on_login.call(()),
                                                    class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-0.5 rounded text-white transition-colors",
                                                    "{login_text}"
                                                }
                                            }
                                        },
                                    }
                                }
                            }
                            div { class: "flex items-center gap-2 shrink-0",
                                if !is_active {
                                    button {
                                        onclick: move |_| on_switch.call(id_switch.clone()),
                                        class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-1 rounded text-white transition-colors",
                                        "{switch_text}"
                                    }
                                }
                                button {
                                    onclick: move |_| on_delete.call(id_delete.clone()),
                                    class: "text-red-400 hover:text-red-300 text-sm px-2 py-1 transition-colors",
                                    "{delete_text}"
                                }
                            }
                        }
                    }
                }
            }
            button {
                onclick: move |_| on_add.call(()),
                class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
                "{i18n::t(\"add_server\")}"
            }
        }
    }
}

#[component]
pub fn DiscordPresenceSettings(enabled: bool, on_change: EventHandler<bool>) -> Element {
    let slider_style = if enabled {
        "inset-inline-start: 4px; width: calc(50% - 4px);"
    } else {
        "inset-inline-start: calc(50% + 2px); width: calc(50% - 4px);"
    };

    let enable_class = if enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    let disable_class = if !enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    rsx! {
        div {
            class: "bg-white/5 p-1 rounded-xl flex relative h-10 items-center border border-white/5 w-48",
            div {
                class: "absolute h-8 bg-white/10 rounded-lg transition-all duration-300 ease-out",
                style: "{slider_style}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {enable_class}",
                onclick: move |_| on_change.call(true),
                "{i18n::t(\"enabled\")}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {disable_class}",
                onclick: move |_| on_change.call(false),
                "{i18n::t(\"disabled\")}"
            }
        }
    }
}

#[component]
pub fn DiscordPresencePausedSettings(enabled: bool, on_change: EventHandler<bool>) -> Element {
    let slider_style = if enabled {
        "inset-inline-start: 4px; width: calc(50% - 4px);"
    } else {
        "inset-inline-start: calc(50% + 2px); width: calc(50% - 4px);"
    };

    let enable_class = if enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    let disable_class = if !enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    rsx! {
        div {
            class: "bg-white/5 p-1 rounded-xl flex relative h-10 items-center border border-white/5 w-48",
            div {
                class: "absolute h-8 bg-white/10 rounded-lg transition-all duration-300 ease-out",
                style: "{slider_style}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {enable_class}",
                onclick: move |_| on_change.call(true),
                "{i18n::t(\"enabled\")}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {disable_class}",
                onclick: move |_| on_change.call(false),
                "{i18n::t(\"disabled\")}"
            }
        }
    }
}

#[component]
pub fn ToggleSetting(enabled: bool, on_change: EventHandler<bool>) -> Element {
    let slider_style = if enabled {
        "inset-inline-start: 4px; width: calc(50% - 4px);"
    } else {
        "inset-inline-start: calc(50% + 2px); width: calc(50% - 4px);"
    };

    let enable_class = if enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    let disable_class = if !enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    rsx! {
        div {
            class: "bg-white/5 p-1 rounded-xl flex relative h-10 items-center border border-white/5 w-48",
            div {
                class: "absolute h-8 bg-white/10 rounded-lg transition-all duration-300 ease-out",
                style: "{slider_style}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {enable_class}",
                onclick: move |_| on_change.call(true),
                "{i18n::t(\"enabled\")}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {disable_class}",
                onclick: move |_| on_change.call(false),
                "{i18n::t(\"disabled\")}"
            }
        }
    }
}

#[component]
pub fn MusicBrainzSettings(current: String, on_save: EventHandler<String>) -> Element {
    let mut input = use_signal(move || current.clone());

    rsx! {
        div {
            class: "flex items-center gap-2 w-full max-w-xl",
            div {
                class: "flex-1 bg-white/5 p-1 rounded-xl border border-white/5",
                input {
                    class: "bg-transparent w-full px-3 py-2 text-sm text-white placeholder:text-white/50 outline-none",
                    placeholder: "{i18n::t(\"listenbrainz_token_placeholder\")}",
                    value: "{input()}",
                    oninput: move |evt| {
                        input.set(evt.value());
                        on_save.call(evt.value());
                    },
                    r#type: "password",
                }
            }
        }
    }
}

#[component]
pub fn LastFmSettings(
    api_key: String,
    api_secret: String,
    session_key: String,
    on_api_key_save: EventHandler<String>,
    on_api_secret_save: EventHandler<String>,
    on_session_key_save: EventHandler<String>,
) -> Element {
    let mut api_key_input = use_signal(move || api_key.clone());
    let mut api_secret_input = use_signal(move || api_secret.clone());

    rsx! {
        div {
            class: "flex flex-col gap-3 w-full max-w-xl",
            div {
                class: "bg-white/5 p-1 rounded-xl border border-white/5",
                input {
                    class: "bg-transparent w-full px-3 py-2 text-sm text-white placeholder:text-white/50 outline-none",
                    placeholder: "{i18n::t(\"lastfm_api_key_placeholder\")}",
                    value: "{api_key_input()}",
                    oninput: move |evt| {
                        let value = evt.value();
                        api_key_input.set(value.clone());
                        on_api_key_save.call(value);
                        on_session_key_save.call(String::new());
                    },
                    r#type: "password",
                }
            }

            div {
                class: "bg-white/5 p-1 rounded-xl border border-white/5",

                input {
                    class: "bg-transparent w-full px-3 py-2 text-sm text-white placeholder:text-white/50 outline-none",
                    placeholder: "{i18n::t(\"lastfm_api_secret_placeholder\")}",
                    value: "{api_secret_input()}",
                    oninput: move |evt| {
                        api_secret_input.set(evt.value());
                        on_api_secret_save.call(evt.value());
                    },
                    r#type: "password",
                }
            }
            button {
                class: "bg-white/10 hover:bg-white/20 px-5 py-2 rounded text-sm text-white transition-colors self-start mx-auto w-fit",
                onclick: move |_| {
                    let api_key = api_key_input();
                    let api_secret = api_secret_input();
                    let on_session_key_save = on_session_key_save;

                    spawn(async move {
                        match lastfm::get_auth_token(&api_key).await {
                            Ok(token) => {
                                let url = lastfm::auth_url(&api_key, &token);

                                if let Err(e) = webbrowser::open(&url) {
                                    tracing::warn!("Failed to open browser: {}", e);
                                    return;
                                }
                                let mut connected = false;
                                for _ in 0..30 {
                                    match lastfm::get_session_key(&api_key, &api_secret, &token).await {
                                        Ok(session_key) => {
                                            on_session_key_save.call(session_key);
                                            tracing::info!("Last.fm connected successfully");
                                            connected = true;
                                            break;
                                        }
                                        Err(_) => {
                                            utils::sleep(std::time::Duration::from_secs(2)).await;
                                        }
                                    }
                                }
                            if !connected {
                                tracing::warn!("Timed out waiting for Last.fm authorization");
                            }

                            }
                            Err(e) => {
                                tracing::warn!("Failed to get auth token: {}", e);
                            }
                        }
                    }.instrument(tracing::info_span!("lastfm.auth")));
                },

                if session_key.is_empty() || api_key_input.is_empty() || api_secret_input.is_empty() {
                    "{i18n::t(\"connect_to_lastfm\")}"
                } else {
                    "{i18n::t(\"lastfm_connected\")}"
                }
            }
        }
    }
}

#[component]
pub fn LibreFmSettings(session_key: String, on_session_key_save: EventHandler<String>) -> Element {
    rsx! {
        div {
            class: "flex flex-col gap-3 w-full max-w-xl",
            button {
                class: "bg-white/10 hover:bg-white/20 px-5 py-2 rounded text-sm text-white transition-colors self-start mx-auto w-fit",
                onclick: move |_| {
                    let on_session_key_save = on_session_key_save;

                    spawn(async move {
                        match librefm::get_auth_token(librefm::API_KEY).await {
                            Ok(token) => {
                                let url = librefm::auth_url(librefm::API_KEY, &token);

                                if let Err(e) = webbrowser::open(&url) {
                                    tracing::warn!("Failed to open browser: {}", e);
                                    return;
                                }
                                let mut connected = false;
                                for _ in 0..30 {
                                    match librefm::get_session_key(
                                        librefm::API_KEY,
                                        librefm::API_SECRET,
                                        &token,
                                    )
                                    .await
                                    {
                                        Ok(session_key) => {
                                            on_session_key_save.call(session_key);
                                            tracing::info!("Libre.fm connected successfully");
                                            connected = true;
                                            break;
                                        }
                                        Err(_) => {
                                            utils::sleep(std::time::Duration::from_secs(2)).await;
                                        }
                                    }
                                }
                                if !connected {
                                    tracing::warn!("Timed out waiting for Libre.fm authorization");
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to get auth token: {}", e);
                            }
                        }
                    });
                },

                if session_key.is_empty() {
                    "{i18n::t(\"connect_to_librefm\")}"
                } else {
                    "{i18n::t(\"librefm_connected\")}"
                }
            }
        }
    }
}

const EQ_MIN_DB: f64 = -12.0;
const EQ_MAX_DB: f64 = 12.0;
const EQ_GRAPH_WIDTH: f64 = 1100.0;
const EQ_GRAPH_HEIGHT: f64 = 280.0;
const EQ_GRAPH_PAD_X: f64 = 40.0;
const EQ_GRAPH_PAD_TOP: f64 = 22.0;
const EQ_GRAPH_PAD_BOTTOM: f64 = 42.0;

fn eq_plot_width() -> f64 {
    EQ_GRAPH_WIDTH - EQ_GRAPH_PAD_X * 2.0
}

fn eq_plot_height() -> f64 {
    EQ_GRAPH_HEIGHT - EQ_GRAPH_PAD_TOP - EQ_GRAPH_PAD_BOTTOM
}

fn eq_band_x(index: usize, total: usize) -> f64 {
    let span = eq_plot_width();
    if total <= 1 {
        return EQ_GRAPH_PAD_X + span / 2.0;
    }
    EQ_GRAPH_PAD_X + (span * index as f64 / (total.saturating_sub(1)) as f64)
}

fn eq_gain_to_y(gain: f32) -> f64 {
    let ratio = (EQ_MAX_DB - gain as f64) / (EQ_MAX_DB - EQ_MIN_DB);
    EQ_GRAPH_PAD_TOP + ratio.clamp(0.0, 1.0) * eq_plot_height()
}

fn eq_y_to_gain(y: f64) -> f32 {
    let clamped = y.clamp(EQ_GRAPH_PAD_TOP, EQ_GRAPH_PAD_TOP + eq_plot_height());
    let ratio = 1.0 - ((clamped - EQ_GRAPH_PAD_TOP) / eq_plot_height().max(1.0));
    let gain = EQ_MIN_DB + ratio * (EQ_MAX_DB - EQ_MIN_DB);
    ((gain * 2.0).round() / 2.0) as f32
}

fn eq_nearest_band(x: f64, total: usize) -> usize {
    let mut nearest = 0usize;
    let mut distance = f64::MAX;
    for index in 0..total {
        let band_x = eq_band_x(index, total);
        let delta = (band_x - x).abs();
        if delta < distance {
            distance = delta;
            nearest = index;
        }
    }
    nearest
}

fn eq_apply_band_gain(base: &EqualizerConfig, index: usize, gain: f32) -> EqualizerConfig {
    let mut next = base.clone();
    let mut bands = base.resolved_bands();
    bands[index] = gain.clamp(EQ_MIN_DB as f32, EQ_MAX_DB as f32);
    next.bands = bands;
    next.preset = EqPreset::Custom;
    next
}

fn eq_apply_drag(base: &EqualizerConfig, index: usize, y: f64) -> EqualizerConfig {
    eq_apply_band_gain(base, index, eq_y_to_gain(y))
}

fn eq_interpolate_bands(from: [f32; 10], to: [f32; 10], progress: f32) -> [f32; 10] {
    std::array::from_fn(|index| from[index] + (to[index] - from[index]) * progress)
}

fn eq_drag_readout_position(index: usize, gain: f32, total: usize) -> (f64, f64) {
    let x = eq_band_x(index, total).clamp(76.0, EQ_GRAPH_WIDTH - 76.0);
    let y = (eq_gain_to_y(gain) - 30.0).clamp(18.0, EQ_GRAPH_HEIGHT - EQ_GRAPH_PAD_BOTTOM - 18.0);
    (x, y)
}

fn eq_preset_label(preset: EqPreset) -> String {
    match preset {
        EqPreset::Flat => i18n::t("eq_preset_flat"),
        EqPreset::BassBoost => i18n::t("eq_preset_bass_boost"),
        EqPreset::TrebleBoost => i18n::t("eq_preset_treble_boost"),
        EqPreset::VocalBoost => i18n::t("eq_preset_vocal_boost"),
        EqPreset::Loudness => i18n::t("eq_preset_loudness"),
        EqPreset::Custom => i18n::t("eq_preset_custom"),
    }
}

#[component]
pub fn EqualizerPanel(
    current: EqualizerConfig,
    on_preview: EventHandler<EqualizerConfig>,
    on_commit: EventHandler<EqualizerConfig>,
) -> Element {
    const BAND_LABELS: [&str; 10] = [
        "32 Hz", "64 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz",
        "16 kHz",
    ];

    let config = use_context::<Signal<AppConfig>>();
    let mut draft = use_signal(|| current.clone());
    let mut dragging_band = use_signal(|| None::<usize>);
    let mut hovered_band = use_signal(|| None::<usize>);
    let mut displayed_bands = use_signal(|| current.resolved_bands());
    let mut animation_token = use_signal(|| 0_u64);
    let reduce_animations = config.read().reduce_animations;
    let enabled = draft.read().enabled;
    let resolved_bands = *displayed_bands.read();
    let slider_style = if enabled {
        "inset-inline-start: 4px; width: calc(50% - 4px);"
    } else {
        "inset-inline-start: calc(50% + 2px); width: calc(50% - 4px);"
    };

    let enable_class = if enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    let disable_class = if !enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };
    let active_drag_band = *dragging_band.read();
    let active_hover_band = *hovered_band.read();
    let highlighted_band = active_drag_band.or(active_hover_band);
    let graph_class = if active_drag_band.is_some() {
        "block mx-auto cursor-grabbing"
    } else {
        "block mx-auto cursor-row-resize"
    };

    let graph_path = resolved_bands
        .iter()
        .enumerate()
        .map(|(index, gain)| {
            let command = if index == 0 { "M" } else { "L" };
            format!(
                "{command} {:.2} {:.2}",
                eq_band_x(index, BAND_LABELS.len()),
                eq_gain_to_y(*gain)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let graph_fill_path = format!(
        "{} L {:.2} {:.2} L {:.2} {:.2} Z",
        graph_path,
        eq_band_x(BAND_LABELS.len().saturating_sub(1), BAND_LABELS.len()),
        EQ_GRAPH_HEIGHT - EQ_GRAPH_PAD_BOTTOM,
        eq_band_x(0, BAND_LABELS.len()),
        EQ_GRAPH_HEIGHT - EQ_GRAPH_PAD_BOTTOM
    );
    let curve_fill_style = {
        let opacity = if enabled {
            if highlighted_band.is_some() {
                0.94
            } else {
                0.82
            }
        } else {
            0.22
        };

        if reduce_animations {
            format!("fill: url(#eq-curve-fill); opacity: {opacity:.2};")
        } else {
            format!(
                "fill: url(#eq-curve-fill); opacity: {opacity:.2}; transition: opacity 160ms ease-out;"
            )
        }
    };
    let curve_stroke_style = if enabled {
        if highlighted_band.is_some() {
            if reduce_animations {
                "stroke: var(--color-indigo-400);".to_string()
            } else {
                "stroke: var(--color-indigo-400); transition: stroke 140ms ease-out;".to_string()
            }
        } else if reduce_animations {
            "stroke: var(--color-indigo-500);".to_string()
        } else {
            "stroke: var(--color-indigo-500); transition: stroke 140ms ease-out;".to_string()
        }
    } else if reduce_animations {
        "stroke: color-mix(in oklab, var(--color-indigo-500) 52%, var(--color-slate-400));"
            .to_string()
    } else {
        "stroke: color-mix(in oklab, var(--color-indigo-500) 52%, var(--color-slate-400)); transition: stroke 180ms ease-out;"
            .to_string()
    };
    let preset_options = EqPreset::all()
        .into_iter()
        .map(|preset| (preset.as_storage().to_string(), eq_preset_label(preset)))
        .collect();

    rsx! {
        div { class: "flex flex-col gap-4 min-w-0 w-full",
            div { class: "grid grid-cols-1 md:grid-cols-2 xl:grid-cols-[12rem_15rem_minmax(16rem,1fr)] items-stretch gap-3",
                div {
                    class: "bg-white/5 p-1 rounded-xl flex relative min-h-10 items-center border border-white/5 w-full",
                    div {
                        class: "absolute h-8 bg-white/10 rounded-lg transition-all duration-300 ease-out",
                        style: "{slider_style}"
                    }
                    button {
                        class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {enable_class}",
                        onclick: move |_| {
                            let mut next = draft.peek().clone();
                            next.enabled = true;
                            draft.set(next.clone());
                            on_preview.call(next.clone());
                            on_commit.call(next);
                        },
                        "{i18n::t(\"enabled\")}"
                    }
                    button {
                        class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {disable_class}",
                        onclick: move |_| {
                            let mut next = draft.peek().clone();
                            next.enabled = false;
                            draft.set(next.clone());
                            on_preview.call(next.clone());
                            on_commit.call(next);
                        },
                        "{i18n::t(\"disabled\")}"
                    }
                }

                div { class: "flex min-w-0 items-center gap-2 bg-white/5 border border-white/10 rounded-xl px-3 py-2",
                    span { class: "text-xs text-slate-400", "{i18n::t(\"eq_preset\")}" }
                    AppSelect {
                        class: "min-w-0 flex-1",
                        value: draft.read().preset.as_storage().to_string(),
                        options: preset_options,
                        on_change: move |value: String| {
                            let mut next = draft.peek().clone();
                            let preset = EqPreset::from_storage(&value);
                            let previous_bands = *displayed_bands.peek();
                            next.preset = preset;
                            if let Some(default_preamp_db) = preset.default_preamp_db() {
                                next.preamp_db = default_preamp_db;
                            }
                            let next_bands = next.resolved_bands();
                            draft.set(next.clone());
                            let token = *animation_token.read() + 1;
                            animation_token.set(token);
                            if reduce_animations {
                                displayed_bands.set(next_bands);
                            } else {
                                spawn(async move {
                                    const STEPS: u32 = 10;
                                    const FRAME_MS: u64 = 18;
                                    for step in 1..=STEPS {
                                        if *animation_token.read() != token {
                                            return;
                                        }
                                        let progress = step as f32 / STEPS as f32;
                                        displayed_bands.set(eq_interpolate_bands(
                                            previous_bands,
                                            next_bands,
                                            progress,
                                        ));
                                        if step < STEPS {
                                            utils::sleep(std::time::Duration::from_millis(FRAME_MS)).await;
                                        }
                                    }
                                });
                            }
                            on_preview.call(next.clone());
                            on_commit.call(next);
                        },
                    }
                }

                div { class: "flex min-w-0 items-center gap-3 bg-white/5 border border-white/10 rounded-xl px-3 py-2 md:col-span-2 xl:col-span-1",
                    div { class: "min-w-0",
                        p { class: "text-xs text-slate-400", "{i18n::t(\"eq_preamp\")}" }
                        p { class: "text-[11px] text-slate-500", "{i18n::t(\"eq_preamp_desc\")}" }
                    }
                    input {
                        r#type: "range",
                        min: "-12",
                        max: "6",
                        step: "0.5",
                        value: format!("{:.1}", draft.read().preamp_db),
                        class: "flex-1",
                        style: "accent-color: var(--color-indigo-500);",
                        oninput: move |evt| {
                            if let Ok(value) = evt.value().parse::<f32>() {
                                let mut next = draft.peek().clone();
                                next.preamp_db = value;
                                draft.set(next.clone());
                                on_preview.call(next);
                            }
                        },
                        onchange: move |evt| {
                            if let Ok(value) = evt.value().parse::<f32>() {
                                let mut next = draft.peek().clone();
                                next.preamp_db = value;
                                draft.set(next.clone());
                                on_commit.call(next);
                            }
                        }
                    }
                    span { class: "text-xs font-mono text-white/80 w-14 text-right", {format!("{:+.1} dB", draft.read().preamp_db)} }
                }
            }

            p { class: "text-xs text-slate-500", "{i18n::t(\"eq_graph_hint\")}" }

            div {
                class: "rounded-lg border border-white/8 bg-white/5 p-4 select-none overflow-x-auto",
                style: "background: color-mix(in oklab, var(--color-neutral-900) 78%, transparent); border-color: color-mix(in oklab, var(--color-white) 8%, transparent);",
                svg {
                    class: "{graph_class}",
                    style: "width: 100%; height: auto; min-width: 680px; aspect-ratio: 1100 / 280;",
                    view_box: "0 0 1100 280",
                    onmousedown: move |evt: MouseEvent| {
                        let point = evt.element_coordinates();
                        let index = eq_nearest_band(point.x, BAND_LABELS.len());
                        dragging_band.set(Some(index));
                        hovered_band.set(Some(index));
                        let next = eq_apply_drag(&draft.peek().clone(), index, point.y);
                        draft.set(next.clone());
                        let token = *animation_token.read() + 1;
                        animation_token.set(token);
                        displayed_bands.set(next.resolved_bands());
                        on_preview.call(next);
                    },
                    onmousemove: move |evt: MouseEvent| {
                        let point = evt.element_coordinates();
                        let index = eq_nearest_band(point.x, BAND_LABELS.len());
                        hovered_band.set(Some(index));
                        if let Some(index) = *dragging_band.read() {
                            let next = eq_apply_drag(&draft.peek().clone(), index, point.y);
                            draft.set(next.clone());
                            displayed_bands.set(next.resolved_bands());
                            on_preview.call(next);
                        }
                    },
                    onmouseup: move |_| {
                        if dragging_band.peek().is_some() {
                            on_commit.call(draft.peek().clone());
                        }
                        dragging_band.set(None);
                        hovered_band.set(None);
                    },
                    onmouseleave: move |_| {
                        if dragging_band.peek().is_some() {
                            on_commit.call(draft.peek().clone());
                        }
                        dragging_band.set(None);
                        hovered_band.set(None);
                    },
                    defs {
                        linearGradient {
                            id: "eq-curve-fill",
                            x1: "0",
                            y1: "0",
                            x2: "0",
                            y2: "1",
                            stop {
                                offset: "0%",
                                style: "stop-color: color-mix(in oklab, var(--color-indigo-400) 34%, transparent); stop-opacity: 1;",
                            }
                            stop {
                                offset: "100%",
                                style: "stop-color: color-mix(in oklab, var(--color-indigo-500) 3%, transparent); stop-opacity: 1;",
                            }
                        }
                    }
                    for db in [-12.0_f64, -6.0, 0.0, 6.0, 12.0] {
                        line {
                            x1: "{EQ_GRAPH_PAD_X}",
                            x2: "{EQ_GRAPH_WIDTH - EQ_GRAPH_PAD_X}",
                            y1: "{eq_gain_to_y(db as f32)}",
                            y2: "{eq_gain_to_y(db as f32)}",
                            stroke_width: if db == 0.0 { "1.5" } else { "1" },
                            stroke_dasharray: if db == 0.0 { "0" } else { "4 6" },
                            style: if db == 0.0 {
                                "stroke: color-mix(in oklab, var(--color-white) 22%, transparent);"
                            } else {
                                "stroke: color-mix(in oklab, var(--color-slate-400) 16%, transparent);"
                            },
                        }
                        text {
                            x: "10",
                            y: "{eq_gain_to_y(db as f32) + 4.0}",
                            font_size: "10",
                            font_family: "JetBrains Mono, monospace",
                            style: "fill: color-mix(in oklab, var(--color-slate-400) 72%, transparent);",
                            {format!("{:+.0}", db)}
                        }
                    }
                    for (index, label) in BAND_LABELS.iter().enumerate() {
                        line {
                            x1: "{eq_band_x(index, BAND_LABELS.len())}",
                            x2: "{eq_band_x(index, BAND_LABELS.len())}",
                            y1: "{EQ_GRAPH_PAD_TOP}",
                            y2: "{EQ_GRAPH_HEIGHT - EQ_GRAPH_PAD_BOTTOM}",
                            stroke_width: "1",
                            style: "stroke: color-mix(in oklab, var(--color-slate-500) 34%, transparent);",
                        }
                        text {
                            x: "{eq_band_x(index, BAND_LABELS.len())}",
                            y: "{EQ_GRAPH_HEIGHT - 14.0}",
                            text_anchor: "middle",
                            font_size: "11",
                            font_family: "JetBrains Mono, monospace",
                            style: "fill: color-mix(in oklab, var(--color-white) 58%, transparent);",
                            "{label}"
                        }
                    }
                    path {
                        d: "{graph_fill_path}",
                        style: "{curve_fill_style}",
                    }
                    if let Some(index) = highlighted_band {
                        line {
                            x1: "{eq_band_x(index, BAND_LABELS.len())}",
                            x2: "{eq_band_x(index, BAND_LABELS.len())}",
                            y1: "{EQ_GRAPH_PAD_TOP}",
                            y2: "{EQ_GRAPH_HEIGHT - EQ_GRAPH_PAD_BOTTOM}",
                            stroke_width: "1.5",
                            style: if reduce_animations {
                                "stroke: color-mix(in oklab, var(--color-indigo-400) 34%, transparent);"
                            } else {
                                "stroke: color-mix(in oklab, var(--color-indigo-400) 34%, transparent); transition: stroke 140ms ease-out;"
                            },
                        }
                    }
                    path {
                        d: "{graph_path}",
                        fill: "none",
                        stroke_width: "2.5",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        style: "{curve_stroke_style}",
                    }
                    for (index, gain) in resolved_bands.iter().enumerate() {
                        {
                            let is_highlighted = highlighted_band == Some(index);
                            rsx! {
                                circle {
                                    cx: "{eq_band_x(index, BAND_LABELS.len())}",
                                    cy: "{eq_gain_to_y(*gain)}",
                                    r: if active_drag_band == Some(index) {
                                        "8"
                                    } else if is_highlighted {
                                        "7"
                                    } else {
                                        "6"
                                    },
                                    style: if active_drag_band == Some(index) {
                                        if reduce_animations {
                                            "fill: var(--color-indigo-400);"
                                        } else {
                                            "fill: var(--color-indigo-400); transition: r 140ms ease-out, fill 140ms ease-out;"
                                        }
                                    } else if is_highlighted {
                                        if reduce_animations {
                                            "fill: var(--color-indigo-400);"
                                        } else {
                                            "fill: var(--color-indigo-400); transition: r 140ms ease-out, fill 140ms ease-out;"
                                        }
                                    } else if reduce_animations {
                                        "fill: var(--color-white);"
                                    } else {
                                        "fill: var(--color-white); transition: r 140ms ease-out, fill 140ms ease-out;"
                                    },
                                }
                                circle {
                                    cx: "{eq_band_x(index, BAND_LABELS.len())}",
                                    cy: "{eq_gain_to_y(*gain)}",
                                    r: if is_highlighted { "16" } else { "14" },
                                    fill: "transparent",
                                    stroke_width: "1",
                                    style: if active_drag_band == Some(index) {
                                        if reduce_animations {
                                            "stroke: color-mix(in oklab, var(--color-indigo-400) 40%, transparent);"
                                        } else {
                                            "stroke: color-mix(in oklab, var(--color-indigo-400) 40%, transparent); transition: r 140ms ease-out, stroke 140ms ease-out;"
                                        }
                                    } else if is_highlighted {
                                        if reduce_animations {
                                            "stroke: color-mix(in oklab, var(--color-indigo-400) 28%, transparent);"
                                        } else {
                                            "stroke: color-mix(in oklab, var(--color-indigo-400) 28%, transparent); transition: r 140ms ease-out, stroke 140ms ease-out;"
                                        }
                                    } else if reduce_animations {
                                        "stroke: color-mix(in oklab, var(--color-white) 10%, transparent);"
                                    } else {
                                        "stroke: color-mix(in oklab, var(--color-white) 10%, transparent); transition: r 140ms ease-out, stroke 140ms ease-out;"
                                    },
                                }
                            }
                        }
                    }
                    if let Some(index) = active_drag_band {
                        {
                            let gain = resolved_bands[index];
                            let (tooltip_x, tooltip_y) =
                                eq_drag_readout_position(index, gain, BAND_LABELS.len());
                            rsx! {
                                rect {
                                    x: "{tooltip_x - 34.0}",
                                    y: "{tooltip_y - 12.0}",
                                    rx: "10",
                                    ry: "10",
                                    width: "68",
                                    height: "24",
                                    style: "fill: color-mix(in oklab, var(--color-neutral-900) 92%, transparent); stroke: color-mix(in oklab, var(--color-indigo-400) 26%, transparent);",
                                    stroke_width: "1",
                                }
                                text {
                                    x: "{tooltip_x}",
                                    y: "{tooltip_y + 3.5}",
                                    text_anchor: "middle",
                                    font_size: "11",
                                    font_family: "JetBrains Mono, monospace",
                                    font_weight: "700",
                                    style: "fill: var(--color-white);",
                                    {format!("{gain:+.1} dB")}
                                }
                            }
                        }
                    }
                }

            }

        }
    }
}

#[component]
pub fn BackBehaviorSelector(
    current: BackBehavior,
    on_change: EventHandler<BackBehavior>,
) -> Element {
    let is_rewind = current == BackBehavior::RewindThenPrev;

    let slider_style = if is_rewind {
        "inset-inline-start: 4px; width: calc(50% - 4px);"
    } else {
        "inset-inline-start: calc(50% + 2px); width: calc(50% - 4px);"
    };

    let rewind_class = if is_rewind {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    let always_class = if !is_rewind {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    rsx! {
        div {
            class: "bg-white/5 p-1 rounded-xl flex relative h-10 items-center border border-white/5 w-48",
            div {
                class: "absolute h-8 bg-white/10 rounded-lg transition-all duration-300 ease-out",
                style: "{slider_style}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {rewind_class}",
                title: "{i18n::t(\"back_behavior_rewind\")}",
                onclick: move |_| on_change.call(BackBehavior::RewindThenPrev),
                "{i18n::t(\"back_behavior_rewind\")}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {always_class}",
                title: "{i18n::t(\"back_behavior_always_prev\")}",
                onclick: move |_| on_change.call(BackBehavior::AlwaysPrev),
                "{i18n::t(\"back_behavior_always_prev\")}"
            }
        }
    }
}

fn channel_mode_label(mode: ChannelMode) -> String {
    match mode {
        ChannelMode::Stereo => i18n::t("channel_mode_stereo"),
        ChannelMode::Mono => i18n::t("channel_mode_mono"),
        ChannelMode::LeftOnly => i18n::t("channel_mode_left_only"),
        ChannelMode::RightOnly => i18n::t("channel_mode_right_only"),
        ChannelMode::SwapLeftRight => i18n::t("channel_mode_swap_left_right"),
    }
}

#[component]
pub fn ChannelModeSelector(current: ChannelMode, on_change: EventHandler<ChannelMode>) -> Element {
    let options = ChannelMode::ALL
        .iter()
        .map(|mode| (mode.value_str().to_string(), channel_mode_label(*mode)))
        .collect();
    rsx! {
        AppSelect {
            value: current.value_str().to_string(),
            options,
            on_change: move |value: String| on_change.call(ChannelMode::from_value_str(&value)),
            class: "settings-select",
        }
    }
}

fn sample_rate_mode_label(mode: SampleRateMode) -> String {
    match mode {
        SampleRateMode::System => i18n::t("sample_rate_mode_system"),
        SampleRateMode::Source => i18n::t("sample_rate_mode_source"),
    }
}

#[component]
pub fn SampleRateModeSelector(
    current: SampleRateMode,
    on_change: EventHandler<SampleRateMode>,
) -> Element {
    let options = SampleRateMode::ALL
        .iter()
        .map(|mode| (mode.value_str().to_string(), sample_rate_mode_label(*mode)))
        .collect();
    rsx! {
        AppSelect {
            value: current.value_str().to_string(),
            options,
            on_change: move |value: String| on_change.call(SampleRateMode::from_value_str(&value)),
            class: "settings-select",
        }
    }
}

fn device_change_behavior_label(behavior: DeviceChangeBehavior) -> String {
    match behavior {
        DeviceChangeBehavior::Resume => i18n::t("device_change_resume"),
        DeviceChangeBehavior::Pause => i18n::t("device_change_pause"),
    }
}

#[component]
pub fn DeviceChangeBehaviorSelector(
    current: DeviceChangeBehavior,
    on_change: EventHandler<DeviceChangeBehavior>,
) -> Element {
    let options = DeviceChangeBehavior::ALL
        .iter()
        .map(|behavior| {
            (
                behavior.value_str().to_string(),
                device_change_behavior_label(*behavior),
            )
        })
        .collect();
    rsx! {
        AppSelect {
            value: current.value_str().to_string(),
            options,
            on_change: move |value: String| {
                on_change.call(DeviceChangeBehavior::from_value_str(&value))
            },
            class: "settings-select",
        }
    }
}

#[component]
pub fn RadioRegistryDropdown(
    registries: Vec<config::RegistryEntry>,
    on_toggle: EventHandler<usize>,
    on_add: EventHandler<()>,
    on_delete: EventHandler<usize>,
    error: Signal<Option<String>>,
) -> Element {
    let mut expanded = use_signal(|| false);
    let is_open = expanded();
    let add_text = i18n::t("add");
    let delete_text = i18n::t("delete");
    let default_registry = i18n::t("radio_default_registry");
    rsx! {
        div { class: "settings-row flex flex-col w-full px-5",
            button {
                r#type: "button",
                class: "flex min-h-[3.25rem] items-center justify-between gap-4 w-full cursor-pointer group text-left",
                aria_expanded: is_open,
                onclick: move |_| expanded.set(!is_open),
                div { class: "flex items-center gap-2",
                    span { class: "text-sm text-white/90 font-medium", "{i18n::t(\"radio\")}" }
                    span {
                        class: "text-xs text-slate-500",
                        {
                            let enabled_count = registries.iter().filter(|r| r.enabled).count();
                            let total = registries.len();
                            i18n::t_with("radio_registries_active", &[("enabled_count", enabled_count.to_string()), ("total", total.to_string())])
                        }
                    }
                }
                i { class: if is_open { "fa-solid fa-chevron-up text-[10px] text-white/40" } else { "fa-solid fa-chevron-down text-[10px] text-white/40" } }
            }
            // Expandable panel
            if is_open {
                div { class: "flex flex-col gap-2 pb-3",
                    if registries.is_empty() {
                        p { class: "text-xs text-slate-500 italic py-1", "{i18n::t(\"radio_registries_empty\")}" }
                    }
                    if let Some(err) = error() {
                        p { class: "text-xs text-red-400 py-1 mb-1", "{err}" }
                    }
                    for (i, entry) in registries.iter().enumerate() {
                        {
                            let url_display = if entry.is_default {
                                default_registry.to_string()
                            } else {
                                entry.url.clone()
                            };
                            let row_key = format!("{i}-{}", entry.url);
                            let is_default = entry.is_default;
                            let is_enabled = entry.enabled;
                            rsx! {
                                div { key: "{row_key}",
                                    class: "flex items-center gap-3 bg-white/5 p-2 rounded w-full",
                                    input {
                                        r#type: "checkbox",
                                        checked: is_enabled,
                                        onchange: move |_| on_toggle.call(i),
                                        class: "accent-indigo-500 w-4 h-4 shrink-0 cursor-pointer",
                                    }
                                    span {
                                        class: if is_enabled {
                                            "text-xs text-slate-300 font-mono truncate flex-1"
                                        } else {
                                            "text-xs text-slate-600 font-mono truncate flex-1 line-through"
                                        },
                                        "{url_display}"
                                    }
                                    if !is_default {
                                        button {
                                            onclick: move |_| on_delete.call(i),
                                            class: "text-red-400 hover:text-red-300 text-xs px-2 py-0.5 rounded transition-colors shrink-0",
                                            "{delete_text}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    button {
                        onclick: move |_| on_add.call(()),
                        class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start mt-1",
                        "{add_text}"
                    }
                }
            }
        }
    }
}
