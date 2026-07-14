use config::{Browser, MusicService};
use dioxus::prelude::*;

#[component]
pub fn AddServerPopup(
    server_name: Signal<String>,
    server_url: Signal<String>,
    server_service: Signal<MusicService>,
    /// Selected Chromium-family browser when service is YouTube Music.
    yt_browser: Signal<Browser>,
    /// YouTube Music anonymous mode — true = no sign-in, browse + play
    /// public surfaces only.
    yt_anonymous: Signal<bool>,
    /// TIDAL public-API app credentials (the user's own developer.tidal.com
    /// client). Secret is optional for a public PKCE client.
    tidal_client_id: Signal<String>,
    tidal_client_secret: Signal<String>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    let _service_value = match server_service() {
        MusicService::Jellyfin => "jellyfin",
        MusicService::Subsonic => "subsonic",
        MusicService::Custom => "custom",
        MusicService::YtMusic => "ytmusic",
        MusicService::SoundCloud => "soundcloud",
        MusicService::Tidal => "tidal",
    };

    let server_name_label = i18n::t("server_name").to_string();
    let server_url_placeholder = i18n::t("server_url_placeholder").to_string();
    let custom_manual = i18n::t("custom_manual").to_string();
    let cancel_text = i18n::t("cancel").to_string();
    let save_text = i18n::t("save").to_string();

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
                    tidal_client_id,
                    tidal_client_secret,
                    server_url_placeholder: server_url_placeholder.clone(),
                }

                select {
                    onchange: move |e| {
                        let service = match e.value().as_str() {
                            "subsonic" => MusicService::Subsonic,
                            "custom" => MusicService::Custom,
                            "ytmusic" => MusicService::YtMusic,
                            "soundcloud" => MusicService::SoundCloud,
                            "tidal" => MusicService::Tidal,
                            _ => MusicService::Jellyfin,
                        };
                        server_service.set(service);
                    },
                    onkeydown: move |e| e.stop_propagation(),
                    option {
                        value: "jellyfin",
                        selected: server_service() == MusicService::Jellyfin,
                        "{i18n::t(\"jellyfin\")}"
                    }
                    option {
                        value: "subsonic",
                        selected: server_service() == MusicService::Subsonic,
                        "{i18n::t(\"subsonic\")}"
                    }
                    option {
                        value: "custom",
                        selected: server_service() == MusicService::Custom,
                        "{custom_manual}"
                    }
                    option {
                        value: "ytmusic",
                        selected: server_service() == MusicService::YtMusic,
                        "YouTube Music"
                    }
                    option {
                        value: "soundcloud",
                        selected: server_service() == MusicService::SoundCloud,
                        "SoundCloud"
                    }
                    option {
                        value: "tidal",
                        selected: server_service() == MusicService::Tidal,
                        "TIDAL"
                    }
                }

                div { class: "actions",
                    button {
                        onclick: move |_| on_close.call(()),
                        "{cancel_text}"
                    }
                    button {
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
    tidal_client_id: Signal<String>,
    tidal_client_secret: Signal<String>,
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
        // SoundCloud is browser sign-in only (no URL); pick the browser for the
        // isolated sign-in window.
        MusicService::SoundCloud => rsx! {
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
        // TIDAL uses the public API with the user's own registered app: enter
        // its client ID (+ secret if a confidential client). After Save, the
        // default browser opens the TIDAL login and kopuz captures the redirect.
        MusicService::Tidal => rsx! {
            p { class: "text-xs text-white/60",
                "Register an app at developer.tidal.com and add the redirect URI http://localhost:8765/callback. Paste its Client ID below (Client Secret only if it's a confidential client). A paid TIDAL subscription is required for playback."
            }
            input {
                placeholder: "Client ID",
                value: "{tidal_client_id()}",
                oninput: move |e| tidal_client_id.set(e.value()),
                onkeydown: move |e| e.stop_propagation()
            }
            input {
                r#type: "password",
                placeholder: "Client secret (optional)",
                value: "{tidal_client_secret()}",
                oninput: move |e| tidal_client_secret.set(e.value()),
                onkeydown: move |e| e.stop_propagation()
            }
            // TIDAL's login rejects Firefox / privacy-hardened browsers
            // (DataDome → UNSUPPORTED_OS), so sign-in runs in a clean isolated
            // Chromium-family profile regardless of your default browser.
            p { class: "text-xs text-white/60 mt-2",
                "Pick a Chromium-family browser for the sign-in window — it opens in an isolated profile (a fresh session; your normal browsing is untouched). Firefox is not supported by TIDAL's login."
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
