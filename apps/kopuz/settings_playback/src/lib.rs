//! Settings: offline downloads, metadata, the player and the tools category.

mod equalizer;

use api::schema::FieldSpec;
use components::format::{t, t_with};
use components::settings::{
    Choice, SettingRow, SliderRow, on_slider_change, section, set_slider, slider_state, toggle_row,
};
use components::{Callout, CalloutTone, FieldChanged, Lockable as _, SchemaForm, icons};
use config::{
    ChannelMode, DeviceChangeBehavior, LYRICS_OFFSET_LIMIT_MS, LibraryEdit, LyricsEdit,
    OfflineQuality, PlaybackEdit, SampleRateMode, SettingsEdit, SystemEdit,
};
use equalizer::Equalizer;
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled, Subscription, Window, div,
};
use link::Link;
use store::{ConfigModel, Notices, PlayerModel, Query};

const CROSSFADE_MAX_SECONDS: f32 = 12.0;
const VOLUME_STEP_MIN_PERCENT: f32 = 1.0;
const VOLUME_STEP_MAX_PERCENT: f32 = 50.0;
const LYRICS_OFFSET_STEP_MS: f32 = 50.0;

pub struct PlaybackSettings {
    artwork: Entity<SchemaForm>,
    artwork_settings: Entity<Query<Vec<FieldSpec>>>,
    crossfade: Entity<SliderState>,
    volume_scroll: Entity<SliderState>,
    lyrics_offset: Entity<SliderState>,
    equalizer: Entity<Equalizer>,
    section: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl PlaybackSettings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = ConfigModel::global(cx);
        let (crossfade_seconds, volume_step, lyrics_offset_ms) = {
            let current = config.read(cx).config();
            (
                current.playback.crossfade_seconds,
                current.playback.volume_scroll_step,
                current.lyrics.lyrics_offset_ms,
            )
        };
        let crossfade =
            cx.new(|_| slider_state(0.0, CROSSFADE_MAX_SECONDS, 1.0, crossfade_seconds as f32));
        let volume_scroll = cx.new(|_| {
            slider_state(
                VOLUME_STEP_MIN_PERCENT,
                VOLUME_STEP_MAX_PERCENT,
                1.0,
                volume_percent(volume_step),
            )
        });
        let limit = LYRICS_OFFSET_LIMIT_MS as f32;
        let lyrics_offset = cx.new(|_| {
            slider_state(
                -limit,
                limit,
                LYRICS_OFFSET_STEP_MS,
                lyrics_offset_ms as f32,
            )
        });

        let artwork = cx.new(|cx| SchemaForm::new(Vec::new(), window, cx));
        let artwork_settings =
            cx.new(|cx| Query::new(&[], |api| async move { api.artwork_settings().await }, cx));
        let equalizer = cx.new(|cx| Equalizer::new(window, cx));

        let player = PlayerModel::global(cx);
        let subscriptions = vec![
            cx.observe_in(&config, window, |this, _, window, cx| this.sync(window, cx)),
            // The auto-offset row reads the engine's measured output latency.
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe_in(&artwork_settings, window, Self::adopt_artwork),
            cx.subscribe_in(&artwork, window, Self::save_artwork),
            on_slider_change(
                &crossfade,
                cx,
                ConfigModel::writer(|value: f32| {
                    SettingsEdit::Playback(PlaybackEdit::CrossfadeSeconds(
                        value.clamp(0.0, CROSSFADE_MAX_SECONDS) as u8,
                    ))
                }),
            ),
            on_slider_change(
                &volume_scroll,
                cx,
                ConfigModel::writer(|value: f32| {
                    SettingsEdit::Playback(PlaybackEdit::VolumeScrollStep(
                        value.clamp(VOLUME_STEP_MIN_PERCENT, VOLUME_STEP_MAX_PERCENT) / 100.0,
                    ))
                }),
            ),
            on_slider_change(
                &lyrics_offset,
                cx,
                ConfigModel::writer(|value: f32| {
                    SettingsEdit::Lyrics(LyricsEdit::LyricsOffsetMs(
                        (value as i32).clamp(-LYRICS_OFFSET_LIMIT_MS, LYRICS_OFFSET_LIMIT_MS),
                    ))
                }),
            ),
        ];

        Self {
            section: None,
            artwork,
            artwork_settings,
            crossfade,
            volume_scroll,
            lyrics_offset,
            equalizer,
            _subscriptions: subscriptions,
        }
    }

    /// Follow the config, so a daemon-side change or a managed file moves the sliders too.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = ConfigModel::global(cx);
        let (crossfade, volume, offset) = {
            let config = model.read(cx).config();
            (
                config.playback.crossfade_seconds as f32,
                volume_percent(config.playback.volume_scroll_step),
                config.lyrics.lyrics_offset_ms as f32,
            )
        };
        set_slider(&self.crossfade, crossfade, window, cx);
        set_slider(&self.volume_scroll, volume, window, cx);
        set_slider(&self.lyrics_offset, offset, window, cx);
        cx.notify();
    }

    /// The daemon owns which artwork providers exist; a settled read is the whole form.
    fn adopt_artwork(
        &mut self,
        query: Entity<Query<Vec<FieldSpec>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = query.read(cx);
        if query.is_loading() {
            return;
        }
        let Some(specs) = query.value().cloned() else {
            return;
        };
        let locked = ConfigModel::global(cx).read(cx).locked_names();
        self.artwork
            .update(cx, |form, cx| form.sync(specs, locked, false, window, cx));
    }

    fn save_artwork(
        &mut self,
        form: &Entity<SchemaForm>,
        event: &FieldChanged,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = event.0.clone();
        let key = value.key.clone();
        let form = form.downgrade();
        let call = Link::global(cx)
            .call(move |api| async move { api.set_artwork_settings(vec![value]).await });
        cx.spawn_in(window, async move |this, cx| match call.await {
            Ok(_) => {
                this.update(cx, |this, cx| {
                    this.artwork_settings
                        .update(cx, |query, cx| query.refresh(cx));
                })
                .ok();
            }
            // The form is showing an answer the daemon never took; put the held one back.
            Err(error) => {
                tracing::warn!(%error, "could not save the artwork settings");
                form.update_in(cx, |form, window, cx| {
                    form.revert(&key, window, cx);
                    Notices::failed(&error, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    fn offline(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let quality = ConfigModel::global(cx)
            .read(cx)
            .config()
            .library
            .offline_quality;
        section(t("offline_downloads")).children(
            SettingRow::new(t("download_quality"))
                .bind("offline_quality", cx)
                .description(t("download_quality_description"))
                .setting(
                    Choice::listed(
                        "offline-quality",
                        quality,
                        OfflineQuality::ALL.iter().copied(),
                        |quality| SharedString::from(quality.label()),
                    )
                    .on_change(ConfigModel::writer(|quality| {
                        SettingsEdit::Library(LibraryEdit::OfflineQuality(quality))
                    })),
                )
                .shown(cx),
        )
    }

    fn metadata(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let model = ConfigModel::global(cx);
        let config = model.read(cx).config();
        let prefer_local = config.lyrics.prefer_local_lyrics;
        let musixmatch = config.lyrics.enable_musixmatch_lyrics;
        let auto = config.lyrics.lyrics_offset_auto;
        let latency_ms = PlayerModel::global(cx)
            .read(cx)
            .output_latency_ms()
            .unwrap_or(0);

        section(t("metadata"))
            .children(components::settings::when_unfiltered(
                div().px_5().py_2p5().child(self.artwork.clone()),
                cx,
            ))
            .children(
                toggle_row(
                    "prefer_local_lyrics",
                    t("prefer_local_lyrics"),
                    prefer_local,
                    ConfigModel::writer(|on| {
                        SettingsEdit::Lyrics(LyricsEdit::PreferLocalLyrics(on))
                    }),
                )
                .bind("prefer_local_lyrics", cx)
                .description(t("prefer_local_lyrics_description"))
                .shown(cx),
            )
            .children(
                toggle_row(
                    "enable_musixmatch_lyrics",
                    t("enable_musixmatch_lyrics"),
                    musixmatch,
                    ConfigModel::writer(|on| {
                        SettingsEdit::Lyrics(LyricsEdit::EnableMusixmatchLyrics(on))
                    }),
                )
                .bind("enable_musixmatch_lyrics", cx)
                .description(t("enable_musixmatch_lyrics_description"))
                .shown(cx),
            )
            .children(
                toggle_row(
                    "lyrics_offset_auto",
                    t("lyrics_offset_auto"),
                    auto,
                    ConfigModel::writer(|on| {
                        SettingsEdit::Lyrics(LyricsEdit::LyricsOffsetAuto(on))
                    }),
                )
                .bind("lyrics_offset_auto", cx)
                .description(t_with(
                    "lyrics_offset_auto_description",
                    &[("latency", latency_ms.into())],
                ))
                .shown(cx),
            )
            .children(
                SettingRow::new(t("lyrics_offset"))
                    .bind("lyrics_offset_ms", cx)
                    .description(t("lyrics_offset_description"))
                    .setting(
                        SliderRow::new(&self.lyrics_offset)
                            .readout(move |value| {
                                SharedString::from(match (auto, value as i32) {
                                    (true, _) => format!("{latency_ms} ms"),
                                    (false, 0) => "0 ms".to_string(),
                                    (false, offset) => format!("{offset:+} ms"),
                                })
                            })
                            .lock(auto),
                    )
                    .shown(cx),
            )
    }

    fn player(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let model = ConfigModel::global(cx);
        let config = model.read(cx).config();
        let channel_mode = config.playback.channel_mode;
        let device_change = config.playback.device_change_behavior;
        let sample_rate = config.playback.sample_rate_mode;

        section(t("player_settings"))
            .children(
                SettingRow::new(t("crossfade"))
                    .bind("crossfade_seconds", cx)
                    .description(t("crossfade_description"))
                    .setting(SliderRow::new(&self.crossfade).readout(|seconds| {
                        match seconds as i32 {
                            0 => t("crossfade_off"),
                            seconds => SharedString::from(format!("{seconds}s")),
                        }
                    }))
                    .shown(cx),
            )
            .children(
                SettingRow::new(t("volume_scroll_step"))
                    .bind("volume_scroll_step", cx)
                    .description(t("volume_scroll_step_description"))
                    .setting(
                        SliderRow::new(&self.volume_scroll)
                            .readout(|percent| SharedString::from(format!("{}%", percent as i32))),
                    )
                    .shown(cx),
            )
            .children(
                SettingRow::new(t("channel_mode"))
                    .bind("channel_mode", cx)
                    .description(t("channel_mode_description"))
                    .setting(
                        Choice::listed(
                            "channel_mode",
                            channel_mode,
                            ChannelMode::ALL.iter().copied(),
                            channel_mode_label,
                        )
                        .on_change(ConfigModel::writer(|mode| {
                            SettingsEdit::Playback(PlaybackEdit::ChannelMode(mode))
                        })),
                    )
                    .shown(cx),
            )
            .children(
                SettingRow::new(t("device_change_behavior"))
                    .bind("device_change_behavior", cx)
                    .description(t("device_change_behavior_description"))
                    .setting(
                        Choice::listed(
                            "device_change_behavior",
                            device_change,
                            DeviceChangeBehavior::ALL.iter().copied(),
                            device_change_label,
                        )
                        .on_change(ConfigModel::writer(|behavior| {
                            SettingsEdit::Playback(PlaybackEdit::DeviceChangeBehavior(behavior))
                        })),
                    )
                    .shown(cx),
            )
            .children(
                SettingRow::new(t("sample_rate_mode"))
                    .bind("sample_rate_mode", cx)
                    .description(t("sample_rate_mode_description"))
                    .setting(
                        Choice::listed(
                            "sample_rate_mode",
                            sample_rate,
                            SampleRateMode::ALL.iter().copied(),
                            sample_rate_label,
                        )
                        .on_change(ConfigModel::writer(|mode| {
                            SettingsEdit::Playback(PlaybackEdit::SampleRateMode(mode))
                        })),
                    )
                    .shown(cx),
            )
            .children(
                SettingRow::new(t("equalizer"))
                    .bind("equalizer", cx)
                    .description(t("equalizer_description"))
                    .stacked()
                    .control(self.equalizer.clone())
                    .shown(cx),
            )
    }

    // TODO(logs): opening and exporting the log folder is local filesystem work a socket-only client has no business doing.
    fn tools(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tracing_enabled = ConfigModel::global(cx)
            .read(cx)
            .config()
            .system
            .tracing_enabled;
        section(t("logs"))
            .children(
                toggle_row(
                    "tracing_enabled",
                    t("enable_tracing"),
                    tracing_enabled,
                    ConfigModel::writer(|on| SettingsEdit::System(SystemEdit::TracingEnabled(on))),
                )
                .bind("tracing_enabled", cx)
                .description(t("enable_tracing_description"))
                .shown(cx),
            )
            .children(components::settings::when_unfiltered(
                div().px_5().pb_3().child(
                    Callout::new(t("tracing_warning"))
                        .tone(CalloutTone::Warning)
                        .icon(icons::WARNING),
                ),
                cx,
            ))
    }
}

impl PlaybackSettings {
    /// Which rail entry is showing; `None` stacks every section.
    pub fn show(&mut self, section: Option<SharedString>, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    fn shows(&self, section: &str) -> bool {
        self.section.as_deref().is_none_or(|shown| shown == section)
    }
}

impl Render for PlaybackSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The settings page owns the scroll; this panel is content inside it.
        v_flex()
            .id("playback-settings")
            .w_full()
            .gap_4()
            .pb_4()
            .when(self.shows("offline"), |panel| panel.child(self.offline(cx)))
            .when(self.shows("metadata"), |panel| {
                panel.child(self.metadata(cx))
            })
            .when(self.shows("player"), |panel| panel.child(self.player(cx)))
            .when(self.shows("tools"), |panel| panel.child(self.tools(cx)))
    }
}

fn volume_percent(step: f32) -> f32 {
    (step * 100.0).round()
}

fn channel_mode_label(mode: ChannelMode) -> SharedString {
    t(match mode {
        ChannelMode::Stereo => "channel_mode_stereo",
        ChannelMode::Mono => "channel_mode_mono",
        ChannelMode::LeftOnly => "channel_mode_left_only",
        ChannelMode::RightOnly => "channel_mode_right_only",
        ChannelMode::SwapLeftRight => "channel_mode_swap_left_right",
    })
}

fn device_change_label(behavior: DeviceChangeBehavior) -> SharedString {
    t(match behavior {
        DeviceChangeBehavior::Resume => "device_change_resume",
        DeviceChangeBehavior::Pause => "device_change_pause",
    })
}

fn sample_rate_label(mode: SampleRateMode) -> SharedString {
    t(match mode {
        SampleRateMode::System => "sample_rate_mode_system",
        SampleRateMode::Source => "sample_rate_mode_source",
    })
}
