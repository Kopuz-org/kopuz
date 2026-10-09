//! The shelf layouts beyond the carousel and the song list: grids, columns
//! of song rows, a top result, and rows for items that are not songs.
//!
//! Each one renders what a `CatalogShelf` declares, so a page lays out the
//! way its source describes it without this crate knowing which source that is.
use api::{CatalogItem, CatalogItemKind, CatalogShelf, TrackInfo};
use components::{CatalogPageRef, NavigationController};
use dioxus::prelude::*;

use super::discover::{DiscoverNowPlaying, DiscoverTile};

/// Where a tile leads. Songs play rather than open, so they have no target.
pub(crate) fn open_item(
    item: &CatalogItem,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
) {
    match item.kind {
        CatalogItemKind::Album => on_select_album.call(item.id.clone()),
        CatalogItemKind::Playlist => {
            on_select_playlist.call((item.kind, item.id.clone(), item.title.clone()))
        }
        CatalogItemKind::Artist => on_open_artist.call(item.id.clone()),
        CatalogItemKind::Mood | CatalogItemKind::Podcast | CatalogItemKind::Page => {
            consume_context::<NavigationController>().open_page(CatalogPageRef {
                kind: item.kind,
                id: item.id.clone(),
                title: item.title.clone(),
            })
        }
        CatalogItemKind::Track
        | CatalogItemKind::Video
        | CatalogItemKind::Episode
        | CatalogItemKind::Unknown => {}
    }
}

/// A shelf's "show all": its own page, opened as whatever kind the source says it is.
pub(crate) fn open_more(
    shelf: &CatalogShelf,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
) {
    let Some(id) = shelf.more_ref.clone() else {
        return;
    };
    match shelf.more_kind {
        CatalogItemKind::Album => on_select_album.call(id),
        CatalogItemKind::Artist => on_open_artist.call(id),
        CatalogItemKind::Mood | CatalogItemKind::Podcast | CatalogItemKind::Page => {
            consume_context::<NavigationController>().open_page(CatalogPageRef {
                kind: shelf.more_kind,
                id,
                title: shelf.title.clone(),
            })
        }
        // A song list's "show all" has always opened as a playlist of the same songs.
        _ => on_select_playlist.call((CatalogItemKind::Playlist, id, shelf.title.clone())),
    }
}

/// Play one song the way a song tile does: as the seed of a radio where the
/// source offers one, and on its own otherwise. An episode is never a seed.
pub(crate) fn play_song(item: &CatalogItem, track: &TrackInfo) {
    let mut ctrl = consume_context::<hooks::use_player_controller::PlayerController>();
    let mut now_playing = consume_context::<DiscoverNowPlaying>().0;
    let key = track.key.clone();
    if now_playing.peek().as_deref() == Some(key.as_str()) {
        ctrl.toggle();
        return;
    }
    now_playing.set(Some(key.clone()));
    let radio = match item.kind {
        CatalogItemKind::Episode => None,
        _ => components::track_row::radio_handler(key.clone()),
    };
    match radio {
        Some(radio) => radio.call(()),
        None => ctrl.set_queue_keys(vec![key], api::QueueMode::Replace, None),
    }
}

/// A shelf's title row, with its "show all" when it has a page of its own.
#[component]
pub(crate) fn ShelfHeader(
    shelf: CatalogShelf,
    on_more: Option<EventHandler<()>>,
    children: Element,
) -> Element {
    if shelf.title.is_empty() && shelf.strapline.is_none() {
        return rsx! {};
    }
    rsx! {
        div { class: "flex items-end justify-between mb-5 gap-4",
            div { class: "min-w-0",
                if let Some(strap) = shelf.strapline.clone() {
                    p { class: "text-[10px] font-bold mb-0.5 text-white/40", "{strap}" }
                }
                h2 { class: "text-2xl md:text-3xl font-bold text-white truncate", "{shelf.title}" }
            }
            div { class: "flex items-center gap-4 shrink-0",
                if let Some(more) = on_more {
                    button {
                        class: "app-button-text text-xs font-bold text-white/60 hover:text-white cursor-pointer transition-colors",
                        onclick: move |_| more.call(()),
                        "{i18n::t(\"discover_show_all\")}"
                    }
                }
                {children}
            }
        }
    }
}

/// Fetches more of one shelf in place, where the source pages it.
#[component]
pub(crate) fn ShowMore(loading: bool, on_more: EventHandler<()>) -> Element {
    rsx! {
        div { class: "flex justify-center pt-4",
            button {
                class: "app-button-tonal inline-flex items-center gap-2 text-xs px-3 py-1.5 rounded-lg bg-white/5 text-slate-400 hover:text-white hover:bg-white/10 transition-colors cursor-pointer disabled:cursor-default",
                disabled: loading,
                onclick: move |_| on_more.call(()),
                if loading {
                    i { class: "fa-solid fa-arrows-rotate fa-spin" }
                }
                "{i18n::t(\"discover_show_more\")}"
            }
        }
    }
}

/// A wrapping grid. Tiles without a picture (moods, genres) are buttons
/// tinted with the colour the source gives them; anything else is a card.
#[component]
pub(crate) fn GridShelf(
    shelf: CatalogShelf,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
    on_show_all: Option<EventHandler<()>>,
    on_more: Option<EventHandler<()>>,
    more_loading: bool,
) -> Element {
    let buttons = shelf.items.iter().all(|item| item.artwork.is_none());
    rsx! {
        section { class: "mb-12",
            ShelfHeader { shelf: shelf.clone(), on_more: on_show_all }
            if buttons {
                div { class: "grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 2xl:grid-cols-5 gap-3",
                    for (idx, item) in shelf.items.iter().enumerate() {
                        PageButton {
                            key: "{idx}",
                            item: item.clone(),
                            on_select_album,
                            on_select_playlist,
                            on_open_artist,
                        }
                    }
                }
            } else {
                div { class: "flex flex-wrap gap-x-5 gap-y-6",
                    for (idx, item) in shelf.items.iter().enumerate() {
                        DiscoverTile {
                            key: "{idx}",
                            item: item.clone(),
                            on_select_album,
                            on_select_playlist,
                            on_open_artist,
                        }
                    }
                }
            }
            if let Some(more) = on_more {
                ShowMore { loading: more_loading, on_more: more }
            }
        }
    }
}

/// A page link drawn as a button, as a mood or a genre is.
#[component]
pub(crate) fn PageButton(
    item: CatalogItem,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
) -> Element {
    let accent = item
        .accent
        .clone()
        .map(|colour| format!("border-inline-start-color: {colour};"))
        .unwrap_or_default();
    let target = item.clone();
    rsx! {
        button {
            class: "min-w-0 h-12 px-4 rounded-lg bg-white/5 hover:bg-white/10 border-s-4 border-white/10 text-start text-sm font-semibold text-white transition-colors cursor-pointer active:scale-[0.98]",
            style: "{accent}",
            onclick: move |_| open_item(&target, on_select_album, on_select_playlist, on_open_artist),
            span { class: "block truncate", "{item.title}" }
        }
    }
}

/// Song rows in columns of four that scroll sideways.
#[component]
pub(crate) fn TrackGridShelf(
    shelf: CatalogShelf,
    scroll_id: String,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
    on_show_all: Option<EventHandler<()>>,
) -> Element {
    rsx! {
        section { class: "mb-12",
            ShelfHeader { shelf: shelf.clone(), on_more: on_show_all,
                ScrollButtons { scroll_id: scroll_id.clone() }
            }
            div {
                id: "{scroll_id}",
                class: "grid grid-rows-4 grid-flow-col gap-x-6 gap-y-1 pb-3 scrollbar-hide scroll-smooth -mx-2 px-2",
                style: "overflow-x: auto; overflow-y: hidden;",
                for (idx, item) in shelf.items.iter().enumerate() {
                    ItemRow {
                        key: "{idx}",
                        item: item.clone(),
                        class: "w-80",
                        on_select_album,
                        on_select_playlist,
                        on_open_artist,
                    }
                }
            }
        }
    }
}

/// The carousel's previous and next buttons.
#[component]
pub(crate) fn ScrollButtons(scroll_id: String) -> Element {
    let scroll_left = scroll_id.clone();
    let scroll_right = scroll_id;
    rsx! {
        div { class: "flex gap-2 shrink-0",
            button {
                class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105 cursor-pointer",
                aria_label: i18n::t("discover_scroll_back"),
                onclick: move |_| {
                    let _ = document::eval(&format!(
                        "document.getElementById('{}').scrollBy({{ left: -800, behavior: 'smooth' }})",
                        scroll_left
                    ));
                },
                i { class: "fa-solid fa-chevron-left text-xs" }
            }
            button {
                class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105 cursor-pointer",
                aria_label: i18n::t("discover_scroll_forward"),
                onclick: move |_| {
                    let _ = document::eval(&format!(
                        "document.getElementById('{}').scrollBy({{ left: 800, behavior: 'smooth' }})",
                        scroll_right
                    ));
                },
                i { class: "fa-solid fa-chevron-right text-xs" }
            }
        }
    }
}

/// One large tile with the rest of the shelf as rows beside it.
#[component]
pub(crate) fn HeroShelf(
    shelf: CatalogShelf,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
    on_show_all: Option<EventHandler<()>>,
) -> Element {
    let Some(hero) = shelf.items.first().cloned() else {
        return rsx! {};
    };
    let rest: Vec<CatalogItem> = shelf.items.iter().skip(1).cloned().collect();
    let artwork = hooks::artwork::url(hero.artwork.as_ref(), hooks::artwork::Size::Thumb);
    let round = hero.kind == CatalogItemKind::Artist;
    let cover = if round { "rounded-full" } else { "rounded-lg" };
    let subtitle = hero.subtitle.clone().unwrap_or_default();
    let target = hero.clone();
    rsx! {
        section { class: "mb-12",
            ShelfHeader { shelf: shelf.clone(), on_more: on_show_all }
            div { class: "flex flex-col lg:flex-row gap-6",
                button {
                    class: "group shrink-0 w-full lg:w-80 p-5 rounded-xl bg-white/5 hover:bg-white/10 transition-colors text-start cursor-pointer flex flex-col gap-4",
                    onclick: move |_| match target.track.clone() {
                        Some(track) => play_song(&target, &track),
                        None => open_item(&target, on_select_album, on_select_playlist, on_open_artist),
                    },
                    div { class: "relative w-32 h-32 overflow-hidden {cover} bg-white/5",
                        if let Some(url) = artwork {
                            img {
                                src: "{url}",
                                class: "w-32 h-32 object-cover",
                                loading: "lazy",
                                decoding: "async",
                            }
                        }
                    }
                    div { class: "min-w-0",
                        p { class: "text-2xl font-bold text-white line-clamp-2 break-words", "{hero.title}" }
                        p { class: "text-sm text-white/50 truncate mt-1", "{subtitle}" }
                    }
                }
                if !rest.is_empty() {
                    div { class: "flex-1 min-w-0 flex flex-col gap-1",
                        for (idx, item) in rest.iter().enumerate() {
                            ItemRow {
                                key: "{idx}",
                                item: item.clone(),
                                class: "w-full",
                                on_select_album,
                                on_select_playlist,
                                on_open_artist,
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A compact row for any item: a song plays, anything else opens.
#[component]
pub(crate) fn ItemRow(
    item: CatalogItem,
    class: String,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(CatalogItemKind, String, String)>,
    on_open_artist: EventHandler<String>,
) -> Element {
    let ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let now_playing = use_context::<DiscoverNowPlaying>().0;
    let artwork = hooks::artwork::url(item.artwork.as_ref(), hooks::artwork::Size::Thumb);
    let cover = if item.kind == CatalogItemKind::Artist {
        "rounded-full"
    } else {
        "rounded-md"
    };
    let subtitle = item.subtitle.clone().unwrap_or_default();
    let track = item.track.clone();
    let is_this = track
        .as_ref()
        .is_some_and(|track| now_playing.read().as_deref() == Some(track.key.as_str()));
    let show_pause = is_this && *ctrl.is_playing.read() && !*ctrl.is_loading.read();
    let show_loading = is_this && *ctrl.is_loading.read();
    let mut menu_open = use_signal(|| false);
    let mut actions = use_signal(|| item.actions.clone());
    let target = item.clone();
    rsx! {
        div {
            class: "group flex items-center gap-3 p-2 rounded-lg hover:bg-white/5 transition-colors cursor-pointer min-w-0 {class}",
            oncontextmenu: move |evt| {
                if target.track.is_some() {
                    evt.prevent_default();
                    components::dots_menu::open_at_pointer(&evt);
                    menu_open.set(true);
                }
            },
            onclick: {
                let item = item.clone();
                move |_| match item.track.clone() {
                    Some(track) => play_song(&item, &track),
                    None => open_item(&item, on_select_album, on_select_playlist, on_open_artist),
                }
            },
            div { class: "relative w-12 h-12 shrink-0 overflow-hidden {cover} bg-white/5",
                if let Some(url) = artwork {
                    img {
                        src: "{url}",
                        class: "w-12 h-12 object-cover",
                        loading: "lazy",
                        decoding: "async",
                    }
                }
                if track.is_some() {
                    div {
                        class: if is_this {
                            "absolute inset-0 flex items-center justify-center bg-black/40"
                        } else {
                            "absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity duration-200"
                        },
                        i {
                            class: if show_loading {
                                "fa-solid fa-arrows-rotate fa-spin text-white text-sm"
                            } else if show_pause {
                                "fa-solid fa-pause text-white text-sm"
                            } else {
                                "fa-solid fa-play text-white text-sm"
                            }
                        }
                    }
                }
            }
            div { class: "min-w-0 flex-1",
                p { class: "text-sm font-semibold text-white truncate", "{item.title}" }
                p { class: "text-xs text-white/50 truncate h-4", "{subtitle}" }
            }
            if let Some(track) = track {
                div { onclick: move |evt| evt.stop_propagation(),
                    components::track_actions::TrackActionsMenu {
                        track,
                        catalog_actions: Some(actions()),
                        on_catalog_actions: move |next| actions.set(next),
                        is_open: Some(menu_open()),
                        on_open: Some(EventHandler::new(move |_| menu_open.set(true))),
                        on_close: Some(EventHandler::new(move |_| menu_open.set(false))),
                        button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                    }
                }
            } else {
                i { class: "fa-solid fa-chevron-right text-xs text-white/30 group-hover:text-white/60 transition-colors pe-2" }
            }
        }
    }
}
