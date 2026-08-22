//! Interactive equalizer settings and graph rendering.

use config::{AppConfig, EqPreset, EqualizerSettings as EqualizerConfig};
use dioxus::prelude::*;

use super::AppSelect;

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
