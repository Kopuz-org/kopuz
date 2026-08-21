use config::{AppConfig, BackBehavior, ChannelMode, DeviceChangeBehavior, SampleRateMode};
use dioxus::prelude::*;
use scrobble::lastfm;
use scrobble::librefm;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::Instrument;

static APP_SELECT_ID: AtomicUsize = AtomicUsize::new(0);

#[component]
pub fn SettingItem(
    title: String,
    control: Element,
    /// The top-level `AppConfig` field this row edits. When the field is
    /// pinned by a managed config layer (Nix/hjem file, drop-in, env var),
    /// the row renders locked. Empty = never locked.
    #[props(default)]
    config_key: String,
    /// The other top-level fields a composite control edits (the Last.fm
    /// credentials, say). The row locks when *any* of its keys is managed —
    /// locking on `config_key` alone would leave the rest editable.
    #[props(default)]
    extra_config_keys: Vec<&'static str>,
    /// Put the control below the title at full width instead of beside it —
    /// for controls too wide for a row (the equalizer graph).
    #[props(default)]
    stacked: bool,
) -> Element {
    let locked = try_consume_context::<config::store::FileLayers>().is_some_and(|layers| {
        (!config_key.is_empty() && layers.is_locked(&config_key))
            || extra_config_keys.iter().any(|key| layers.is_locked(key))
    });
    rsx! {
        div {
            class: if stacked { "settings-row px-5 py-4" } else { "settings-row flex items-center justify-between gap-5 px-5 py-2.5" },
            div {
                class: if stacked { "flex items-center gap-2 mb-3" } else { "min-w-0 flex items-center gap-2" },
                p { class: "min-w-0 text-sm text-white/90 font-medium", "{title}" }
                if locked {
                    i {
                        class: "fa-solid fa-lock text-[10px] text-white/40",
                        title: i18n::t("setting_managed_by_system"),
                    }
                }
            }
            if locked {
                // `inert` is what actually disables the control: it takes the
                // whole subtree out of the tab order and drops click/key
                // events, so a managed setting can't be changed by keyboard
                // either. The class/aria only cover the look and the a11y
                // label.
                div {
                    class: "opacity-50 pointer-events-none select-none",
                    inert: true,
                    aria_disabled: true,
                    title: i18n::t("setting_managed_by_system"),
                    {control}
                }
            } else {
                {control}
            }
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
        ("amoled".into(), i18n::t("amoled_black")),
        (utils::live_theme::THEME_ID.into(), i18n::t("live_theme")),
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
    // Android has no palette control, since choosing one opens a native file
    // dialog, so don't offer a theme it can't point at anything.
    if cfg!(target_os = "android") {
        options.retain(|(id, _)| id != utils::live_theme::THEME_ID);
    }
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

#[path = "sources.rs"]
mod sources;
pub use sources::{LocalSourceSettings, MultiDirectoryPicker, ServerSettings};
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

#[path = "equalizer.rs"]
mod equalizer;
pub use equalizer::EqualizerPanel;
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
