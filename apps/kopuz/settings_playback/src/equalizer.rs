//! The equalizer panel: enable, preset, preamp and the ten bands, heard live through `preview_equalizer`.

use components::format::t;
use components::settings::{Choice, READOUT, SliderRow, Toggle, set_slider, slider_state};
use components::{Lockable as _, motion, style};
use config::{EqPreset, EqualizerSettings, PlaybackEdit, SettingsEdit};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Div, ElementId, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled, Subscription, Window, div,
};
use link::Link;
use store::ConfigModel;

/// `border-white/8` around the band graph.
const GRAPH_BORDER: f32 = 0.08;
/// `text-white/58` under each band, the way the graph labelled its axis.
const BAND_LABEL: f32 = 0.58;

const BAND_MIN_DB: f32 = -12.0;
const BAND_MAX_DB: f32 = 12.0;
const PREAMP_MIN_DB: f32 = -12.0;
const PREAMP_MAX_DB: f32 = 6.0;
const STEP_DB: f32 = 0.5;
const CONFIG_KEY: &str = "equalizer";

const BAND_LABELS: [&str; 10] = [
    "32", "64", "125", "250", "500", "1k", "2k", "4k", "8k", "16k",
];

pub struct Equalizer {
    preamp: Entity<SliderState>,
    bands: Vec<Entity<SliderState>>,
    _subscriptions: Vec<Subscription>,
}

impl Equalizer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = current(cx);
        let gains = settings.resolved_bands();
        let preamp =
            cx.new(|_| slider_state(PREAMP_MIN_DB, PREAMP_MAX_DB, STEP_DB, settings.preamp_db));
        let bands: Vec<Entity<SliderState>> = gains
            .iter()
            .map(|gain| cx.new(|_| slider_state(BAND_MIN_DB, BAND_MAX_DB, STEP_DB, *gain)))
            .collect();

        let config = ConfigModel::global(cx);
        let mut subscriptions = vec![
            cx.observe_in(&config, window, |this, _, window, cx| this.sync(window, cx)),
            cx.subscribe(&preamp, |_, _, event: &SliderEvent, cx| {
                let (value, settled) = match event {
                    SliderEvent::Change(value) => (value, false),
                    SliderEvent::Release(value) => (value, true),
                };
                let mut settings = current(cx);
                settings.preamp_db = value.end();
                apply(settings, settled, cx);
            }),
        ];
        for (index, band) in bands.iter().enumerate() {
            subscriptions.push(cx.subscribe(band, move |_, _, event: &SliderEvent, cx| {
                let (value, settled) = match event {
                    SliderEvent::Change(value) => (value, false),
                    SliderEvent::Release(value) => (value, true),
                };
                set_band(index, value.end(), settled, cx);
            }));
        }

        Self {
            preamp,
            bands,
            _subscriptions: subscriptions,
        }
    }

    /// Follow the config: a preset, a daemon-side change or another row moved the curve.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = current(cx);
        set_slider(&self.preamp, settings.preamp_db, window, cx);
        // A preset's ten bands are swept by `render` instead, so they are left alone here.
        if settings.preset == EqPreset::Custom {
            for (band, gain) in self.bands.iter().zip(settings.resolved_bands()) {
                set_slider(band, gain, window, cx);
            }
        }
        cx.notify();
    }

    /// The ten sliders a preset moves at once, each tweened; sampled even mid-drag so the next sweep starts there.
    fn sweep(
        &self,
        gains: &[f32],
        sweeping: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<f32> {
        let mut shown = Vec::with_capacity(self.bands.len());
        for (index, band) in self.bands.iter().enumerate() {
            let Some(gain) = gains.get(index).copied() else {
                continue;
            };
            let value = motion::amount(
                (ElementId::from("eq-band"), BAND_LABELS[index]),
                gain,
                motion::SLOW,
                window,
                cx,
            );
            if sweeping {
                set_slider(band, value, window, cx);
                shown.push(value);
            } else {
                shown.push(gain);
            }
        }
        shown
    }
}

impl Render for Equalizer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = ConfigModel::global(cx);
        let locked = model.read(cx).is_locked(CONFIG_KEY);
        let settings = current(cx);
        let enabled = settings.enabled;
        let preset = settings.preset;
        let gains = self.sweep(
            &settings.resolved_bands(),
            preset != EqPreset::Custom,
            window,
            cx,
        );
        let muted = cx.theme().muted_foreground;
        let readout = style::white(READOUT, cx);

        v_flex()
            .w_full()
            .gap_4()
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_stretch()
                    .child(
                        Toggle::new(CONFIG_KEY)
                            .enabled(enabled)
                            .disabled(locked)
                            .on_change(cx.listener(|_, checked: &bool, _, cx| {
                                let mut settings = current(cx);
                                settings.enabled = *checked;
                                apply(settings, true, cx);
                            })),
                    )
                    .child(
                        panel(cx)
                            .w_64()
                            .child(div().text_xs().text_color(muted).child(t("eq_preset")))
                            .child(
                                Choice::listed("eq-preset", preset, EqPreset::all(), preset_label)
                                    .fill(true)
                                    .on_change(set_preset)
                                    .lock(locked),
                            ),
                    )
                    .child(
                        panel(cx)
                            .flex_1()
                            .gap_3()
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(div().text_xs().text_color(muted).child(t("eq_preamp")))
                                    .child(
                                        div()
                                            .text_size(style::text_11())
                                            .text_color(style::slate_500(cx))
                                            .child(t("eq_preamp_desc")),
                                    ),
                            )
                            .child(
                                SliderRow::new(&self.preamp)
                                    .fill(true)
                                    .readout(|db| SharedString::from(format!("{db:+.1} dB")))
                                    .lock(locked),
                            ),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(style::slate_500(cx))
                    .child(t("eq_graph_hint")),
            )
            .child(
                // The Dioxus panel drew a draggable SVG curve, which GPUI has no path element for.
                div()
                    .w_full()
                    .p_4()
                    .rounded(style::radius_lg())
                    .border_1()
                    .border_color(style::white(GRAPH_BORDER, cx))
                    .bg(style::white(style::RAISED, cx))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .justify_between()
                            .items_end()
                            .children(self.bands.iter().enumerate().map(|(index, band)| {
                                v_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(style::text_11())
                                            .text_color(readout)
                                            .child(SharedString::from(format!(
                                                "{:+.1}",
                                                gains[index]
                                            ))),
                                    )
                                    .child(Slider::new(band).vertical().disabled(locked))
                                    .child(
                                        div()
                                            .text_size(style::text_11())
                                            .text_color(style::white(BAND_LABEL, cx))
                                            .child(SharedString::from(BAND_LABELS[index])),
                                    )
                            })),
                    ),
            )
    }
}

/// `bg-white/5 border border-white/10 rounded-xl px-3 py-2`: one control box of the top row.
fn panel(cx: &App) -> Div {
    h_flex()
        .min_w_0()
        .gap_2()
        .items_center()
        .px_3()
        .py_2()
        .rounded(style::radius_xl())
        .border_1()
        .border_color(style::white(style::BORDER, cx))
        .bg(style::white(style::RAISED, cx))
}

fn current(cx: &App) -> EqualizerSettings {
    ConfigModel::global(cx)
        .read(cx)
        .config()
        .playback
        .equalizer
        .clone()
}

/// How often a drag may preview: `Change` fires per pointer move, and unlike the config write a preview is not debounced.
const PREVIEW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(40);

/// When the last preview went out.
#[derive(Default)]
struct LastPreview(Option<std::time::Instant>);

impl gpui_kit::Global for LastPreview {}

fn preview_due(cx: &mut App) -> bool {
    let now = std::time::Instant::now();
    let last = cx.default_global::<LastPreview>().0;
    let due = last.is_none_or(|last| now.duration_since(last) >= PREVIEW_INTERVAL);
    if due {
        cx.default_global::<LastPreview>().0 = Some(now);
    }
    due
}

/// Save the curve and preview it; a `settled` drag always previews, or the engine would sit on a stale curve.
fn apply(settings: EqualizerSettings, settled: bool, cx: &mut App) {
    let preview = settings.clone();
    ConfigModel::edit(
        cx,
        SettingsEdit::Playback(PlaybackEdit::Equalizer(settings)),
    );
    if !settled && !preview_due(cx) {
        return;
    }
    Link::global(cx).fire(cx, "preview equalizer", move |api| async move {
        api.preview_equalizer(preview).await
    });
}

fn set_band(index: usize, gain: f32, settled: bool, cx: &mut App) {
    let mut settings = current(cx);
    let mut bands = settings.resolved_bands();
    let Some(band) = bands.get_mut(index) else {
        return;
    };
    *band = gain.clamp(BAND_MIN_DB, BAND_MAX_DB);
    settings.bands = bands;
    settings.preset = EqPreset::Custom;
    apply(settings, settled, cx);
}

/// A preset is one jump, not a drag, so it previews immediately.
fn set_preset(preset: EqPreset, cx: &mut App) {
    let mut settings = current(cx);
    settings.preset = preset;
    if let Some(preamp_db) = preset.default_preamp_db() {
        settings.preamp_db = preamp_db;
    }
    apply(settings, true, cx);
}

fn preset_label(preset: EqPreset) -> SharedString {
    t(match preset {
        EqPreset::Flat => "eq_preset_flat",
        EqPreset::BassBoost => "eq_preset_bass_boost",
        EqPreset::TrebleBoost => "eq_preset_treble_boost",
        EqPreset::VocalBoost => "eq_preset_vocal_boost",
        EqPreset::Loudness => "eq_preset_loudness",
        EqPreset::Custom => "eq_preset_custom",
    })
}
