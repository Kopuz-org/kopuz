use config::UiStyle;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use radio::browser::{self, BrowserStation};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

#[derive(Props, Clone, PartialEq)]
pub struct RadioProps {
    pub config: Signal<config::AppConfig>,
}

/// Insert the station into the live registry,
/// report the play to radio-browser, and start it.
fn play_browser_station(
    ctrl: &mut PlayerController,
    registry: &mut Signal<radio::registry::StationRegistry>,
    station: &BrowserStation,
) {
    let manifest = browser::to_manifest(station);
    let station_id = manifest.id.clone();
    registry.write().insert_manifest(manifest);
    browser::count_click(&station.stationuuid);
    ctrl.play_radio(&station_id, browser::BROWSER_STREAM_ID);
}

/// Pin or unpin a browser station from the Selected list, persisted in config.
fn toggle_pin_station(
    config: &mut Signal<config::AppConfig>,
    registry: &mut Signal<radio::registry::StationRegistry>,
    station: &BrowserStation,
) {
    let manifest = browser::to_manifest(station);
    let id = manifest.id.clone();
    if registry.read().is_registry_station(&id) {
        registry.write().unpin_station(&id);
        config.write().pinned_stations.retain(|json| {
            serde_json::from_str::<radio::manifest::StationManifest>(json)
                .map(|m| m.id != id)
                .unwrap_or(false)
        });
    } else {
        if let Ok(json) = serde_json::to_string(&manifest) {
            config.write().pinned_stations.push(json);
        }
        registry.write().pin_manifest(manifest);
    }
}

#[component]
pub fn Radio(props: RadioProps) -> Element {
    let _ = &props;
    let mut ctrl = use_context::<PlayerController>();
    let config = props.config;
    let is_vaxry = config.read().ui_style == UiStyle::Vaxry;

    let registry_sig = use_context::<Signal<radio::registry::StationRegistry>>();
    // can panic, will check again later.
    // Curated registries only; runtime radio-browser inserts excluded.
    let stations: Vec<radio::manifest::StationManifest> = registry_sig
        .read()
        .registry_stations()
        .into_iter()
        .cloned()
        .collect();

    // Search / filter
    let mut filter = use_signal(String::new);
    let debounce_gen = use_hook(|| Arc::new(AtomicU64::new(0))).clone();

    // Expanded stations set for stream overflow
    let mut expanded_stations = use_signal(std::collections::HashSet::<String>::new);

    // radio-browser.info: popular stations is default,
    // live search once the debounced filter has text.
    let browser_res: Resource<Result<Vec<BrowserStation>, String>> = use_resource(move || {
        let query = filter();
        utils::offload(async move {
            let q = query.trim().to_string();
            let result = if q.is_empty() {
                browser::top_stations(60).await
            } else {
                browser::search(&q, 60).await
            };
            result.map_err(|e| e.to_string())
        })
    });

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
                    || s.tags.iter().any(|t| t.to_lowercase().contains(&query))
            }
        })
        .collect();
    let has_custom = !filtered.is_empty();
    let searching = !query.is_empty();

    // Resource keeps its stale value while refetching,
    // track pending separately for the search spinner.
    let browser_loading = matches!(
        *browser_res.state().read(),
        UseResourceState::Pending | UseResourceState::Paused
    );
    let browser_state = browser_res.read();

    rsx! {
            div {
                class: if cfg!(target_os = "android") {
                    "px-4 pt-2 pb-28 w-full h-full overflow-y-auto"
                } else if is_vaxry {
                    "px-6 pt-6 pb-24 w-full h-full overflow-y-auto"
                } else {
                    "p-8 w-full h-full overflow-y-auto"
                },

                if is_vaxry {
                    div { class: "mb-6 flex items-end justify-between",
                        div {
                            p {
                                class: "text-[10px] font-bold mb-1",
                                style: "color: rgba(255,255,255,0.35);",
                                "{i18n::t(\"discover\")}"
                            }
                            h1 {
                                class: "text-2xl font-bold text-white",
                                "{i18n::t(\"radio\")}"
                            }
                        }
                        // Search — Vaxry
                        div { class: "relative w-64",
                            i {
                                class: "fa-solid fa-magnifying-glass absolute top-1/2 -translate-y-1/2 text-xs",
                                style: "left: 12px; color: rgba(255,255,255,0.3);",
                            }
                            input {
                                r#type: "text",
                                placeholder: "{i18n::t(\"radio_search_stations\")}",
                                class: "w-full py-1.5 pr-3 rounded-lg text-xs text-white focus:outline-none transition-colors",
                                style: "padding-left: 2.25rem; background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.08);",
                                oninput: {
                                    let debounce_gen = debounce_gen.clone();
                                    move |evt| {
                                        let value = evt.value();
                                        let tick = debounce_gen.fetch_add(1, Ordering::Relaxed) + 1;
                                        let dg = debounce_gen.clone();
                                        spawn(async move {
                                            tokio::time::sleep(Duration::from_millis(300)).await;
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
                                placeholder: "{i18n::t(\"radio_search_stations\")}",
                                class: "w-full bg-white/5 border border-white/10 rounded-full py-2.5 pl-12 pr-4 text-sm text-white focus:outline-none focus:border-white/20 transition-colors",
                                oninput: {
                                    let debounce_gen = debounce_gen.clone();
                                    move |evt| {
                                        let value = evt.value();
                                        let tick = debounce_gen.fetch_add(1, Ordering::Relaxed) + 1;
                                        let dg = debounce_gen.clone();
                                        spawn(async move {
                                            tokio::time::sleep(Duration::from_millis(300)).await;
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

                // ── Custom registry stations (user-added registries) ────────────
                if has_custom {
                    h2 {
                        class: if is_vaxry { "text-[10px] font-bold mb-2" } else { "text-sm font-bold mb-3 uppercase tracking-wider" },
                        style: if is_vaxry { "color: rgba(255,255,255,0.35);" } else { "color: var(--color-slate-400);" },
                        "{i18n::t(\"radio_selected\")}"
                    }
                }

                if is_vaxry {
                    // Vaxry
                    if has_custom {
                        div { class: "flex flex-col mb-8",
                            div {
                                class: "grid px-4 py-2 text-[10px] font-bold border-b mb-1",
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

                                        div { class: "flex items-center justify-start text-left min-w-0 pr-4 gap-2",
                                            span {
                                                class: "text-sm truncate",
                                                style: "color: rgba(255,255,255,0.4);",
                                                "{i18n::t(&station.description)}"
                                            }
                                            for tag in station.tags.iter().take(2) {
                                                span {
                                                    class: "px-3 py-1.5 rounded-lg text-xs font-medium flex items-center gap-1.5 shrink-0 whitespace-nowrap",
    style: "background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 25%, transparent); color: var(--color-indigo-400);",
                                                    i { class: "fa-solid fa-music text-xs" }
                                                    "{tag}"
                                                }
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
                    if has_custom {
                        div { class: "grid grid-cols-1 lg:grid-cols-2 gap-4 mb-8",
                            for station in filtered.iter() {
                                div {
                                    key: "{station.id}",
                                    class: "group relative rounded-lg overflow-hidden border transition-all duration-300 cursor-pointer hover:border-white/15",
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

                                            if !station.tags.is_empty() {
                                                div { class: "flex flex-wrap items-center gap-1.5 mb-3",
                                                    for tag in station.tags.iter().take(4) {
                                                        span {
                                                            class: "px-3 py-1.5 rounded-lg text-xs font-medium flex items-center gap-1.5 whitespace-nowrap",
    style: "background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 25%, transparent); color: var(--color-indigo-400);",
                                                            i { class: "fa-solid fa-music text-xs" }
                                                            "{tag}"
                                                        }
                                                    }
                                                }
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

                // radio-browser.info directory
                div { class: "flex items-end justify-between mb-3",
                    h2 {
                        class: if is_vaxry { "text-[10px] font-bold" } else { "text-sm font-bold uppercase tracking-wider" },
                        style: if is_vaxry { "color: rgba(255,255,255,0.35);" } else { "color: var(--color-slate-400);" },
                        if searching {
                            "{i18n::t(\"radio_search_results\")}"
                        } else {
                            "{i18n::t(\"radio_top_stations\")}"
                        }
                    }
                    span {
                        class: "text-[10px]",
                        style: "color: rgba(255,255,255,0.25);",
                        "{i18n::t(\"radio_powered_by\")}"
                    }
                }

                match (browser_loading, &*browser_state) {
                    (true, _) | (false, None) => rsx! {
                        div { class: "flex items-center justify-center py-16 gap-3",
                            i {
                                class: "fa-solid fa-circle-notch fa-spin",
                                style: "color: rgba(255,255,255,0.3);",
                            }
                            p {
                                class: "text-sm",
                                style: "color: rgba(255,255,255,0.3);",
                                "{i18n::t(\"radio_loading_stations\")}"
                            }
                        }
                    },
                    (_, Some(Err(e))) => rsx! {
                        div { class: "flex flex-col items-center justify-center py-16 gap-3",
                            i {
                                class: "fa-solid fa-tower-broadcast text-4xl",
                                style: "color: rgba(255,255,255,0.12);",
                            }
                            p {
                                class: "text-sm",
                                style: "color: rgba(255,255,255,0.3);",
                                "{i18n::t(\"radio_search_failed\")}"
                            }
                            p {
                                class: "text-xs",
                                style: "color: rgba(255,255,255,0.2);",
                                "{e}"
                            }
                        }
                    },
                    (_, Some(Ok(results))) if results.is_empty() => rsx! {
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
                    },
                    (_, Some(Ok(results))) => {
                        if is_vaxry {
                            rsx! {
                                div { class: "flex flex-col",
                                    for st in results.iter() {
                                        div {
                                            key: "{st.stationuuid}",
                                            class: "grid items-center px-4 py-2.5 rounded-lg mx-1 group cursor-pointer transition-colors hover:bg-white/[0.04]",
                                            style: "grid-template-columns: 48px 1fr 1.5fr 180px;",
                                            onclick: {
                                                let st = st.clone();
                                                let mut registry_sig = registry_sig;
                                                move |_| {
                                                    play_browser_station(&mut ctrl, &mut registry_sig, &st);
                                                }
                                            },

                                            div { class: "flex items-center justify-center",
                                                if st.favicon.starts_with("https://") {
                                                    img {
                                                        src: "{st.favicon}",
                                                        class: "w-9 h-9 rounded-lg object-cover shrink-0",
                                                        style: "background: rgba(255,255,255,0.05);",
                                                        decoding: "async", loading: "lazy",
                                                    }
                                                } else {
                                                    div {
                                                        class: "w-9 h-9 rounded-lg flex items-center justify-center shrink-0",
                                                        style: "background: color-mix(in oklab, var(--color-indigo-500) 15%, transparent);",
                                                        i {
                                                            class: "fa-solid fa-radio text-base",
                                                            style: "color: var(--color-indigo-500);",
                                                        }
                                                    }
                                                }
                                            }

                                            div { class: "flex items-center min-w-0 pr-4",
                                                span {
                                                    class: "text-sm font-semibold truncate text-white",
                                                    "{st.name.trim()}"
                                                }
                                            }

                                            div { class: "flex items-center justify-start text-left min-w-0 pr-4 gap-2",
                                                span {
                                                    class: "text-sm truncate",
                                                    style: "color: rgba(255,255,255,0.4);",
                                                    "{browser::station_detail(st)}"
                                                }
                                                for tag in st.tags.split(',').map(str::trim).filter(|t| !t.is_empty()).take(2) {
                                                    span {
                                                        class: "px-3 py-1.5 rounded-lg text-xs font-medium flex items-center gap-1.5 shrink-0 whitespace-nowrap",
    style: "background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 25%, transparent); color: var(--color-indigo-400);",
                                                        i { class: "fa-solid fa-music text-xs" }
                                                        "{tag}"
                                                    }
                                                }
                                            }

                                            div { class: "flex items-center gap-2 justify-end min-w-0",
                                                button {
                                                    class: if registry_sig.read().is_registry_station(&st.stationuuid) {
                                                        "inline-flex items-center justify-center w-8 h-8 rounded-full transition-all"
                                                    } else {
                                                        "inline-flex items-center justify-center w-8 h-8 rounded-full transition-all opacity-0 group-hover:opacity-100"
                                                    },
                                                    style: "background: rgba(255,255,255,0.06); color: rgba(255,255,255,0.5);",
                                                    onclick: {
                                                        let st = st.clone();
                                                        let mut registry_sig = registry_sig;
                                                        let mut config = config;
                                                        move |evt: MouseEvent| {
                                                            evt.stop_propagation();
                                                            toggle_pin_station(&mut config, &mut registry_sig, &st);
                                                        }
                                                    },
                                                    if registry_sig.read().is_registry_station(&st.stationuuid) {
                                                        i { class: "fa-solid fa-check text-xs" }
                                                    } else {
                                                        i { class: "fa-solid fa-plus text-xs" }
                                                    }
                                                }
                                                button {
                                                    class: "inline-flex items-center justify-center w-8 h-8 rounded-full transition-all opacity-0 group-hover:opacity-100",
                                                    style: "background: color-mix(in oklab, var(--color-indigo-500) 20%, transparent); color: var(--color-indigo-400);",
                                                    onclick: {
                                                        let st = st.clone();
                                                        let mut registry_sig = registry_sig;
                                                        move |evt: MouseEvent| {
                                                            evt.stop_propagation();
                                                            play_browser_station(&mut ctrl, &mut registry_sig, &st);
                                                        }
                                                    },
                                                    i { class: "fa-solid fa-play text-xs" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            rsx! {
                                div { class: "grid grid-cols-1 lg:grid-cols-2 gap-4",
                                    for st in results.iter() {
                                        div {
                                            key: "{st.stationuuid}",
                                            class: "group relative rounded-lg overflow-hidden border transition-all duration-300 cursor-pointer hover:border-white/15",
                                            style: "border-color: rgba(255,255,255,0.06); background: rgba(255,255,255,0.03);",
                                            onclick: {
                                                let st = st.clone();
                                                let mut registry_sig = registry_sig;
                                                move |_| {
                                                    play_browser_station(&mut ctrl, &mut registry_sig, &st);
                                                }
                                            },

                                            div { class: "p-5 flex items-start gap-4",
                                                if st.favicon.starts_with("https://") {
                                                    img {
                                                        src: "{st.favicon}",
                                                        class: "w-12 h-12 rounded-xl object-cover shrink-0 transition-transform group-hover:scale-105",
                                                        style: "background: rgba(255,255,255,0.05);",
                                                        decoding: "async", loading: "lazy",
                                                    }
                                                } else {
                                                    div {
                                                        class: "w-12 h-12 rounded-xl flex items-center justify-center shrink-0 transition-transform group-hover:scale-105",
                                                        style: "background: color-mix(in oklab, var(--color-indigo-500) 15%, transparent);",
                                                        i {
                                                            class: "fa-solid fa-radio text-xl",
                                                            style: "color: var(--color-indigo-400);",
                                                        }
                                                    }
                                                }

                                                div { class: "flex-1 min-w-0",
                                                    h2 {
                                                        class: "text-lg font-bold text-white mb-0.5 truncate",
                                                        "{st.name.trim()}"
                                                    }
                                                    p {
                                                        class: "text-xs mb-3 leading-relaxed truncate",
                                                        style: "color: var(--color-slate-400);",
                                                        "{browser::station_detail(st)}"
                                                    }
                                                    if st.tags.split(',').any(|t| !t.trim().is_empty()) {
                                                        div { class: "flex flex-wrap items-center gap-1.5 mb-3",
                                                            for tag in st.tags.split(',').map(str::trim).filter(|t| !t.is_empty()).take(4) {
                                                                span {
                                                                    class: "px-3 py-1.5 rounded-lg text-xs font-medium flex items-center gap-1.5 whitespace-nowrap",
    style: "background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 25%, transparent); color: var(--color-indigo-400);",
                                                                    i { class: "fa-solid fa-music text-xs" }
                                                                    "{tag}"
                                                                }
                                                            }
                                                        }
                                                    }
                                                    div {
                                                        class: "flex items-center gap-2 text-sm font-medium",
                                                        style: "color: var(--color-indigo-400);",
                                                        i { class: "fa-solid fa-play text-xs" }
                                                        "{i18n::t(\"radio_play\")}"
                                                    }
                                                }

                                                button {
                                                    class: "inline-flex items-center justify-center w-8 h-8 rounded-full transition-all shrink-0 hover:opacity-80",
                                                    style: "background: rgba(255,255,255,0.06); color: rgba(255,255,255,0.5);",
                                                    onclick: {
                                                        let st = st.clone();
                                                        let mut registry_sig = registry_sig;
                                                        let mut config = config;
                                                        move |evt: MouseEvent| {
                                                            evt.stop_propagation();
                                                            toggle_pin_station(&mut config, &mut registry_sig, &st);
                                                        }
                                                    },
                                                    if registry_sig.read().is_registry_station(&st.stationuuid) {
                                                        i { class: "fa-solid fa-check text-xs" }
                                                    } else {
                                                        i { class: "fa-solid fa-plus text-xs" }
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
}
