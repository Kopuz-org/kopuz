use config::UiStyle;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

#[derive(Props, Clone, PartialEq)]
pub struct RadioProps {
    pub config: Signal<config::AppConfig>,
}

#[component]
pub fn Radio(props: RadioProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<config::AppConfig>>();
    let is_modern = config.read().ui_style == UiStyle::Modern;

    let registry = use_context::<Signal<radio::registry::StationRegistry>>();
    // can panic, will check again later.
    let stations: Vec<radio::manifest::StationManifest> = registry
        .read()
        .all_stations()
        .into_iter()
        .cloned()
        .collect();

    // Search / filter
    let mut filter = use_signal(|| String::new());
    let debounce_gen = use_hook(|| Arc::new(AtomicU64::new(0))).clone();

    // Expanded stations set for stream overflow
    let mut expanded_stations = use_signal(|| std::collections::HashSet::<String>::new());

    let query = filter.read().to_lowercase();
    let filtered: Vec<&radio::manifest::StationManifest> = stations
        .iter()
        .filter(|s| {
            if query.is_empty() {
                true
            } else {
                i18n::t(&s.name).to_lowercase().contains(&query)
                    || i18n::t(&s.description).to_lowercase().contains(&query)
                    || s.streams
                        .iter()
                        .any(|st| i18n::t(&st.name).to_lowercase().contains(&query))
            }
        })
        .collect();

    rsx! {
        div {
            class: if is_modern {
                "px-6 pt-6 pb-24 w-full h-full overflow-y-auto"
            } else {
                "p-8 w-full h-full overflow-y-auto"
            },

            if is_modern {
                div { class: "mb-6 flex items-end justify-between",
                    div {
                        p {
                            class: "text-[10px] font-bold tracking-widest uppercase mb-1",
                            style: "color: rgba(255,255,255,0.35);",
                            "{i18n::t(\"discover\")}"
                        }
                        h1 {
                            class: "text-2xl font-bold text-white",
                            "{i18n::t(\"radio\")}"
                        }
                    }
                    // Search — Modern
                    div { class: "relative w-64",
                        i {
                            class: "fa-solid fa-magnifying-glass absolute top-1/2 -translate-y-1/2 text-xs",
                            style: "left: 12px; color: rgba(255,255,255,0.3);",
                        }
                        input {
                            r#type: "text",
                            placeholder: "{i18n::t(\"radio_filter_stations\")}",
                            class: "w-full py-1.5 pr-3 rounded-lg text-xs text-white focus:outline-none transition-colors",
                            style: "padding-left: 2.25rem; background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.08);",
                            oninput: {
                                let debounce_gen = debounce_gen.clone();
                                move |evt| {
                                    let value = evt.value();
                                    let tick = debounce_gen.fetch_add(1, Ordering::Relaxed) + 1;
                                    let dg = debounce_gen.clone();
                                    spawn(async move {
                                        tokio::time::sleep(Duration::from_millis(120)).await;
                                        if dg.load(Ordering::Relaxed) == tick {
                                            filter.set(value);
                                        }
                                    });
                                }
                            },
                            onkeydown: move |e| e.stop_propagation(),
                        }
                    }
                }
            } else {
                div { class: "mb-8 flex items-end justify-between flex-wrap gap-4",
                    div {
                        div { class: "flex items-center gap-3 mb-2",
                            i {
                                class: "fa-solid fa-radio text-2xl",
                                style: "color: var(--color-indigo-400);",
                            }
                            h1 { class: "text-3xl font-bold text-white",
                                "{i18n::t(\"radio\")}"
                            }
                        }
                        p {
                            class: "text-sm",
                            style: "color: var(--color-slate-400);",
                            "{i18n::t(\"radio_subtitle\")}"
                        }
                    }
                    div { class: "relative max-w-sm w-full",
                        i {
                            class: "fa-solid fa-magnifying-glass absolute left-4 top-1/2 -translate-y-1/2",
                            style: "color: var(--color-slate-400);",
                        }
                        input {
                            r#type: "text",
                            placeholder: "{i18n::t(\"radio_filter_stations\")}",
                            class: "w-full bg-white/5 border border-white/10 rounded-full py-2.5 pl-12 pr-4 text-sm text-white focus:outline-none focus:border-white/20 transition-colors",
                            oninput: {
                                let debounce_gen = debounce_gen.clone();
                                move |evt| {
                                    let value = evt.value();
                                    let tick = debounce_gen.fetch_add(1, Ordering::Relaxed) + 1;
                                    let dg = debounce_gen.clone();
                                    spawn(async move {
                                        tokio::time::sleep(Duration::from_millis(120)).await;
                                        if dg.load(Ordering::Relaxed) == tick {
                                            filter.set(value);
                                        }
                                    });
                                }
                            },
                            onkeydown: move |e| e.stop_propagation(),
                        }
                    }
                }
            }

            if filtered.is_empty() {
                div { class: "flex flex-col items-center justify-center py-16 gap-3",
                    i {
                        class: "fa-solid fa-radio text-4xl",
                        style: "color: rgba(255,255,255,0.12);",
                    }
                    p {
                        class: "text-sm",
                        style: "color: rgba(255,255,255,0.3);",
                        "{i18n::t(\"radio_no_stations_match\")}"
                    }
                }
            }

            if is_modern {
                // Modern
                if !filtered.is_empty() {
                    div { class: "flex flex-col",
                        div {
                            class: "grid px-4 py-2 text-[10px] font-bold uppercase tracking-widest border-b mb-1",
                            style: "grid-template-columns: 48px 1fr 1.5fr 180px; color: rgba(255,255,255,0.25); border-color: rgba(255,255,255,0.06);",
                            div {}
                            div { class: "text-left", "{i18n::t(\"radio_station_col\")}" }
                            div { class: "text-left", "{i18n::t(\"radio_description_col\")}" }
                            div { class: "text-right pr-2", "{i18n::t(\"radio_streams_col\")}" }
                        }

                        for station in filtered.iter() {
                            // Outer wrapper — not a grid, expanded row renders below without overlap
                            div {
                                class: "rounded-lg mx-1 group cursor-pointer transition-colors hover:bg-white/[0.04]",
                                onclick: {
                                    let station_id = station.id.clone();
                                    let stream_id = station.streams.first().map(|s| s.id.clone()).unwrap_or_default();
                                    move |_| {
                                        ctrl.play_radio(&station_id, &stream_id);
                                    }
                                },

                                div {
                                    class: "grid items-center px-4 py-2.5",
                                    style: "grid-template-columns: 48px 1fr 1.5fr 180px;",

                                    div { class: "flex items-center justify-center",
                                        div {
                                            class: "w-9 h-9 rounded-lg flex items-center justify-center shrink-0",
                                            style: "background: color-mix(in oklab, var(--color-indigo-500) 15%, transparent);",
                                            i {
                                                class: "{station.icon} text-base",
                                                style: "color: var(--color-indigo-500);",
                                            }
                                        }
                                    }

                                    div { class: "flex items-center min-w-0 pr-4",
                                        span {
                                            class: "text-sm font-semibold truncate text-white",
                                            "{i18n::t(&station.name)}"
                                        }
                                    }

                                    div { class: "flex items-center justify-start text-left min-w-0 pr-4",
                                        span {
                                            class: "text-sm truncate w-full",
                                            style: "color: rgba(255,255,255,0.4);",
                                            "{i18n::t(&station.description)}"
                                        }
                                    }

                                    div { class: "flex items-center gap-2 justify-end min-w-0",
                                        if station.streams.len() == 1 {
                                            button {
                                                class: "inline-flex items-center justify-center w-8 h-8 rounded-full transition-all opacity-0 group-hover:opacity-100",
                                                style: "background: color-mix(in oklab, var(--color-indigo-500) 20%, transparent); color: var(--color-indigo-400);",
                                                onclick: {
                                                    let station_id = station.id.clone();
                                                    let stream_id = station.streams.first().map(|s| s.id.clone()).unwrap_or_default();
                                                    move |evt: MouseEvent| {
                                                        evt.stop_propagation();
                                                        ctrl.play_radio(&station_id, &stream_id);
                                                    }
                                                },
                                                i { class: "fa-solid fa-play text-xs" }
                                            }
                                        } else {
                                            if station.streams.len() == 2 {
                                                for stream in &station.streams {
                                                    button {
                                                        class: "inline-flex items-center gap-2 h-8 px-4 rounded-full text-sm font-medium transition-all hover:opacity-90 active:scale-95 whitespace-nowrap",
                                                        style: "background: color-mix(in oklab, var(--color-indigo-500) 20%, transparent); color: var(--color-indigo-400); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 30%, transparent);",
                                                        onclick: {
                                                            let station_id = station.id.clone();
                                                            let stream_id = stream.id.clone();
                                                            move |evt: MouseEvent| {
                                                                evt.stop_propagation();
                                                                ctrl.play_radio(&station_id, &stream_id);
                                                            }
                                                        },
                                                        i { class: "{stream.icon.as_deref().unwrap_or(\"fa-solid fa-play\")} text-xs" }
                                                        "{i18n::t(&stream.name)}"
                                                    }
                                                }
                                            } else {
                                                if expanded_stations.read().contains(&station.id) {
                                                    button {
                                                        class: "inline-flex items-center justify-center w-8 h-8 rounded-full transition-all shrink-0 hover:opacity-80",
                                                        style: "background: rgba(255,255,255,0.06); color: rgba(255,255,255,0.4);",
                                                        onclick: {
                                                            let station_id = station.id.clone();
                                                            move |evt: MouseEvent| {
                                                                evt.stop_propagation();
                                                                expanded_stations.write().remove(&station_id);
                                                            }
                                                        },
                                                        i { class: "fa-solid fa-chevron-up text-xs" }
                                                    }
                                                } else {
                                                    if let Some(first) = station.streams.first() {
                                                        button {
                                                            class: "inline-flex items-center gap-2 h-8 px-4 rounded-full text-sm font-medium transition-all hover:opacity-90 active:scale-95 whitespace-nowrap",
                                                            style: "background: color-mix(in oklab, var(--color-indigo-500) 20%, transparent); color: var(--color-indigo-400); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 30%, transparent);",
                                                            onclick: {
                                                                let station_id = station.id.clone();
                                                                let stream_id = first.id.clone();
                                                                move |evt: MouseEvent| {
                                                                    evt.stop_propagation();
                                                                    ctrl.play_radio(&station_id, &stream_id);
                                                                }
                                                            },
                                                            i { class: "{first.icon.as_deref().unwrap_or(\"fa-solid fa-play\")} text-xs" }
                                                            "{i18n::t(&first.name)}"
                                                        }
                                                    }
                                                    button {
                                                        class: "inline-flex items-center justify-center h-8 px-3 rounded-full text-xs font-semibold transition-all hover:opacity-80 shrink-0 whitespace-nowrap",
                                                        style: "background: rgba(255,255,255,0.06); color: rgba(255,255,255,0.5); border: 1px solid rgba(255,255,255,0.08);",
                                                        onclick: {
                                                            let station_id = station.id.clone();
                                                            move |evt: MouseEvent| {
                                                                evt.stop_propagation();
                                                                expanded_stations.write().insert(station_id.clone());
                                                            }
                                                        },
                                                        "+{station.streams.len() - 1}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Expanded stream row — full width below grid, no overlap possible
                                if expanded_stations.read().contains(&station.id) {
                                    div {
                                        class: "flex flex-wrap items-center gap-2 px-4 pb-3",
                                        style: "padding-left: calc(48px + 1rem);",
                                        for stream in &station.streams {
                                            button {
                                                class: "inline-flex items-center gap-2 h-8 px-4 rounded-full text-sm font-medium transition-all hover:opacity-90 active:scale-95 whitespace-nowrap",
                                                style: "background: color-mix(in oklab, var(--color-indigo-500) 20%, transparent); color: var(--color-indigo-400); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 30%, transparent);",
                                                onclick: {
                                                    let station_id = station.id.clone();
                                                    let stream_id = stream.id.clone();
                                                    move |evt: MouseEvent| {
                                                        evt.stop_propagation();
                                                        ctrl.play_radio(&station_id, &stream_id);
                                                    }
                                                },
                                                i { class: "{stream.icon.as_deref().unwrap_or(\"fa-solid fa-play\")} text-xs" }
                                                "{i18n::t(&stream.name)}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                // Normal
                if !filtered.is_empty() {
                    div { class: "grid grid-cols-1 lg:grid-cols-2 gap-4",
                        for station in filtered.iter() {
                            div {
                                key: "{station.id}",
                                class: "group relative rounded-2xl overflow-hidden border transition-all duration-300 cursor-pointer hover:border-white/15",
                                style: "border-color: rgba(255,255,255,0.06); background: rgba(255,255,255,0.03);",
                                onclick: {
                                    let station_id = station.id.clone();
                                    let stream_id = station.streams.first().map(|s| s.id.clone()).unwrap_or_default();
                                    move |_| {
                                        ctrl.play_radio(&station_id, &stream_id);
                                    }
                                },

                                div { class: "p-5 flex items-start gap-4",
                                    div {
                                        class: "w-12 h-12 rounded-xl flex items-center justify-center shrink-0 transition-transform group-hover:scale-105",
                                        style: "background: color-mix(in oklab, var(--color-indigo-500) 15%, transparent);",
                                        i {
                                            class: "{station.icon} text-xl",
                                            style: "color: var(--color-indigo-400);",
                                        }
                                    }

                                    div { class: "flex-1 min-w-0",
                                        h2 {
                                            class: "text-lg font-bold text-white mb-0.5 truncate",
                                            "{i18n::t(&station.name)}"
                                        }
                                        p {
                                            class: "text-xs mb-3 leading-relaxed",
                                            style: "color: var(--color-slate-400);",
                                            "{i18n::t(&station.description)}"
                                        }

                                        if station.streams.len() > 1 {
                                            div { class: "flex flex-wrap items-center gap-2",
                                                for stream in &station.streams {
                                                    button {
                                                        class: "px-3 py-1.5 rounded-lg text-xs font-medium transition-all duration-200 flex items-center gap-1.5 hover:scale-[1.02] active:scale-95 whitespace-nowrap",
                                                        style: "background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 25%, transparent); color: var(--color-indigo-400);",
                                                        onclick: {
                                                            let station_id = station.id.clone();
                                                            let stream_id = stream.id.clone();
                                                            move |evt: MouseEvent| {
                                                                evt.stop_propagation();
                                                                ctrl.play_radio(&station_id, &stream_id);
                                                            }
                                                        },
                                                        i { class: "{stream.icon.as_deref().unwrap_or(\"fa-solid fa-play\")} text-xs" }
                                                        "{i18n::t(&stream.name)}"
                                                    }
                                                }
                                            }
                                        } else {
                                            div {
                                                class: "flex items-center gap-2 text-sm font-medium",
                                                style: "color: var(--color-indigo-400);",
                                                i { class: "fa-solid fa-play text-xs" }
                                                "{i18n::t(\"radio_play\")}"
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
    }
}
