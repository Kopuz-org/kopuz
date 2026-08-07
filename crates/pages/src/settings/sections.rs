use components::settings_items::{ChannelModeSelector, DeviceChangeBehaviorSelector, DiscordPresencePausedSettings, DiscordPresenceSettings, EqualizerPanel, IntifaceSettings, LastFmSettings, LibreFmSettings, MusicBrainzSettings, SampleRateModeSelector, SettingItem, SettingsSection, ToggleSetting};
use config::{AppConfig, FetchStrategy, OfflineQuality};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

#[component]
pub(super) fn ConnectivitySection(mut config: Signal<AppConfig>) -> Element {
    rsx! {
        SettingsSection { title: i18n::t("connectivity").to_string(),
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
                if config.read().discord_presence.unwrap_or(true) {
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
            if !cfg!(target_os = "android") {
                SettingItem {
                    title: i18n::t("intiface").to_string(),
                    control: rsx! {
                        IntifaceSettings {
                            host: config.read().intiface_host.clone(),
                            port: config.read().intiface_port.clone(),
                            connected: config.read().intiface_connected.unwrap_or(false),
                            on_host_save: move |value: String| {
                                config.write().intiface_host = value;
                            },
                            on_port_save: move |value: String| {
                                config.write().intiface_port = value;
                            },
                            on_connected_save: move |value: bool| {
                                config.write().intiface_connected = Some(value);
                            },
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn DownloadsSection(mut config: Signal<AppConfig>) -> Element {
    rsx! {
        SettingsSection { title: i18n::t("offline_downloads").to_string(),
            SettingItem {
                title: i18n::t("download_quality").to_string(),
                control: rsx! {
                    select {
                        class: "bg-white/10 text-white rounded-lg px-3 py-2 text-sm border border-white/10 focus:outline-none focus:border-white/25",
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

#[component]
pub(super) fn MetadataSection(mut config: Signal<AppConfig>) -> Element {
    rsx! {
        SettingsSection { title: i18n::t("metadata").to_string(),
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
                                class: "bg-white/10 text-white rounded-lg px-3 py-2 text-sm border border-white/10 focus:outline-none focus:border-white/25",
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
        }
    }
}

#[component]
pub(super) fn PlayerSection(mut config: Signal<AppConfig>) -> Element {
    let ctrl = use_context::<PlayerController>();
    let crossfade_label = if config.read().crossfade_seconds == 0 {
        i18n::t("crossfade_off")
    } else {
        format!("{}s", config.read().crossfade_seconds)
    };

    rsx! {
        SettingsSection { title: i18n::t("player_settings").to_string(),
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
            div { class: "px-5 py-4",
                p { class: "text-sm text-white/90 font-medium mb-3", "{i18n::t(\"equalizer\")}" }
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
    }
}
