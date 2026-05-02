use config::{
    AppConfig, BackBehavior, EqPreset, EqualizerSettings as EqualizerConfig, MusicServer,
};
use dioxus::prelude::*;
use rfd::AsyncFileDialog;

#[component]
pub fn SettingItem(title: String, control: Element) -> Element {
    rsx! {
        div { class: "flex items-center justify-between py-2",
            p { class: "text-white font-medium", "{title}" }
            {control}
        }
    }
}

#[component]
pub fn LanguageSelector(current_language: String, on_change: EventHandler<String>) -> Element {
    rsx! {
        select {
            class: "bg-white/5 border border-white/10 rounded px-3 py-1 text-sm text-white focus:outline-none focus:border-white/20",
            onchange: move |evt| on_change.call(evt.value()),
            for (code, name) in i18n::available_languages() {
                option {
                    value: *code,
                    selected: *code == current_language.as_str(),
                    "{name}"
                }
            }
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

    rsx! {
        select {
            class: "bg-white/5 border border-white/10 rounded px-3 py-1 text-sm text-white focus:outline-none focus:border-white/20",
            value: "{current_theme}",
            onchange: move |evt| on_change.call(evt.value()),
            optgroup { label: "{i18n::t(\"theme_group_dynamic\")}",
                option { value: "album-art", "{i18n::t(\"album_art_gradient\")}" }
            }
            optgroup { label: "{i18n::t(\"theme_group_dark\")}",
                option { value: "default", "{i18n::t(\"default_theme\")}" }
                option { value: "gruvbox", "{i18n::t(\"gruvbox_material\")}" }
                option { value: "gruvbox-classic", "{i18n::t(\"gruvbox_classic\")}" }
                option { value: "gruvbox-dark-soft", "{i18n::t(\"gruvbox_dark_soft\")}" }
                option { value: "dracula", "{i18n::t(\"dracula\")}" }
                option { value: "nord", "{i18n::t(\"nord\")}" }
                option { value: "catppuccin", "{i18n::t(\"catppuccin_mocha\")}" }
                option { value: "ef-night", "{i18n::t(\"ef_night\")}" }
                option { value: "ayu-dark", "{i18n::t(\"ayu_dark\")}" }
                option { value: "ayu-mirage", "{i18n::t(\"ayu_mirage\")}" }
                option { value: "vague", "{i18n::t(\"vague\")}" }
                option { value: "onedarkpro", "{i18n::t(\"one_dark_pro\")}" }
                option { value: "osmium", "{i18n::t(\"osmium\")}" }
                option { value: "kanagawa-dragon", "{i18n::t(\"kanagawa_dragon\")}" }
                option { value: "everforest", "{i18n::t(\"everforest\")}" }
                option { value: "rosepine", "{i18n::t(\"rosepine\")}" }
                option { value: "kettek16", "kettek16" }
            }
            optgroup { label: "{i18n::t(\"theme_group_light\")}",
                option { value: "default-light", "{i18n::t(\"default_light\")}" }
                option { value: "catppuccin-latte", "{i18n::t(\"catppuccin_latte\")}" }
                option { value: "rosepine-dawn", "{i18n::t(\"rosepine_dawn\")}" }
                option { value: "everforest-light", "{i18n::t(\"everforest_light\")}" }
                option { value: "ayu-light", "{i18n::t(\"ayu_light\")}" }
                option { value: "one-light", "{i18n::t(\"one_light\")}" }
                option { value: "gruvbox-light", "{i18n::t(\"gruvbox_light_soft\")}" }
            }
            if !custom.is_empty() {
                optgroup { label: "{i18n::t(\"theme_group_custom\")}",
                    for (id, name) in &custom {
                        option { value: "{id}", "{name}" }
                    }
                }
            }
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
            if !cfg!(target_arch = "wasm32") {
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
    }
}

#[component]
pub fn ServerSettings(
    server: Option<MusicServer>,
    on_add: EventHandler<()>,
    on_delete: EventHandler<()>,
    on_login: EventHandler<()>,
) -> Element {
    let login_text = i18n::t("login");
    let delete_text = i18n::t("delete");

    rsx! {
        div { class: "flex flex-col gap-2",
            if let Some(server) = server {
                div { class: "flex items-center justify-between gap-4 bg-white/5 p-2 rounded w-full",
                    div {
                        p { class: "text-sm font-medium text-white", "{server.name}" }
                        p { class: "text-xs text-white/60", "{i18n::t_with(\"service\", &[(\"name\", server.service.display_name().to_string())])}" }
                        p { class: "text-xs text-white/60", "{server.url}" }
                        if server.access_token.is_some() {
                            p { class: "text-xs text-green-400 mt-1", "{i18n::t(\"connected\")}" }
                        } else {
                            div { class: "flex items-center gap-2 mt-1",
                                p { class: "text-xs text-red-400", "{i18n::t(\"disconnected\")}" }
                                button {
                                    onclick: move |_| on_login.call(()),
                                    class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-0.5 rounded text-white transition-colors",
                                    "{login_text}"
                                }
                            }
                        }
                    }
                    button {
                        onclick: move |_| on_delete.call(()),
                        class: "text-red-400 hover:text-red-300 text-sm px-2 py-1 transition-colors",
                        "{delete_text}"
                    }
                }
            } else {
                button {
                    onclick: move |_| on_add.call(()),
                    class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
                    "{i18n::t(\"add_server\")}"
                }
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

const EQ_MIN_DB: f64 = -12.0;
const EQ_MAX_DB: f64 = 12.0;
const EQ_GRAPH_WIDTH: f64 = 760.0;
const EQ_GRAPH_HEIGHT: f64 = 280.0;
const EQ_GRAPH_PAD_X: f64 = 36.0;
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
    const BAND_LABELS: [&str; 5] = ["60 Hz", "250 Hz", "1 kHz", "4 kHz", "12 kHz"];

    let mut draft = use_signal(|| current.clone());
    let mut dragging_band = use_signal(|| None::<usize>);
    let enabled = draft.read().enabled;
    let resolved_bands = draft.read().resolved_bands();
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

    rsx! {
        div { class: "flex flex-col gap-4 w-full",
            div { class: "flex flex-wrap items-center gap-3",
                div {
                    class: "bg-white/5 p-1 rounded-xl flex relative h-10 items-center border border-white/5 w-48",
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

                div { class: "flex items-center gap-2 bg-white/5 border border-white/10 rounded-xl px-3 py-2",
                    span { class: "text-xs uppercase tracking-[0.18em] text-slate-400", "{i18n::t(\"eq_preset\")}" }
                    select {
                        class: "bg-transparent text-sm text-white focus:outline-none",
                        value: "{draft.read().preset.as_storage()}",
                        onchange: move |evt| {
                            let mut next = draft.peek().clone();
                            next.preset = EqPreset::from_storage(&evt.value());
                            draft.set(next.clone());
                            on_preview.call(next.clone());
                            on_commit.call(next);
                        },
                        for preset in EqPreset::all() {
                            option {
                                value: "{preset.as_storage()}",
                                selected: preset == draft.read().preset,
                                "{eq_preset_label(preset)}"
                            }
                        }
                    }
                }

                div { class: "flex items-center gap-3 bg-white/5 border border-white/10 rounded-xl px-3 py-2 min-w-[220px] flex-1",
                    div { class: "min-w-0",
                        p { class: "text-xs uppercase tracking-[0.18em] text-slate-400", "{i18n::t(\"eq_preamp\")}" }
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
                class: "rounded-2xl border border-white/8 bg-white/5 p-4 select-none overflow-x-auto",
                style: "background: color-mix(in oklab, var(--color-neutral-900) 78%, transparent); border-color: color-mix(in oklab, var(--color-white) 8%, transparent);",
                svg {
                    class: "block mx-auto cursor-row-resize",
                    style: "width: 760px; height: 280px; min-width: 760px;",
                    view_box: "0 0 760 280",
                    onmousedown: move |evt: MouseEvent| {
                        let point = evt.element_coordinates();
                        let index = eq_nearest_band(point.x, BAND_LABELS.len());
                        dragging_band.set(Some(index));
                        let next = eq_apply_drag(&draft.peek().clone(), index, point.y);
                        draft.set(next.clone());
                        on_preview.call(next);
                    },
                    onmousemove: move |evt: MouseEvent| {
                        if let Some(index) = *dragging_band.read() {
                            let point = evt.element_coordinates();
                            let next = eq_apply_drag(&draft.peek().clone(), index, point.y);
                            draft.set(next.clone());
                            on_preview.call(next);
                        }
                    },
                    onmouseup: move |_| {
                        if dragging_band.peek().is_some() {
                            on_commit.call(draft.peek().clone());
                        }
                        dragging_band.set(None);
                    },
                    onmouseleave: move |_| {
                        if dragging_band.peek().is_some() {
                            on_commit.call(draft.peek().clone());
                        }
                        dragging_band.set(None);
                    },
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
                        d: "{graph_path}",
                        fill: "none",
                        stroke_width: "2.5",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        style: "stroke: var(--color-indigo-500);",
                    }
                    for (index, gain) in resolved_bands.iter().enumerate() {
                        circle {
                            cx: "{eq_band_x(index, BAND_LABELS.len())}",
                            cy: "{eq_gain_to_y(*gain)}",
                            r: if *dragging_band.read() == Some(index) { "8" } else { "6" },
                            style: if *dragging_band.read() == Some(index) {
                                "fill: var(--color-indigo-400);"
                            } else {
                                "fill: var(--color-white);"
                            },
                        }
                        circle {
                            cx: "{eq_band_x(index, BAND_LABELS.len())}",
                            cy: "{eq_gain_to_y(*gain)}",
                            r: "14",
                            fill: "transparent",
                            stroke_width: "1",
                            style: if *dragging_band.read() == Some(index) {
                                "stroke: color-mix(in oklab, var(--color-indigo-400) 40%, transparent);"
                            } else {
                                "stroke: color-mix(in oklab, var(--color-white) 10%, transparent);"
                            },
                        }
                    }
                }

            }

        }
    }
}

// #[component]
// pub fn LastFmSettings(current: String, on_save: EventHandler<String>) -> Element {
//     let mut input = use_signal(move || current.clone());

//     rsx! {
//         div { class: "flex items-center gap-2 w-full max-w-xl",
//             div { class: "flex-1 bg-white/5 p-1 rounded-xl border border-white/5",
//                 input {
//                     class: "bg-transparent w-full px-3 py-2 text-sm text-white placeholder:text-white/50 outline-none",
//                     placeholder: "Enter your last.fm token",
//                     value: "{input()}",
//                     oninput: move |evt| {
//                         input.set(evt.value());
//                         on_save.call(evt.value());
//                     },
//                     r#type: "text",
//                 }
//             }
//         }
//     }
// }

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
