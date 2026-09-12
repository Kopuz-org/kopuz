use components::settings_items::{
    ChannelModeSelector, DeviceChangeBehaviorSelector, EqualizerPanel, SampleRateModeSelector,
    SettingItem, SettingsSection, ToggleSetting,
};
use config::{AppConfig, LYRICS_OFFSET_LIMIT_MS, OfflineQuality};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

#[component]
pub(super) fn ConnectivitySection() -> Element {
    // What is offered, and whether each is connected, is the daemon's answer;
    // this counter is what asks it again after a change.
    let changed = use_signal(|| 0u64);
    let mut integrations = hooks::integrations::use_integrations();
    use_effect(move || {
        let _ = changed();
        integrations.restart();
    });
    let listed = integrations.read().clone().unwrap_or_default();
    rsx! {
        SettingsSection { title: i18n::t("connectivity").to_string(),
            for integration in listed.into_iter() {
                IntegrationRows { key: "{integration.id}", integration, changed }
            }
        }
    }
}

/// One integration: its fields, and the button that connects it where
/// filling the fields in is not the whole of it.
#[component]
fn IntegrationRows(integration: api::IntegrationInfo, changed: Signal<u64>) -> Element {
    let name = components::forms::text(&integration.name);
    let id = integration.id.clone();
    let connect_id = integration.id.clone();
    rsx! {
        components::forms::schema_form::SchemaForm {
            fields: integration.fields.clone(),
            values: Vec::new(),
            on_change: move |value: api::FieldValue| {
                hooks::integrations::set_settings(id.clone(), vec![value], changed);
            },
        }
        if integration.connect == api::ConnectKind::WebSignIn {
            SettingItem {
                title: name.clone(),
                control: rsx! {
                    button {
                        class: if integration.configured {
                            "bg-green-500/20 text-green-300 px-3 py-2 rounded-xl text-sm transition-colors"
                        } else {
                            "bg-white/10 hover:bg-white/20 text-white px-3 py-2 rounded-xl text-sm transition-colors"
                        },
                        onclick: move |_| {
                            hooks::integrations::authenticate(connect_id.clone(), changed);
                        },
                        if integration.configured {
                            "{i18n::t_with(\"integration_connected\", &[(\"name\", name.clone())])}"
                        } else {
                            "{i18n::t_with(\"integration_connect\", &[(\"name\", name.clone())])}"
                        }
                    }
                },
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
                config_key: "offline_quality",
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
    let ctrl = use_context::<PlayerController>();
    let artwork_changed = use_signal(|| 0u64);
    let artwork = hooks::artwork_settings::use_settings(artwork_changed);
    let lyrics_offset = config.read().lyrics_offset_ms;
    let lyrics_offset_auto = config.read().lyrics_offset_auto;
    let lyrics_offset_label = if lyrics_offset_auto {
        format!(
            "{} ms",
            (ctrl.output_latency_secs() * 1000.0).round() as i32
        )
    } else if lyrics_offset == 0 {
        "0 ms".to_string()
    } else {
        format!("{lyrics_offset:+} ms")
    };
    let lyrics_offset_class = if lyrics_offset_auto {
        "flex items-center gap-3 min-w-[220px] opacity-40"
    } else {
        "flex items-center gap-3 min-w-[220px]"
    };

    rsx! {
        SettingsSection { title: i18n::t("metadata").to_string(),
            components::forms::schema_form::SchemaForm {
                fields: artwork.read().clone().unwrap_or_default(),
                values: Vec::new(),
                on_change: move |value: api::FieldValue| {
                    hooks::artwork_settings::set(value, artwork_changed);
                },
            }
            SettingItem {
                title: i18n::t("prefer_local_lyrics").to_string(),
                config_key: "prefer_local_lyrics",
                control: rsx! {
                    ToggleSetting {
                        enabled: config.read().prefer_local_lyrics,
                        on_change: move |val| config.write().prefer_local_lyrics = val,
                    }
                }
            }
            SettingItem {
                title: i18n::t("enable_musixmatch_lyrics").to_string(),
                config_key: "enable_musixmatch_lyrics",
                control: rsx! {
                    ToggleSetting {
                        enabled: config.read().enable_musixmatch_lyrics,
                        on_change: move |val| config.write().enable_musixmatch_lyrics = val,
                    }
                }
            }
            SettingItem {
                title: i18n::t("lyrics_offset_auto").to_string(),
                config_key: "lyrics_offset_auto",
                control: rsx! {
                    ToggleSetting {
                        enabled: lyrics_offset_auto,
                        on_change: move |val| config.write().lyrics_offset_auto = val,
                    }
                }
            }
            SettingItem {
                title: i18n::t("lyrics_offset").to_string(),
                config_key: "lyrics_offset_ms",
                control: rsx! {
                    div { class: "{lyrics_offset_class}",
                        input {
                            r#type: "range",
                            min: "{-LYRICS_OFFSET_LIMIT_MS}",
                            max: "{LYRICS_OFFSET_LIMIT_MS}",
                            step: "50",
                            value: "{lyrics_offset}",
                            disabled: lyrics_offset_auto,
                            class: "w-40",
                            style: "accent-color: var(--color-indigo-500);",
                            oninput: move |evt| {
                                if let Ok(value) = evt.value().parse::<i32>() {
                                    config.write().lyrics_offset_ms = value
                                        .clamp(-LYRICS_OFFSET_LIMIT_MS, LYRICS_OFFSET_LIMIT_MS);
                                }
                            }
                        }
                        span {
                            class: "text-xs font-mono text-white/80 w-20 text-right",
                            "{lyrics_offset_label}"
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
                config_key: "crossfade_seconds",
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
                config_key: "volume_scroll_step",
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
                config_key: "channel_mode",
                control: rsx! {
                    ChannelModeSelector {
                        current: config.read().channel_mode,
                        on_change: move |mode| {
                            config.write().channel_mode = mode;
                        }
                    }
                }
            }
            SettingItem {
                title: i18n::t("device_change_behavior").to_string(),
                config_key: "device_change_behavior",
                control: rsx! {
                    DeviceChangeBehaviorSelector {
                        current: config.read().device_change_behavior,
                        on_change: move |behavior| {
                            config.write().device_change_behavior = behavior;
                        }
                    }
                }
            }
            SettingItem {
                title: i18n::t("sample_rate_mode").to_string(),
                config_key: "sample_rate_mode",
                control: rsx! {
                    SampleRateModeSelector {
                        current: config.read().sample_rate_mode,
                        on_change: move |mode| {
                            config.write().sample_rate_mode = mode;
                        }
                    }
                }
            }
            SettingItem {
                title: i18n::t("equalizer").to_string(),
                config_key: "equalizer",
                stacked: true,
                control: rsx! {
                    EqualizerPanel {
                        current: config.read().equalizer.clone(),
                        on_preview: move |equalizer: config::EqualizerSettings| {
                            ctrl.preview_equalizer(equalizer);
                        },
                        on_commit: move |equalizer: config::EqualizerSettings| {
                            config.write().equalizer = equalizer;
                        }
                    }
                }
            }
        }
    }
}
