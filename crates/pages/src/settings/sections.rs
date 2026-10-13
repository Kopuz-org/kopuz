use components::settings_items::{
    AppSelect, ChannelModeSelector, DeviceChangeBehaviorSelector, EqualizerPanel, GainSlider,
    ReplayGainModeSelector, SampleRateModeSelector, SettingItem, SettingsGroup, SettingsSection,
    StreamQualitySelector, ToggleSetting,
};
use config::{
    LYRICS_OFFSET_LIMIT_MS, LibraryEdit, LyricsEdit, OfflineQuality, PlaybackEdit, ReplayGainMode,
    Settings, SettingsEdit,
};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

fn lyrics_offset_edit(value: &str) -> Option<SettingsEdit> {
    let value = value.parse::<i32>().ok()?;
    Some(SettingsEdit::Lyrics(LyricsEdit::LyricsOffsetMs(
        value.clamp(-LYRICS_OFFSET_LIMIT_MS, LYRICS_OFFSET_LIMIT_MS),
    )))
}

fn crossfade_edit(value: &str) -> Option<SettingsEdit> {
    let value = value.parse::<u8>().ok()?;
    Some(SettingsEdit::Playback(PlaybackEdit::CrossfadeSeconds(
        value.min(12),
    )))
}

fn volume_scroll_step_edit(value: &str) -> Option<SettingsEdit> {
    let pct = value.parse::<i32>().ok()?.clamp(1, 50);
    Some(SettingsEdit::Playback(PlaybackEdit::VolumeScrollStep(
        pct as f32 / 100.0,
    )))
}

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
                            "app-button-tonal bg-green-500/20 text-green-300 px-3 py-2 rounded-xl text-sm transition-colors"
                        } else {
                            "app-button-tonal bg-white/10 hover:bg-white/20 text-white px-3 py-2 rounded-xl text-sm transition-colors"
                        },
                        onclick: move |_| {
                            hooks::integrations::authenticate(connect_id.clone(), changed);
                        },
                        if integration.configured {
                            "{i18n::t_with(\"integration_connected\", &[(\"name\", name.clone().into())])}"
                        } else {
                            "{i18n::t_with(\"integration_connect\", &[(\"name\", name.clone().into())])}"
                        }
                    }
                },
            }
        }
        if integration.connect == api::ConnectKind::BrowserProfile {
            BrowserConnectRow { id: integration.id.clone(), name: name.clone(), changed }
        }
    }
}

/// Connect from a browser profile already signed in to the service, beside
/// the fields that take a pasted credential.
#[component]
fn BrowserConnectRow(id: String, name: String, changed: Signal<u64>) -> Element {
    let profiles = hooks::integrations::use_browser_profiles();
    let mut picked = use_signal(String::new);
    let busy = use_signal(|| false);
    let listed = profiles.read().clone().unwrap_or_default();
    let selected = listed
        .iter()
        .find(|profile| profile.id == picked())
        .or(listed.first())
        .map(|profile| profile.id.clone())
        .unwrap_or_default();
    let options: Vec<(String, String)> = listed
        .iter()
        .map(|profile| {
            (
                profile.id.clone(),
                i18n::t_with(
                    "integration_browser_profile",
                    &[
                        ("name", profile.name.clone().into()),
                        ("browser", profile.browser.clone().into()),
                    ],
                ),
            )
        })
        .collect();
    rsx! {
        SettingItem {
            title: i18n::t("integration_browser_title").to_string(),
            control: rsx! {
                if options.is_empty() {
                    p { class: "text-sm text-white/60", "{i18n::t(\"integration_browser_none\")}" }
                } else {
                    div { class: "flex items-center gap-2",
                        AppSelect {
                            class: "settings-select",
                            value: selected.clone(),
                            options,
                            on_change: move |value: String| picked.set(value),
                        }
                        button {
                            class: "app-button-tonal bg-white/10 hover:bg-white/20 text-white px-3 py-2 rounded-xl text-sm transition-colors disabled:opacity-50",
                            disabled: busy(),
                            onclick: move |_| {
                                hooks::integrations::connect_from_browser(
                                    id.clone(),
                                    selected.clone(),
                                    busy,
                                    changed,
                                );
                            },
                            if busy() {
                                "{i18n::t(\"integration_connecting\")}"
                            } else {
                                "{i18n::t_with(\"integration_connect\", &[(\"name\", name.clone().into())])}"
                            }
                        }
                    }
                }
            },
        }
    }
}

#[component]
pub(super) fn DownloadsSection(settings: Signal<Settings>) -> Element {
    let edits = hooks::settings::use_settings();
    rsx! {
        SettingsSection { title: i18n::t("offline_downloads").to_string(),
            SettingItem {
                title: i18n::t("download_quality").to_string(),
                config_key: "offline_quality",
                control: rsx! {
                    AppSelect {
                        class: "settings-select",
                        value: settings.read().library.offline_quality.value_str().to_string(),
                        options: OfflineQuality::ALL
                            .iter()
                            .map(|q| (q.value_str().to_string(), q.label().to_string()))
                            .collect::<Vec<_>>(),
                        on_change: move |value: String| {
                            edits.edit(SettingsEdit::Library(LibraryEdit::OfflineQuality(OfflineQuality::from_value_str(&value))));
                        },
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn MetadataSection(settings: Signal<Settings>) -> Element {
    let edits = hooks::settings::use_settings();
    let ctrl = use_context::<PlayerController>();
    let artwork_changed = use_signal(|| 0u64);
    let artwork = hooks::artwork_settings::use_settings(artwork_changed);
    let lyrics_offset = settings.read().lyrics.lyrics_offset_ms;
    let lyrics_offset_auto = settings.read().lyrics.lyrics_offset_auto;
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
        "settings-slider flex items-center gap-3 min-w-[220px] opacity-40"
    } else {
        "settings-slider flex items-center gap-3 min-w-[220px]"
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
                        enabled: settings.read().lyrics.prefer_local_lyrics,
                        on_change: move |val| edits.edit(SettingsEdit::Lyrics(LyricsEdit::PreferLocalLyrics(val))),
                    }
                }
            }
            SettingItem {
                title: i18n::t("enable_musixmatch_lyrics").to_string(),
                config_key: "enable_musixmatch_lyrics",
                control: rsx! {
                    ToggleSetting {
                        enabled: settings.read().lyrics.enable_musixmatch_lyrics,
                        on_change: move |val| edits.edit(SettingsEdit::Lyrics(LyricsEdit::EnableMusixmatchLyrics(val))),
                    }
                }
            }
            SettingItem {
                title: i18n::t("lyrics_offset_auto").to_string(),
                config_key: "lyrics_offset_auto",
                control: rsx! {
                    ToggleSetting {
                        enabled: lyrics_offset_auto,
                        on_change: move |val| edits.edit(SettingsEdit::Lyrics(LyricsEdit::LyricsOffsetAuto(val))),
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
                            style: components::settings::items::range_style(f64::from(lyrics_offset), f64::from(-LYRICS_OFFSET_LIMIT_MS), f64::from(LYRICS_OFFSET_LIMIT_MS)),
                            oninput: move |evt| {
                                if let Some(edit) = lyrics_offset_edit(&evt.value()) {
                                    edits.preview(edit);
                                }
                            },
                            onchange: move |evt| {
                                if let Some(edit) = lyrics_offset_edit(&evt.value()) {
                                    edits.edit(edit);
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
pub(super) fn PlayerSection(settings: Signal<Settings>) -> Element {
    let edits = hooks::settings::use_settings();
    let ctrl = use_context::<PlayerController>();
    let caps = hooks::sources::use_capabilities();
    let caps = caps.read().clone();
    let crossfade_label = if settings.read().playback.crossfade_seconds == 0 {
        i18n::t("crossfade_off")
    } else {
        format!("{}s", settings.read().playback.crossfade_seconds)
    };

    rsx! {
        SettingsSection { title: i18n::t("player_settings").to_string(),
            SettingsGroup { label: i18n::t("settings_group_playback") }
            SettingItem {
                title: i18n::t("crossfade").to_string(),
                config_key: "crossfade_seconds",
                control: rsx! {
                    div { class: "settings-slider flex items-center gap-3 min-w-[220px]",
                        input {
                            r#type: "range",
                            min: "0",
                            max: "12",
                            step: "1",
                            value: format!("{}", settings.read().playback.crossfade_seconds),
                            class: "w-40",
                            style: components::settings::items::range_style(f64::from(settings.read().playback.crossfade_seconds), 0.0, 12.0),
                            oninput: move |evt| {
                                if let Some(edit) = crossfade_edit(&evt.value()) {
                                    edits.preview(edit);
                                }
                            },
                            onchange: move |evt| {
                                if let Some(edit) = crossfade_edit(&evt.value()) {
                                    edits.edit(edit);
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
            // Mouse wheel only.
            if !cfg!(target_os = "android") {
                SettingItem {
                    title: i18n::t("volume_scroll_step").to_string(),
                    config_key: "volume_scroll_step",
                    control: rsx! {
                        div { class: "settings-slider flex items-center gap-3 min-w-[220px]",
                            input {
                                r#type: "range",
                                min: "1",
                                max: "50",
                                step: "1",
                                value: format!("{}", (settings.read().playback.volume_scroll_step * 100.0).round() as i32),
                                class: "w-40",
                                style: components::settings::items::range_style(f64::from(settings.read().playback.volume_scroll_step) * 100.0, 1.0, 50.0),
                                oninput: move |evt| {
                                    if let Some(edit) = volume_scroll_step_edit(&evt.value()) {
                                        edits.preview(edit);
                                    }
                                },
                                onchange: move |evt| {
                                    if let Some(edit) = volume_scroll_step_edit(&evt.value()) {
                                        edits.edit(edit);
                                    }
                                }
                            }
                            span {
                                class: "text-xs font-mono text-white/80 w-16 text-right",
                                "{(settings.read().playback.volume_scroll_step * 100.0).round() as i32}%"
                            }
                        }
                    }
                }
            }
            if caps.track_radio {
                SettingItem {
                    title: i18n::t("autoplay_radio").to_string(),
                    config_key: "autoplay_radio",
                    control: rsx! {
                        ToggleSetting {
                            enabled: settings.read().playback.autoplay_radio,
                            on_change: move |val| edits.edit(SettingsEdit::Playback(PlaybackEdit::AutoplayRadio(val))),
                        }
                    }
                }
            }
            if caps.explicit_flags {
                SettingItem {
                    title: i18n::t("skip_explicit").to_string(),
                    config_key: "skip_explicit",
                    control: rsx! {
                        ToggleSetting {
                            enabled: settings.read().playback.skip_explicit,
                            on_change: move |val| edits.edit(SettingsEdit::Playback(PlaybackEdit::SkipExplicit(val))),
                        }
                    }
                }
            }
            if caps.stream_quality || caps.watch_history {
                SettingsGroup { label: i18n::t("settings_group_streaming") }
            }
            if caps.stream_quality {
                SettingItem {
                    title: i18n::t("stream_quality").to_string(),
                    config_key: "stream_quality",
                    control: rsx! {
                        StreamQualitySelector {
                            current: settings.read().playback.stream_quality,
                            on_change: move |quality| edits.edit(SettingsEdit::Playback(PlaybackEdit::StreamQuality(quality))),
                        }
                    }
                }
            }
            if caps.watch_history {
                SettingItem {
                    title: i18n::t("pause_watch_history").to_string(),
                    config_key: "pause_watch_history",
                    control: rsx! {
                        ToggleSetting {
                            enabled: settings.read().playback.pause_watch_history,
                            on_change: move |val| edits.edit(SettingsEdit::Playback(PlaybackEdit::PauseWatchHistory(val))),
                        }
                    }
                }
            }
            SettingsGroup { label: i18n::t("settings_group_output") }
            SettingItem {
                title: i18n::t("channel_mode").to_string(),
                config_key: "channel_mode",
                control: rsx! {
                    ChannelModeSelector {
                        current: settings.read().playback.channel_mode,
                        on_change: move |mode| {
                            edits.edit(SettingsEdit::Playback(PlaybackEdit::ChannelMode(mode)));
                        }
                    }
                }
            }
            SettingItem {
                title: i18n::t("device_change_behavior").to_string(),
                config_key: "device_change_behavior",
                control: rsx! {
                    DeviceChangeBehaviorSelector {
                        current: settings.read().playback.device_change_behavior,
                        on_change: move |behavior| {
                            edits.edit(SettingsEdit::Playback(PlaybackEdit::DeviceChangeBehavior(behavior)));
                        }
                    }
                }
            }
            SettingItem {
                title: i18n::t("sample_rate_mode").to_string(),
                config_key: "sample_rate_mode",
                control: rsx! {
                    SampleRateModeSelector {
                        current: settings.read().playback.sample_rate_mode,
                        on_change: move |mode| {
                            edits.edit(SettingsEdit::Playback(PlaybackEdit::SampleRateMode(mode)));
                        }
                    }
                }
            }
            SettingsGroup { label: i18n::t("settings_group_loudness") }
            SettingItem {
                title: i18n::t("replay_gain").to_string(),
                config_key: "replay_gain",
                control: rsx! {
                    ReplayGainModeSelector {
                        current: settings.read().playback.replay_gain.mode,
                        on_change: move |mode| {
                            let mut replay_gain = settings.peek().playback.replay_gain;
                            replay_gain.mode = mode;
                            edits.edit(SettingsEdit::Playback(PlaybackEdit::ReplayGain(replay_gain)));
                        }
                    }
                }
            }
            SettingItem {
                title: i18n::t("replay_gain_youtube_loudness").to_string(),
                config_key: "replay_gain",
                control: rsx! {
                    ToggleSetting {
                        enabled: settings.read().playback.replay_gain.normalize_loudness,
                        on_change: move |enabled| {
                            let mut replay_gain = settings.peek().playback.replay_gain;
                            replay_gain.normalize_loudness = enabled;
                            edits.edit(SettingsEdit::Playback(PlaybackEdit::ReplayGain(replay_gain)));
                        }
                    }
                }
            }
            if settings.read().playback.replay_gain.mode != ReplayGainMode::Off {
                SettingItem {
                    title: i18n::t("replay_gain_prevent_clipping").to_string(),
                    config_key: "replay_gain",
                    nested: true,
                    control: rsx! {
                        ToggleSetting {
                            enabled: settings.read().playback.replay_gain.prevent_clipping,
                            on_change: move |enabled| {
                                let mut replay_gain = settings.peek().playback.replay_gain;
                                replay_gain.prevent_clipping = enabled;
                                edits.edit(SettingsEdit::Playback(PlaybackEdit::ReplayGain(replay_gain)));
                            }
                        }
                    }
                }
                SettingItem {
                    title: i18n::t("replay_gain_preamp").to_string(),
                    config_key: "replay_gain",
                    nested: true,
                    control: rsx! {
                        GainSlider {
                            value: settings.read().playback.replay_gain.preamp_db,
                            min: -15.0,
                            max: 15.0,
                            on_preview: move |db| {
                                let mut replay_gain = settings.peek().playback.replay_gain;
                                replay_gain.preamp_db = db;
                                edits.preview(SettingsEdit::Playback(PlaybackEdit::ReplayGain(replay_gain)));
                                ctrl.preview_replay_gain(replay_gain);
                            },
                            on_commit: move |db| {
                                let mut replay_gain = settings.peek().playback.replay_gain;
                                replay_gain.preamp_db = db;
                                edits.edit(SettingsEdit::Playback(PlaybackEdit::ReplayGain(replay_gain)));
                            }
                        }
                    }
                }
                SettingItem {
                    title: i18n::t("replay_gain_fallback").to_string(),
                    config_key: "replay_gain",
                    nested: true,
                    control: rsx! {
                        GainSlider {
                            value: settings.read().playback.replay_gain.fallback_gain_db,
                            min: -15.0,
                            max: 15.0,
                            on_preview: move |db| {
                                let mut replay_gain = settings.peek().playback.replay_gain;
                                replay_gain.fallback_gain_db = db;
                                edits.preview(SettingsEdit::Playback(PlaybackEdit::ReplayGain(replay_gain)));
                                ctrl.preview_replay_gain(replay_gain);
                            },
                            on_commit: move |db| {
                                let mut replay_gain = settings.peek().playback.replay_gain;
                                replay_gain.fallback_gain_db = db;
                                edits.edit(SettingsEdit::Playback(PlaybackEdit::ReplayGain(replay_gain)));
                            }
                        }
                    }
                }
            }
            if !cfg!(target_os = "android") {
                EqualizerItem { settings }
            }
        }
    }
}

#[component]
pub(super) fn EqualizerSection(settings: Signal<Settings>) -> Element {
    rsx! {
        SettingsSection { title: i18n::t("equalizer").to_string(),
            EqualizerItem { settings }
        }
    }
}

#[component]
fn EqualizerItem(settings: Signal<Settings>) -> Element {
    let edits = hooks::settings::use_settings();
    let ctrl = use_context::<PlayerController>();
    rsx! {
        SettingItem {
            title: i18n::t("equalizer").to_string(),
            config_key: "equalizer",
            stacked: true,
            control: rsx! {
                EqualizerPanel {
                    current: settings.read().playback.equalizer.clone(),
                    on_preview: move |equalizer: config::EqualizerSettings| {
                        ctrl.preview_equalizer(equalizer);
                    },
                    on_commit: move |equalizer: config::EqualizerSettings| {
                        edits.edit(SettingsEdit::Playback(PlaybackEdit::Equalizer(equalizer)));
                    }
                }
            }
        }
    }
}
