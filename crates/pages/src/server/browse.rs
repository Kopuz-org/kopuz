//! The catalog pages a source declares: its home feed, Explore, charts, the
//! library tabs and whatever else it lists in `SourceCapabilities::pages`,
//! plus the pages its tiles open (a mood, a chart, a shelf's "show all").
//!
//! A page is drawn from what `catalog_detail` answers: its header kind, its
//! chips and its shelves, each in the layout the shelf declares. A source
//! that declares no pages keeps the plain Discover feed.
use api::{
    CatalogChip, CatalogDetail, CatalogDetailRequest, CatalogHeader, CatalogItem, CatalogItemKind,
    CatalogShelf, PageEntry, ShelfLayout,
};
use components::{CatalogPageRef, NavigationController};
use dioxus::prelude::*;
use tracing::Instrument;

use super::discover::{
    DiscoverNowPlaying, DiscoverPage, DiscoverPrefetchCache, ShelfRow, failure_text, play_catalog,
};

fn entry_page(entry: &PageEntry) -> CatalogPageRef {
    CatalogPageRef {
        kind: CatalogItemKind::Page,
        id: entry.id.clone(),
        title: components::forms::text(&entry.label),
    }
}

/// The Discover route: the first page the source declares, under the row of
/// all of them, or the plain feed for a source that declares none.
#[component]
pub fn Discover(
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
) -> Element {
    let caps = hooks::sources::use_capabilities();
    let first = caps().pages.first().map(entry_page);
    match first {
        None => rsx! {
            DiscoverPage { on_select_album, on_select_playlist, on_open_artist }
        },
        // A list of one, so that a different page is a fresh view rather than
        // the old one's state under a new title.
        Some(page) => rsx! {
            for page in std::iter::once(page) {
                CatalogPageView {
                    key: "{page.id}",
                    page,
                    on_select_album,
                    on_select_playlist,
                    on_open_artist,
                    on_back: None,
                }
            }
        },
    }
}

/// The Browse route: whichever page navigation last opened.
#[component]
pub fn Browse(
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
    on_back: EventHandler<()>,
) -> Element {
    let nav = use_context::<NavigationController>();
    let Some(page) = nav.catalog_page.read().clone() else {
        return rsx! {
            div { class: "flex items-center justify-center h-full text-white/60 p-12",
                p { "{i18n::t(\"catalog_page_empty\")}" }
            }
        };
    };
    rsx! {
        for page in std::iter::once(page) {
            CatalogPageView {
                key: "{page.kind:?}:{page.id}",
                page,
                on_select_album,
                on_select_playlist,
                on_open_artist,
                on_back: Some(on_back),
            }
        }
    }
}

/// A shelf continuation in flight: which shelf it continues.
#[derive(Clone, Copy, PartialEq)]
enum Loading {
    Page,
    Shelf(usize),
}

#[component]
fn CatalogPageView(
    page: CatalogPageRef,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
    /// Absent on a page the source lists, which sits under the page row instead.
    on_back: Option<EventHandler<()>>,
) -> Element {
    let api = hooks::use_api();
    let caps = hooks::sources::use_capabilities();
    let mut detail = use_signal(|| None::<CatalogDetail>);
    let mut error = use_signal(|| None::<String>);
    let mut loading_more = use_signal(|| None::<Loading>);
    // A chip filters the page in place; deselecting it goes back to `page`.
    let mut showing = use_signal(|| page.id.clone());
    // The label of the chip on once one has been picked here. The source can
    // keep marking a chip after it was cleared, so the pick is what the row shows.
    let mut picked = use_signal(|| None::<Option<String>>);
    let kind = page.kind;

    let mut fetch_gen = use_signal(|| 0u64);
    use_effect(move || {
        let id = showing.read().clone();
        let my_gen = fetch_gen.with_mut(|generation| {
            *generation += 1;
            *generation
        });
        detail.set(None);
        error.set(None);
        loading_more.set(None);
        let span = tracing::info_span!("catalog.page", id = %id);
        let api = api.clone();
        spawn(
            async move {
                let result = api
                    .catalog_detail(CatalogDetailRequest::new(kind, id))
                    .await;
                if *fetch_gen.peek() != my_gen {
                    return;
                }
                match result {
                    Ok(answer) => detail.set(Some(answer)),
                    Err(failure) => error.set(Some(failure_text(&failure))),
                }
            }
            .instrument(span),
        );
    });

    // The page's own continuation loads on scroll; failing that, the last
    // shelf's does, as a library tab is one long list.
    let scroll_target = move || -> Option<(Loading, String)> {
        let detail = detail.peek();
        let detail = detail.as_ref()?;
        if let Some(token) = detail.continuation.clone() {
            return Some((Loading::Page, token));
        }
        let last = detail.shelves.len().checked_sub(1)?;
        let token = detail.shelves[last].continuation.clone()?;
        Some((Loading::Shelf(last), token))
    };

    let load_more = move |target: Loading, token: String| {
        if loading_more.peek().is_some() {
            return;
        }
        loading_more.set(Some(target));
        let id = showing.peek().clone();
        let my_gen = *fetch_gen.peek();
        let api = hooks::consume_api();
        let span = tracing::info_span!("catalog.page_more", id = %id);
        spawn(
            async move {
                let request = CatalogDetailRequest {
                    kind,
                    id,
                    continuation: Some(token),
                };
                let result = api.catalog_detail(request).await;
                if *fetch_gen.peek() != my_gen {
                    return;
                }
                match result {
                    Ok(answer) => {
                        if let Some(current) = detail.write().as_mut() {
                            extend(current, target, answer);
                        }
                    }
                    Err(failure) => {
                        tracing::warn!(error = %failure, "catalog page continuation failed");
                        if let Some(current) = detail.write().as_mut() {
                            // Stop asking: a failed token fails again on every scroll.
                            match target {
                                Loading::Page => current.continuation = None,
                                Loading::Shelf(idx) => {
                                    if let Some(shelf) = current.shelves.get_mut(idx) {
                                        shelf.continuation = None;
                                    }
                                }
                            }
                        }
                    }
                }
                loading_more.set(None);
            }
            .instrument(span),
        );
    };

    use_effect(move || {
        let mut load_more = load_more;
        spawn(async move {
            let mut eval = document::eval(
                r#"
                const sentinel = document.getElementById('catalog-page-sentinel');
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
                if v.as_str() == Some("load-more")
                    && let Some((target, token)) = scroll_target()
                {
                    load_more(target, token);
                }
            }
        });
    });

    let pages = caps().pages.clone();
    let listed = pages.iter().any(|entry| entry.id == page.id);
    let loaded = detail.read().clone();
    let auto_shelf = match scroll_target() {
        Some((Loading::Shelf(idx), _)) => Some(idx),
        _ => None,
    };
    let in_flight = *loading_more.read();

    rsx! {
        div { class: "p-6 md:p-10 max-w-[1600px] mx-auto",
            if listed {
                h1 { class: "text-3xl md:text-4xl font-black text-white mb-4", "{i18n::t(\"discover\")}" }
                PageTabs { pages: pages.clone(), active: page.id.clone() }
                div { class: "h-px bg-white/10 mt-4 mb-8" }
            } else if let Some(back) = on_back {
                components::back_button::BackButton { on_click: move |_| back.call(()) }
            }

            if let Some(err) = error.read().clone() {
                div { class: "py-12 text-rose-400 text-sm", "{err}" }
            } else if let Some(loaded) = loaded {
                if !listed {
                    PageHeader { detail: loaded.clone(), fallback: page.title.clone() }
                }
                if !loaded.chips.is_empty() {
                    Chips {
                        chips: match picked.read().clone() {
                            Some(on) => loaded
                                .chips
                                .iter()
                                .cloned()
                                .map(|chip| CatalogChip {
                                    selected: on.as_deref() == Some(chip.label.as_str()),
                                    ..chip
                                })
                                .collect(),
                            None => loaded.chips.clone(),
                        },
                        on_pick: move |chip: CatalogChip| {
                            // Picking the chip that is on clears the filter.
                            if chip.selected {
                                picked.set(Some(None));
                                showing.set(page.id.clone());
                            } else {
                                picked.set(Some(Some(chip.label.clone())));
                                showing.set(chip.id);
                            }
                        },
                    }
                }
                if !loaded.tracks.is_empty() {
                    ShelfRow {
                        shelf: tracks_shelf(&loaded),
                        scroll_id: "catalog-page-tracks".to_string(),
                        on_select_album,
                        on_select_playlist,
                        on_open_artist,
                    }
                }
                for (idx, shelf) in loaded.shelves.iter().enumerate() {
                    ShelfRow {
                        key: "{idx}",
                        shelf: shelf.clone(),
                        scroll_id: format!("catalog-page-shelf-{idx}"),
                        on_select_album,
                        on_select_playlist,
                        on_open_artist,
                        on_more: match (&shelf.continuation, auto_shelf) {
                            (Some(token), auto) if auto != Some(idx) => {
                                let token = token.clone();
                                {
                                    let mut load_more = load_more;
                                    Some(EventHandler::new(move |_| load_more(Loading::Shelf(idx), token.clone())))
                                }
                            }
                            _ => None,
                        },
                        more_loading: in_flight == Some(Loading::Shelf(idx)),
                    }
                }
                if loaded.tracks.is_empty() && loaded.shelves.is_empty() {
                    div { class: "py-24 flex flex-col items-center justify-center text-slate-600",
                        i { class: "fa-regular fa-folder-open text-4xl mb-4" }
                        p { class: "text-lg", "{i18n::t(\"catalog_page_empty\")}" }
                    }
                }
            } else {
                div { class: "flex justify-center py-24",
                    i { class: "fa-solid fa-arrows-rotate fa-spin text-2xl text-white/60" }
                }
            }

            div { id: "catalog-page-sentinel", class: "h-8" }

            if in_flight.is_some() && in_flight == scroll_target().map(|(target, _)| target) {
                div { class: "flex items-center justify-center gap-3 py-6 text-white/50 text-xs",
                    i { class: "fa-solid fa-arrows-rotate fa-spin" }
                    span { "{i18n::t(\"discover_more_loading\")}" }
                }
            }
        }
    }
}

/// Fold a continuation's answer into the page: more shelves for the page, or
/// more items for the one shelf it continues.
fn extend(current: &mut CatalogDetail, target: Loading, answer: CatalogDetail) {
    match target {
        Loading::Page => {
            current.tracks.extend(answer.tracks);
            current.shelves.extend(answer.shelves);
            current.continuation = answer.continuation;
        }
        Loading::Shelf(idx) => {
            let Some(shelf) = current.shelves.get_mut(idx) else {
                return;
            };
            match answer.shelves.into_iter().next() {
                Some(more) => {
                    shelf.items.extend(more.items);
                    shelf.continuation = more.continuation;
                }
                None => shelf.continuation = None,
            }
        }
    }
}

/// A page's own songs, drawn as a song list like any other.
fn tracks_shelf(detail: &CatalogDetail) -> CatalogShelf {
    CatalogShelf {
        items: detail
            .tracks
            .iter()
            .map(|track| CatalogItem {
                kind: CatalogItemKind::Track,
                id: track.key.clone(),
                title: track.title.clone(),
                track: Some(track.clone()),
                ..CatalogItem::default()
            })
            .collect(),
        list: true,
        layout: ShelfLayout::List,
        ..CatalogShelf::default()
    }
}

/// The source's pages as a row of links, the one on screen lit.
#[component]
fn PageTabs(pages: Vec<PageEntry>, active: String) -> Element {
    let nav = use_context::<NavigationController>();
    let first = pages.first().map(|entry| entry.id.clone());
    rsx! {
        nav {
            class: "flex flex-wrap gap-1",
            aria_label: i18n::t("discover"),
            for entry in pages.iter() {
                {
                    let is_active = entry.id == active;
                    let target = entry_page(entry);
                    let is_first = first.as_deref() == Some(entry.id.as_str());
                    rsx! {
                        button {
                            key: "{entry.id}",
                            class: if is_active {
                                "app-chip inline-flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium bg-white/10 text-white transition-colors cursor-pointer"
                            } else {
                                "app-chip inline-flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium text-slate-400 hover:text-white/90 hover:bg-white/5 transition-colors cursor-pointer active:scale-[0.98]"
                            },
                            aria_current: if is_active { "page" } else { "false" },
                            onclick: move |_| {
                                if is_active {
                                    return;
                                }
                                if is_first {
                                    let mut route = nav.current_route;
                                    route.set(kopuz_route::Route::Discover);
                                } else {
                                    nav.open_page(target.clone());
                                }
                            },
                            components::forms::Glyph { icon: entry.icon.clone(), class: "text-xs opacity-80" }
                            span { "{components::forms::text(&entry.label)}" }
                        }
                    }
                }
            }
        }
    }
}

/// The filters over a page, as a row of chips.
#[component]
fn Chips(chips: Vec<CatalogChip>, on_pick: EventHandler<CatalogChip>) -> Element {
    rsx! {
        div { class: "flex flex-wrap gap-2 mb-8",
            for chip in chips.iter().cloned() {
                button {
                    key: "{chip.id}",
                    class: if chip.selected {
                        "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/20 text-white font-medium transition-colors cursor-pointer"
                    } else {
                        "app-chip text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors cursor-pointer"
                    },
                    aria_pressed: if chip.selected { "true" } else { "false" },
                    onclick: {
                        let chip = chip.clone();
                        move |_| on_pick.call(chip.clone())
                    },
                    "{chip.label}"
                }
            }
        }
    }
}

/// The top of a page a tile opened, drawn the way its header kind says.
#[component]
fn PageHeader(detail: CatalogDetail, fallback: String) -> Element {
    let mut actions = use_signal(|| detail.actions.clone());
    let ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let now_playing = use_context::<DiscoverNowPlaying>().0;
    let cache = use_context::<DiscoverPrefetchCache>().0;
    let title = if detail.title.is_empty() {
        fallback
    } else {
        detail.title.clone()
    };
    let artwork = hooks::artwork::url(detail.artwork.as_ref(), hooks::artwork::Size::Thumb);
    let play = detail.playback_id.clone().map(|id| {
        EventHandler::new(move |_: ()| {
            play_catalog(
                CatalogItemKind::Playlist,
                id.clone(),
                ctrl,
                now_playing,
                cache,
            )
        })
    });
    match detail.header {
        CatalogHeader::Detail => rsx! {
            div { class: "showcase-hero flex flex-col md:flex-row md:flex-wrap items-end gap-8 mb-12",
                div { class: "showcase-cover w-64 h-64 rounded-xl bg-stone-800 overflow-hidden relative flex-shrink-0",
                    if let Some(url) = artwork {
                        img { src: "{url}", class: "w-full h-full object-cover" }
                    } else {
                        div { class: "w-full h-full flex flex-col items-center justify-center text-white/20",
                            i { class: "fa-solid fa-music text-6xl mb-4" }
                        }
                    }
                }
                div { class: "flex-1 min-w-0 md:min-w-64",
                    if let Some(subtitle) = detail.subtitle.clone() {
                        h5 { class: "text-sm font-bold text-white/60 mb-2", "{subtitle}" }
                    }
                    h1 { class: "showcase-title text-5xl md:text-7xl font-semibold tracking-tight text-white mb-6 break-words line-clamp-3 text-balance", "{title}" }
                    if let Some(description) = detail.description.clone() {
                        p { class: "text-sm text-white/60 max-w-3xl line-clamp-3 text-pretty mb-6", "{description}" }
                    }
                    div { class: "flex items-center gap-4",
                        if let Some(play) = play {
                            PlayButton { on_play: play }
                        }
                        components::catalog_actions::CatalogActionButtons {
                            actions: actions(),
                            on_change: move |next| actions.set(next),
                        }
                    }
                }
            }
        },
        CatalogHeader::Artist => {
            let banner_style = artwork
                .map(|url| format!("background-image: linear-gradient(to bottom, rgba(0,0,0,0.2) 0%, rgba(0,0,0,0.95) 100%), url('{url}'); background-size: cover; background-position: center; min-height: 320px;"))
                .unwrap_or_else(|| "min-height: 240px;".to_string());
            rsx! {
                div {
                    class: "relative overflow-hidden rounded-xl flex flex-col justify-end mb-12",
                    style: "{banner_style}",
                    div { class: "px-6 md:px-10 pt-16 pb-10 flex flex-col gap-4",
                        h1 { class: "text-4xl md:text-6xl font-black text-white break-words drop-shadow-lg text-balance", "{title}" }
                        if let Some(subtitle) = detail.subtitle.clone() {
                            p { class: "text-sm text-white/70", "{subtitle}" }
                        }
                        if let Some(description) = detail.description.clone() {
                            p { class: "text-sm text-white/60 max-w-3xl line-clamp-3 text-pretty", "{description}" }
                        }
                        div { class: "flex items-center gap-3 mt-2",
                            if let Some(play) = play {
                                PlayButton { on_play: play }
                            }
                            components::catalog_actions::CatalogActionButtons {
                                actions: actions(),
                                on_change: move |next| actions.set(next),
                            }
                        }
                    }
                }
            }
        }
        CatalogHeader::Title | CatalogHeader::None => rsx! {
            div { class: "mb-8",
                h1 { class: "text-3xl md:text-4xl font-black text-white mb-2 text-balance", "{title}" }
                if let Some(subtitle) = detail.subtitle.clone() {
                    p { class: "text-sm text-white/50", "{subtitle}" }
                }
            }
            div { class: "h-px bg-white/10 mb-8" }
        },
    }
}

#[component]
fn PlayButton(on_play: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: "playback-play w-14 h-14 rounded-full bg-indigo-500 hover:bg-indigo-400 text-black flex items-center justify-center transition-transform hover:scale-105 active:scale-95 cursor-pointer",
            aria_label: i18n::t("play"),
            onclick: move |_| on_play.call(()),
            i { class: "fa-solid fa-play text-xl ml-1" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shelf(title: &str, items: usize, continuation: Option<&str>) -> CatalogShelf {
        CatalogShelf {
            title: title.to_string(),
            items: vec![CatalogItem::default(); items],
            continuation: continuation.map(str::to_string),
            ..CatalogShelf::default()
        }
    }

    #[test]
    fn a_page_continuation_adds_its_shelves() {
        let mut page = CatalogDetail {
            shelves: vec![shelf("a", 2, None)],
            continuation: Some("next".into()),
            ..CatalogDetail::default()
        };
        let answer = CatalogDetail {
            shelves: vec![shelf("b", 3, None)],
            ..CatalogDetail::default()
        };
        extend(&mut page, Loading::Page, answer);
        assert_eq!(page.shelves.len(), 2);
        assert_eq!(page.continuation, None);
    }

    #[test]
    fn a_shelf_continuation_grows_that_shelf() {
        let mut page = CatalogDetail {
            shelves: vec![shelf("a", 2, Some("more")), shelf("b", 1, None)],
            ..CatalogDetail::default()
        };
        let answer = CatalogDetail {
            shelves: vec![shelf("", 4, Some("again"))],
            ..CatalogDetail::default()
        };
        extend(&mut page, Loading::Shelf(0), answer);
        assert_eq!(page.shelves.len(), 2);
        assert_eq!(page.shelves[0].items.len(), 6);
        assert_eq!(page.shelves[0].continuation.as_deref(), Some("again"));
    }

    #[test]
    fn an_empty_shelf_answer_stops_asking() {
        let mut page = CatalogDetail {
            shelves: vec![shelf("a", 2, Some("more"))],
            ..CatalogDetail::default()
        };
        extend(&mut page, Loading::Shelf(0), CatalogDetail::default());
        assert_eq!(page.shelves[0].continuation, None);
    }
}
