//! Search for a source that declares search filters: a chip per filter, the
//! "all" view as a top result and a shelf per kind, one filter as a grid or a
//! list, and the source's completions under the box.
//!
//! The filters, their labels and which shelf leads to which filter all come
//! from the daemon, so nothing here knows which service answers.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use api::{
    CatalogItemKind, CatalogShelf, SearchFilter, SearchRequest, SearchSuggestion, ShelfLayout,
};
use dioxus::prelude::*;
use tracing::Instrument;

use super::discover::{ShelfRow, failure_text};
use super::shelves;

#[component]
pub fn CatalogSearch(
    search_query: Signal<String>,
    filters: Vec<SearchFilter>,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
) -> Element {
    let nav = use_context::<components::NavigationController>();
    let on_open_artist = EventHandler::new(move |artist: String| nav.open_artist(artist));
    let first = filters
        .first()
        .map(|filter| filter.id.clone())
        .unwrap_or_default();
    let mut filter = use_signal(|| first);
    let mut shelves = use_signal(Vec::<CatalogShelf>::new);
    let mut continuation = use_signal(|| None::<String>);
    let mut correction = use_signal(|| None::<String>);
    let mut loading = use_signal(|| false);
    let mut loading_more = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let mut fetch_gen = use_signal(|| 0u64);
    use_effect(move || {
        let query = search_query.read().trim().to_string();
        let picked = filter.read().clone();
        let my_gen = fetch_gen.with_mut(|generation| {
            *generation += 1;
            *generation
        });
        shelves.set(Vec::new());
        continuation.set(None);
        correction.set(None);
        error.set(None);
        if query.is_empty() {
            loading.set(false);
            return;
        }
        loading.set(true);
        let api = hooks::consume_api();
        let span = tracing::info_span!("query.catalog_search", filter = %picked);
        spawn(
            async move {
                let result = api.search(SearchRequest::filtered(query, picked)).await;
                if *fetch_gen.peek() != my_gen {
                    return;
                }
                match result {
                    Ok(results) => {
                        shelves.set(results.shelves);
                        continuation.set(results.continuation);
                        correction.set(results.correction);
                    }
                    Err(failure) => error.set(Some(failure_text(&failure))),
                }
                loading.set(false);
            }
            .instrument(span),
        );
    });

    let load_more = move || {
        let Some(token) = continuation.peek().clone() else {
            return;
        };
        if *loading_more.peek() {
            return;
        }
        loading_more.set(true);
        let my_gen = *fetch_gen.peek();
        let request = SearchRequest {
            query: search_query.peek().trim().to_string(),
            filter: Some(filter.peek().clone()),
            continuation: Some(token),
        };
        let api = hooks::consume_api();
        spawn(
            async move {
                let result = api.search(request).await;
                if *fetch_gen.peek() != my_gen {
                    return;
                }
                match result {
                    Ok(results) => {
                        // A continuation's one shelf continues the last one.
                        let mut current = shelves.write();
                        match (current.last_mut(), results.shelves.into_iter().next()) {
                            (Some(last), Some(more)) => last.items.extend(more.items),
                            (None, Some(more)) => current.push(more),
                            _ => {}
                        }
                        drop(current);
                        continuation.set(results.continuation);
                    }
                    Err(failure) => {
                        tracing::warn!(error = %failure, "search continuation failed");
                        continuation.set(None);
                    }
                }
                loading_more.set(false);
            }
            .instrument(tracing::info_span!("query.catalog_search_more")),
        );
    };

    use_effect(move || {
        let mut load_more = load_more;
        spawn(async move {
            let mut eval = document::eval(
                r#"
                const sentinel = document.getElementById('catalog-search-sentinel');
                if (sentinel) {
                    const obs = new IntersectionObserver((entries) => {
                        for (const e of entries) {
                            if (e.isIntersecting) {
                                dioxus.send('load-more');
                            }
                        }
                    }, { rootMargin: '600px' });
                    obs.observe(sentinel);
                }
                "#,
            );
            while let Ok(v) = eval.recv::<serde_json::Value>().await {
                if v.as_str() == Some("load-more") {
                    load_more();
                }
            }
        });
    });

    let all = filters.first().map(|filter| filter.id.clone());
    let filtered = all.as_deref() != Some(filter.read().as_str());
    let query_text = search_query.read().trim().to_string();
    let results = shelves.read().clone();
    let corrected_note = correction
        .read()
        .clone()
        .map(|corrected| i18n::t_with("search_showing_results_for", &[("query", corrected)]));

    rsx! {
        div { class: "p-8 max-w-[1600px] mx-auto",
            CatalogSearchBar {
                search_query,
                on_select_album,
                on_select_playlist,
                on_open_artist,
            }

            div { class: "flex flex-wrap gap-2 mb-8", role: "radiogroup", aria_label: i18n::t("search"),
                for option in filters.iter().cloned() {
                    {
                        let on = *filter.read() == option.id;
                        rsx! {
                            button {
                                key: "{option.id}",
                                role: "radio",
                                aria_checked: if on { "true" } else { "false" },
                                class: if on {
                                    "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors cursor-pointer"
                                } else {
                                    "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors cursor-pointer"
                                },
                                onclick: move |_| filter.set(option.id.clone()),
                                "{components::forms::text(&option.label)}"
                            }
                        }
                    }
                }
            }

            if let Some(note) = corrected_note {
                p { class: "text-sm text-white/50 mb-6", "{note}" }
            }

            if query_text.is_empty() {
                div { class: "py-24 flex flex-col items-center justify-center text-slate-600",
                    i { class: "fa-solid fa-magnifying-glass text-4xl mb-4" }
                    p { class: "text-lg", "{i18n::t(\"search_prompt\")}" }
                }
            } else if *loading.read() {
                div { class: "flex justify-center py-24",
                    i { class: "fa-solid fa-arrows-rotate fa-spin text-2xl text-white/60" }
                }
            } else if let Some(err) = error.read().clone() {
                div { class: "py-12 text-rose-400 text-sm", "{err}" }
            } else if results.iter().all(|shelf| shelf.items.is_empty()) {
                div { class: "py-24 flex flex-col items-center justify-center text-slate-600",
                    i { class: "fa-regular fa-folder-open text-4xl mb-4" }
                    p { class: "text-lg", "{i18n::t_with(\"discover_results_for\", &[(\"query\", query_text.clone())])}" }
                    p { class: "text-sm mt-1", "{i18n::t(\"discover_no_results_hint\")}" }
                }
            } else {
                for (idx, shelf) in results.into_iter().enumerate() {
                    {
                        let shelf = if filtered && shelf.layout == ShelfLayout::Carousel {
                            // One filter is one shelf, so it has the room to wrap.
                            CatalogShelf { layout: ShelfLayout::Grid, ..shelf }
                        } else {
                            shelf
                        };
                        let show_all = shelf.search_filter.clone().map(|target| {
                            EventHandler::new(move |_| filter.set(target.clone()))
                        });
                        rsx! {
                            ShelfRow {
                                key: "{idx}",
                                shelf,
                                scroll_id: format!("catalog-search-shelf-{idx}"),
                                on_select_album,
                                on_select_playlist,
                                on_open_artist,
                                show_all,
                            }
                        }
                    }
                }
            }

            div { id: "catalog-search-sentinel", class: "h-8" }
            if *loading_more.read() {
                div { class: "flex items-center justify-center gap-3 py-6 text-white/50 text-xs",
                    i { class: "fa-solid fa-arrows-rotate fa-spin" }
                    span { "{i18n::t(\"discover_more_loading\")}" }
                }
            }
        }
    }
}

/// The search box, with the source's completions under it while it has focus.
#[component]
fn CatalogSearchBar(
    search_query: Signal<String>,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
) -> Element {
    let debounce_gen = use_hook(|| Arc::new(AtomicU64::new(0))).clone();
    let mut text = use_signal(|| search_query.peek().clone());
    let mut suggestions = use_signal(Vec::<SearchSuggestion>::new);
    let mut open = use_signal(|| false);

    let mut suggest_gen = use_signal(|| 0u64);
    use_effect(move || {
        let typed = text.read().trim().to_string();
        let my_gen = suggest_gen.with_mut(|generation| {
            *generation += 1;
            *generation
        });
        if typed.is_empty() {
            suggestions.set(Vec::new());
            return;
        }
        let api = hooks::consume_api();
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if *suggest_gen.peek() != my_gen {
                return;
            }
            let answer = api.search_suggestions(typed).await.unwrap_or_default();
            if *suggest_gen.peek() == my_gen {
                suggestions.set(answer);
            }
        });
    });

    let mut run = move |query: String| {
        text.set(query.clone());
        search_query.set(query);
        open.set(false);
    };
    let shown = suggestions.read().clone();
    let expanded = open() && !shown.is_empty();

    rsx! {
        div { class: "relative max-w-2xl mb-6",
            i { class: "fa-solid fa-magnifying-glass absolute left-4 top-1/2 -translate-y-1/2 text-slate-500 pointer-events-none" }
            input {
                r#type: "text",
                placeholder: "{i18n::t(\"search_placeholder\")}",
                aria_label: i18n::t("search"),
                aria_expanded: if expanded { "true" } else { "false" },
                aria_controls: "catalog-search-suggestions",
                value: "{text}",
                class: "app-search-field w-full bg-white/10 border border-white/10 rounded-full py-3 pl-12 pr-4 text-white focus:outline-none focus:border-white/25 transition-colors",
                onfocus: move |_| open.set(true),
                // Late enough that a click on a suggestion lands first.
                onblur: move |_| {
                    spawn(async move {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        open.set(false);
                    });
                },
                oninput: move |evt| {
                    let value = evt.value();
                    text.set(value.clone());
                    open.set(true);
                    let tick = debounce_gen.fetch_add(1, Ordering::Relaxed) + 1;
                    let debounce_gen = debounce_gen.clone();
                    spawn(async move {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        if debounce_gen.load(Ordering::Relaxed) == tick {
                            search_query.set(value);
                        }
                    });
                },
                onkeydown: move |evt| {
                    evt.stop_propagation();
                    match evt.key() {
                        Key::Enter => {
                            // The peek guard has to drop before `run` writes `text`.
                            let query = text.peek().clone();
                            run(query);
                        }
                        Key::Escape => open.set(false),
                        _ => {}
                    }
                },
            }
            if expanded {
                div {
                    id: "catalog-search-suggestions",
                    role: "listbox",
                    class: "app-menu absolute left-0 right-0 top-full mt-2 z-40 flex flex-col bg-neutral-900 border border-white/10 rounded-lg py-1 shadow-xl",
                    for (idx, suggestion) in shown.into_iter().enumerate() {
                        {
                            match suggestion.item.clone() {
                                Some(item) => rsx! {
                                    div {
                                        key: "{idx}",
                                        role: "option",
                                        class: "px-2",
                                        onmousedown: move |evt| evt.prevent_default(),
                                        onclick: move |_| open.set(false),
                                        shelves::ItemRow {
                                            item,
                                            class: "w-full",
                                            on_select_album,
                                            on_select_playlist,
                                            on_open_artist,
                                        }
                                    }
                                },
                                None => {
                                    let query = suggestion.text.clone();
                                    rsx! {
                                        button {
                                            key: "{idx}",
                                            role: "option",
                                            class: "app-list-item px-4 py-2 text-sm text-white hover:bg-white/10 flex items-center gap-3 text-start transition-colors cursor-pointer",
                                            onmousedown: move |evt| evt.prevent_default(),
                                            onclick: move |_| run(query.clone()),
                                            i {
                                                class: if suggestion.from_history {
                                                    "fa-solid fa-clock-rotate-left text-xs text-white/40 w-4"
                                                } else {
                                                    "fa-solid fa-magnifying-glass text-xs text-white/40 w-4"
                                                },
                                                aria_hidden: "true",
                                            }
                                            span { class: "truncate", "{suggestion.text}" }
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
