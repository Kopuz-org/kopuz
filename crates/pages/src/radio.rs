//! Internet radio: the selected stations and the public directory.
//!
//! Both lists are the daemon's. It holds the registry, searches the directory
//! and remembers the pins, so a station plays by naming it and one of its
//! streams; nothing here imports a registry or builds a stream URL.

use api::RadioStationInfo;
use config::UiStyle;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

#[derive(Props, Clone, PartialEq)]
pub struct RadioProps {
    pub config: Signal<config::AppConfig>,
}

/// Pin or unpin, then re-read the selected list. The daemon keeps the
/// station's manifest, so a pin outlives a registry that stops listing it.
///
/// The local set moves first and is put back on failure: the answer is a round
/// trip away and a check mark that waits for it reads as a dead button.
fn toggle_pin(id: String, mut pinned_ids: Signal<HashSet<String>>, mut generation: Signal<u64>) {
    let pinned = pinned_ids.peek().contains(&id);
    if pinned {
        pinned_ids.write().remove(&id);
    } else {
        pinned_ids.write().insert(id.clone());
    }
    let api = hooks::consume_api();
    spawn(async move {
        if let Err(error) = api.pin_radio_station(id.clone(), !pinned).await {
            tracing::warn!(%error, "pinning a station failed");
            hooks::toast::toast_error(&error.to_string());
            if pinned {
                pinned_ids.write().insert(id);
            } else {
                pinned_ids.write().remove(&id);
            }
            return;
        }
        generation += 1;
    });
}

/// The stream a click plays when the row does not name one.
fn first_stream(station: &RadioStationInfo) -> &str {
    station
        .streams
        .first()
        .map(|stream| stream.id.as_str())
        .unwrap_or_default()
}

/// The station's own picture, where it has one.
fn station_art(station: &RadioStationInfo) -> Option<utils::CoverUrl> {
    hooks::artwork::url(station.artwork.as_ref(), hooks::artwork::Size::Thumb)
}

fn matches_query(station: &RadioStationInfo, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    i18n::t(&station.name).to_lowercase().contains(query)
        || i18n::t(&station.description).to_lowercase().contains(query)
        || station
            .streams
            .iter()
            .any(|stream| i18n::t(&stream.name).to_lowercase().contains(query))
        || station
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query))
}

#[component]
pub fn Radio(props: RadioProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let config = props.config;
    let is_vaxry = config.read().ui_style == UiStyle::Vaxry;
    let api = hooks::use_api();

    // Search / filter
    let mut filter = use_signal(String::new);
    let debounce_gen = use_hook(|| Arc::new(AtomicU64::new(0))).clone();

    // Expanded stations set for stream overflow
    let mut expanded_stations = use_signal(HashSet::<String>::new);

    // Bumped by a pin, so the selected list re-reads without re-searching.
    let pin_gen = use_signal(|| 0u64);
    let selected_api = api.clone();
    let selected = use_resource(move || {
        let _ = pin_gen();
        let api = selected_api.clone();
        async move { api.radio_stations().await.unwrap_or_default() }
    });

    // The public directory: popular stations by default, a live search once
    // the debounced filter has text. The daemon queries it.
    let directory: Resource<Result<Vec<RadioStationInfo>, String>> = use_resource(move || {
        let query = filter();
        let api = api.clone();
        async move {
            api.search_radio(query.trim().to_string(), 60)
                .await
                .map_err(|error| error.to_string())
        }
    });

    let stations = selected.read().clone().unwrap_or_default();
    let mut pinned_ids = use_signal(HashSet::<String>::new);
    use_effect(move || {
        let next: HashSet<String> = selected
            .read()
            .clone()
            .unwrap_or_default()
            .iter()
            .filter(|station| station.pinned)
            .map(|station| station.id.clone())
            .collect();
        if *pinned_ids.peek() != next {
            pinned_ids.set(next);
        }
    });

    let query = filter.read().to_lowercase();
    let filtered: Vec<&RadioStationInfo> = stations
        .iter()
        .filter(|station| station.pinned && matches_query(station, &query))
        .collect();
    let has_custom = !filtered.is_empty();
    let searching = !query.is_empty();

    // Resource keeps its stale value while refetching,
    // track pending separately for the search spinner.
    let browser_loading = matches!(
        *directory.state().read(),
        UseResourceState::Pending | UseResourceState::Paused
    );
    let browser_state = directory.read();

    rsx! {
            div {
                class: if cfg!(target_os = "android") {
                    "px-4 pt-2 pb-6 w-full h-full overflow-y-auto"
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
                                class: "text-2xl font-semibold tracking-tight text-white",
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
                                h1 { class: "text-3xl font-semibold tracking-tight text-white",
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
                                class: "w-full bg-white/10 border border-white/10 rounded-full py-2.5 pl-12 pr-4 text-sm text-white focus:outline-none focus:border-white/25 transition-colors",
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
                        div { class: "grid grid-cols-1 lg:grid-cols-2 gap-3 mb-8",
                            for station in filtered.iter() {
                                div {
                                    key: "{station.id}",
                                    class: "group flex items-start gap-4 p-4 rounded-xl border transition-colors cursor-pointer hover:bg-white/10",
                                    style: "border-color: rgba(255,255,255,0.08);",
                                    onclick: {
                                        let station_id = station.id.clone();
                                        let stream_id = station.streams.first().map(|s| s.id.clone()).unwrap_or_default();
                                        move |_| {
                                            ctrl.play_radio(&station_id, &stream_id);
                                        }
                                    },

                                    div {
                                        class: "w-11 h-11 rounded-md flex items-center justify-center shrink-0",
                                        style: "background: rgba(255,255,255,0.05);",
                                        i {
                                            class: "{station.icon} text-lg",
                                            style: "color: var(--color-slate-300);",
                                        }
                                    }

                                    div { class: "flex-1 min-w-0",
                                        h2 {
                                            class: "text-base font-semibold text-white truncate",
                                            "{i18n::t(&station.name)}"
                                        }
                                        p {
                                            class: "text-xs mt-0.5 leading-relaxed line-clamp-2",
                                            style: "color: var(--color-slate-400);",
                                            "{i18n::t(&station.description)}"
                                        }

                                        if !station.tags.is_empty() {
                                            p {
                                                class: "text-xs mt-2 truncate",
                                                style: "color: var(--color-slate-500);",
                                                {station.tags.iter().take(4).cloned().collect::<Vec<_>>().join(" · ")}
                                            }
                                        }

                                        if station.streams.len() > 1 {
                                            div { class: "flex flex-wrap items-center gap-2 mt-2.5",
                                                for stream in &station.streams {
                                                    button {
                                                        class: "px-3 py-1 rounded-lg text-xs font-medium bg-white/10 hover:bg-white/20 transition-colors hover:text-white",
                                                        style: "color: var(--color-slate-300);",
                                                        onclick: {
                                                            let station_id = station.id.clone();
                                                            let stream_id = stream.id.clone();
                                                            move |evt: MouseEvent| {
                                                                evt.stop_propagation();
                                                                ctrl.play_radio(&station_id, &stream_id);
                                                            }
                                                        },
                                                        "{i18n::t(&stream.name)}"
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
                                            key: "{st.id}",
                                            class: "grid items-center px-4 py-2.5 rounded-lg mx-1 group cursor-pointer transition-colors hover:bg-white/[0.04]",
                                            style: "grid-template-columns: 48px 1fr 1.5fr 180px;",
                                            onclick: {
                                                let st = st.clone();
                                                move |_| {
                                                    ctrl.play_radio(&st.id, first_stream(&st));
                                                }
                                            },

                                            div { class: "flex items-center justify-center",
                                                if let Some(art) = station_art(st) {
                                                    img {
                                                        src: "{art}",
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
                                                    "{st.name}"
                                                }
                                            }

                                            div { class: "flex items-center justify-start text-left min-w-0 pr-4 gap-2",
                                                span {
                                                    class: "text-sm truncate",
                                                    style: "color: rgba(255,255,255,0.4);",
                                                    "{st.description}"
                                                }
                                                for tag in st.tags.iter().take(2) {
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
                                                    class: if pinned_ids.read().contains(&st.id) {
                                                        "inline-flex items-center justify-center w-8 h-8 rounded-full transition-all"
                                                    } else {
                                                        "inline-flex items-center justify-center w-8 h-8 rounded-full transition-all opacity-0 group-hover:opacity-100"
                                                    },
                                                    style: "background: rgba(255,255,255,0.06); color: rgba(255,255,255,0.5);",
                                                    onclick: {
                                                        let st = st.clone();
                                                        move |evt: MouseEvent| {
                                                            evt.stop_propagation();
                                                            toggle_pin(st.id.clone(), pinned_ids, pin_gen);
                                                        }
                                                    },
                                                    if pinned_ids.read().contains(&st.id) {
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
                                                        move |evt: MouseEvent| {
                                                            evt.stop_propagation();
                                                            ctrl.play_radio(&st.id, first_stream(&st));
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
                                div { class: "grid grid-cols-1 lg:grid-cols-2 gap-3",
                                    for st in results.iter() {
                                        div {
                                            key: "{st.id}",
                                            class: "group flex items-start gap-4 p-4 rounded-xl border transition-colors cursor-pointer hover:bg-white/10",
                                            style: "border-color: rgba(255,255,255,0.08);",
                                            onclick: {
                                                let st = st.clone();
                                                move |_| {
                                                    ctrl.play_radio(&st.id, first_stream(&st));
                                                }
                                            },

                                            if let Some(art) = station_art(st) {
                                                img {
                                                    src: "{art}",
                                                    class: "w-11 h-11 rounded-md object-cover shrink-0",
                                                    style: "background: rgba(255,255,255,0.05);",
                                                    decoding: "async", loading: "lazy",
                                                }
                                            } else {
                                                div {
                                                    class: "w-11 h-11 rounded-md flex items-center justify-center shrink-0",
                                                    style: "background: rgba(255,255,255,0.05);",
                                                    i {
                                                        class: "fa-solid fa-radio text-lg",
                                                        style: "color: var(--color-slate-300);",
                                                    }
                                                }
                                            }

                                            div { class: "flex-1 min-w-0",
                                                h2 {
                                                    class: "text-base font-semibold text-white truncate",
                                                    "{st.name}"
                                                }
                                                p {
                                                    class: "text-xs mt-0.5 leading-relaxed truncate",
                                                    style: "color: var(--color-slate-400);",
                                                    "{st.description}"
                                                }
                                                if !st.tags.is_empty() {
                                                    p {
                                                        class: "text-xs mt-2 truncate",
                                                        style: "color: var(--color-slate-500);",
                                                        {st.tags.iter().take(4).cloned().collect::<Vec<_>>().join(" · ")}
                                                    }
                                                }
                                            }

                                            button {
                                                class: "inline-flex items-center justify-center w-9 h-9 rounded-full transition-colors shrink-0 hover:bg-white/10 active:scale-95",
                                                style: "color: var(--color-slate-400);",
                                                onclick: {
                                                    let st = st.clone();
                                                    move |evt: MouseEvent| {
                                                        evt.stop_propagation();
                                                        toggle_pin(st.id.clone(), pinned_ids, pin_gen);
                                                    }
                                                },
                                                if pinned_ids.read().contains(&st.id) {
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
