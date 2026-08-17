use crate::settings_items::MultiDirectoryPicker;
use config::{Browser, MusicService};
use dioxus::prelude::*;

#[component]
pub fn AddLocalSourcePopup(
    name: Signal<String>,
    directories: Signal<Vec<std::path::PathBuf>>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "overlay", onclick: move |_| on_close.call(()),
            div { class: "popup", onclick: |e| e.stop_propagation(),
                h2 { "{i18n::t(\"add_local_library\")}" }
                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }
                input {
                    placeholder: "{i18n::t(\"local_library_name\")}",
                    value: "{name()}",
                    oninput: move |e| name.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                }
                MultiDirectoryPicker {
                    current_paths: directories(),
                    on_add: move |path| {
                        if !directories.peek().contains(&path) {
                            directories.write().push(path);
                        }
                    },
                    on_remove: move |index| {
                        if index < directories.peek().len() {
                            directories.write().remove(index);
                        }
                    },
                }
                div { class: "actions",
                    button { onclick: move |_| on_close.call(()), "{i18n::t(\"cancel\")}" }
                    button { onclick: move |_| on_save.call(()), "{i18n::t(\"save\")}" }
                }
            }
        }
    }
}

/// The built-in services, in dropdown order: option value, enum, label key.
/// `None` for the label key means the label is the service's own display name.
const BUILT_IN_SERVICES: &[(&str, MusicService, Option<&str>)] = &[
    ("jellyfin", MusicService::Jellyfin, Some("jellyfin")),
    ("subsonic", MusicService::Subsonic, Some("subsonic")),
    ("custom", MusicService::Custom, Some("custom_manual")),
    ("ytmusic", MusicService::YtMusic, None),
    ("soundcloud", MusicService::SoundCloud, None),
    ("spotify", MusicService::Spotify, None),
];

#[component]
pub fn AddServerPopup(
    server_name: Signal<String>,
    server_url: Signal<String>,
    server_service: Signal<MusicService>,
    /// The selected plugin when `server_service` is `Plugin`.
    plugin_id: Signal<Option<String>>,
    /// Selected Chromium-family browser when service is YouTube Music.
    yt_browser: Signal<Browser>,
    /// YouTube Music anonymous mode — true = no sign-in, browse + play
    /// public surfaces only.
    yt_anonymous: Signal<bool>,
    host_access: Signal<bool>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    // Discovered plugins join the dropdown as data. Nothing here names a
    // provider — a plugin's entry is its manifest's own id and name.
    let plugins = use_hook(|| {
        ::server::registry()
            .manifests()
            .into_iter()
            .map(|m| (m.id, m.name))
            .collect::<Vec<_>>()
    });
    let selected_value = match (server_service(), plugin_id()) {
        (MusicService::Plugin, Some(id)) => format!("plugin:{id}"),
        (MusicService::Plugin, None) => String::new(),
        (service, _) => BUILT_IN_SERVICES
            .iter()
            .find(|(_, s, _)| *s == service)
            .map(|(value, ..)| (*value).to_string())
            .unwrap_or_default(),
    };

    let server_name_label = i18n::t("server_name").to_string();
    let server_url_placeholder = i18n::t("server_url_placeholder").to_string();
    let custom_manual = i18n::t("custom_manual").to_string();
    let cancel_text = i18n::t("cancel").to_string();
    let save_text = i18n::t("save").to_string();

    let saving_not_supported = {
        let service = server_service();
        !host_access()
            && service.uses_browser_signin()
            && (service != MusicService::YtMusic || !yt_anonymous())
    };

    let flatpak_access_command =
        "flatpak override --user --talk-name=org.freedesktop.Flatpak moe.kopuz.kopuz";

    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "{i18n::t(\"add_media_server\")}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                if saving_not_supported {
                    div { class: "warning",
                        p {  "Browser Sign-in requires access to host.",  br {}, "Run the command below and restart kopuz to provide access." }
                        button {
                            class: "flatpak-command",
                            title: "Click to copy",
                            aria_label: "Click to copy",
                            onclick: move |_| {
                            let js = format!(
                                "navigator.clipboard.writeText('{flatpak_access_command}').catch((e) => console.error('clipboard writeText failed', e));"
                                );
                            let _ = dioxus::document::eval(&js);
                            },
                            "{flatpak_access_command}"
                        }
                    }
                }


                input {
                    placeholder: "{server_name_label}",
                    value: "{server_name()}",
                    oninput: move |e| server_name.set(e.value()),
                    onkeydown: move |e| e.stop_propagation()
                }

                ServerServiceFields {
                    server_service,
                    server_url,
                    yt_browser,
                    yt_anonymous,
                    server_url_placeholder: server_url_placeholder.clone(),
                }

                select {
                    onchange: move |e| {
                        let value = e.value();
                        match value.strip_prefix("plugin:") {
                            Some(id) => {
                                server_service.set(MusicService::Plugin);
                                plugin_id.set(Some(id.to_string()));
                            }
                            None => {
                                plugin_id.set(None);
                                server_service
                                    .set(
                                        BUILT_IN_SERVICES
                                            .iter()
                                            .find(|(v, ..)| *v == value)
                                            .map(|(_, service, _)| *service)
                                            .unwrap_or(MusicService::Jellyfin),
                                    );
                            }
                        }
                    },
                    onkeydown: move |e| e.stop_propagation(),
                    for (value , service , label_key) in BUILT_IN_SERVICES.iter().copied() {
                        option {
                            key: "{value}",
                            value: "{value}",
                            selected: selected_value == value,
                            {
                                match label_key {
                                    Some("custom_manual") => custom_manual.clone(),
                                    Some(key) => i18n::t(key).to_string(),
                                    None => service.display_name().to_string(),
                                }
                            }
                        }
                    }
                    for (id , name) in plugins.iter() {
                        option {
                            key: "plugin:{id}",
                            value: "plugin:{id}",
                            selected: selected_value == format!("plugin:{id}"),
                            "{name}"
                        }
                    }
                }

                div { class: "actions",
                    button {
                        onclick: move |_| on_close.call(()),
                        "{cancel_text}"
                    }
                    button {
                        disabled: saving_not_supported,
                        onclick: move |_| on_save.call(()),
                        "{save_text}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn LoginPopup(
    username: Signal<String>,
    password: Signal<String>,
    service_name: String,
    error: Signal<Option<String>>,
    loading: Signal<bool>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    let cancel_text = i18n::t("cancel").to_string();
    let login_text = i18n::t("login").to_string();
    let username_placeholder = i18n::t("username").to_string();
    let password_placeholder = i18n::t("password").to_string();
    let login_to_service_text =
        i18n::t_with("login_to_service", &[("service", service_name.clone())]);

    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "{login_to_service_text}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "{username_placeholder}",
                    value: "{username()}",
                    oninput: move |e| username.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                    disabled: loading()
                }

                input {
                    r#type: "password",
                    placeholder: "{password_placeholder}",
                    value: "{password()}",
                    oninput: move |e| password.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                    disabled: loading()
                }

                div { class: "actions",
                    button {
                        onclick: move |_| if !loading() { on_close.call(()) },
                        disabled: loading(),
                        "{cancel_text}"
                    }
                    button {
                        onclick: move |_| if !loading() { on_save.call(()) },
                        disabled: loading(),
                        if loading() { "{i18n::t(\"logging_in\")}" } else { "{login_text}" }
                    }
                }
            }
        }
    }
}

#[component]
pub fn AddRegistryPopup(
    registry_url: Signal<String>,
    error: Signal<Option<String>>,
    loading: Signal<bool>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    let url_placeholder = i18n::t("radio_registry_url_placeholder").to_string();
    let cancel_text = i18n::t("cancel").to_string();
    let save_text = i18n::t("save").to_string();

    rsx! {
        div {
            class: "overlay",
            onclick: move |_| { if !loading() { on_close.call(()) } },

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "{i18n::t(\"add_radio_registry\")}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "{url_placeholder}",
                    value: "{registry_url()}",
                    oninput: move |e| registry_url.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                    disabled: loading()
                }

                div { class: "actions",
                    button {
                        onclick: move |_| if !loading() { on_close.call(()) },
                        disabled: loading(),
                        "{cancel_text}"
                    }
                    button {
                        onclick: move |_| if !loading() { on_save.call(()) },
                        disabled: loading(),
                        if loading() { "{i18n::t(\"saving\")}" } else { "{save_text}" }
                    }
                }
            }
        }
    }
}

#[component]
fn ServerServiceFields(
    server_service: Signal<MusicService>,
    server_url: Signal<String>,
    yt_browser: Signal<Browser>,
    yt_anonymous: Signal<bool>,
    server_url_placeholder: String,
) -> Element {
    match server_service() {
        MusicService::YtMusic => {
            let anon = yt_anonymous();
            rsx! {
                // Auth method selector.
                div { class: "flex flex-col gap-2 mb-2",
                    label { class: "flex items-center gap-2 text-sm text-white cursor-pointer",
                        input {
                            r#type: "radio",
                            name: "yt-auth-method",
                            checked: !anon,
                            onchange: move |_| yt_anonymous.set(false),
                        }
                        span { "Sign in with a browser" }
                    }
                    label { class: "flex items-center gap-2 text-sm text-white cursor-pointer",
                        input {
                            r#type: "radio",
                            name: "yt-auth-method",
                            checked: anon,
                            onchange: move |_| yt_anonymous.set(true),
                        }
                        span { "Continue without signing in (anonymous)" }
                    }
                }

                if anon {
                    p { class: "text-xs text-white/60",
                        "kopuz will use YouTube Music without signing in. You can browse, search, and play — but Liked Music, your library playlists, and following/liking are disabled."
                    }
                } else {
                    p { class: "text-xs text-white/60",
                        "Pick which browser kopuz should use for the YouTube Music sign-in window. It opens in an isolated profile (a fresh, separate session) — your normal browsing is untouched. Make sure the browser is installed."
                    }
                    // Windows Chrome keeps the auth cookies in memory until the
                    // browser closes, so kopuz can only read them after close.
                    if cfg!(target_os = "windows") {
                        p { class: "text-xs text-amber-300/90 mt-1",
                            "After you finish signing in, close the browser window — kopuz completes sign-in once it does."
                        }
                    }
                    select {
                        onchange: move |e| {
                            if let Some(b) = Browser::from_id(&e.value()) {
                                yt_browser.set(b);
                            }
                        },
                        onkeydown: move |e| e.stop_propagation(),
                        for browser in Browser::ALL.iter().copied() {
                            option {
                                value: "{browser.id()}",
                                selected: yt_browser() == browser,
                                "{browser.label()}"
                            }
                        }
                    }
                }
            }
        }
        MusicService::SoundCloud => rsx! {
            // SoundCloud is browser sign-in only (no URL); pick the browser for
            // the isolated sign-in window.
            p { class: "text-xs text-white/60",
                "Pick which browser kopuz should use for the SoundCloud sign-in window. It opens in an isolated profile (a fresh, separate session) — your normal browsing is untouched. Make sure the browser is installed."
            }
            select {
                onchange: move |e| {
                    if let Some(b) = Browser::from_id(&e.value()) {
                        yt_browser.set(b);
                    }
                },
                onkeydown: move |e| e.stop_propagation(),
                for browser in Browser::ALL.iter().copied() {
                    option {
                        value: "{browser.id()}",
                        selected: yt_browser() == browser,
                        "{browser.label()}"
                    }
                }
            }
        },
        // A plugin collects whatever it needs in its own wizard, so there is
        // nothing for Kopuz to ask here.
        MusicService::Plugin => rsx! {
            p { class: "text-xs text-white/60", "{i18n::t(\"plugin_no_setup\")}" }
        },
        MusicService::Spotify => rsx! {
            input {
                placeholder: "Spotify Client ID",
                value: "{server_url()}",
                oninput: move |e| server_url.set(e.value()),
                onkeydown: move |e| e.stop_propagation()
            }
            p { class: "text-xs text-white/60",
                "Create an app at developer.spotify.com, add the redirect URI "
                code { "http://127.0.0.1:8898/callback" }
                ", add your Spotify account under User Management, and paste its Client ID above. Saving opens Spotify's sign-in page in your default browser — kopuz never sees your password. Spotify Development Mode is limited to five authorized users and requires the app owner to have Premium. Playback also requires Premium; followed playlists may be listed but Spotify only exposes tracks for playlists you own or collaborate on."
            }
        },
        _ => rsx! {
            input {
                placeholder: "{server_url_placeholder}",
                value: "{server_url()}",
                oninput: move |e| server_url.set(e.value()),
                onkeydown: move |e| e.stop_propagation()
            }
        },
    }
}
