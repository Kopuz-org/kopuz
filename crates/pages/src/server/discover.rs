//! The source's browse catalog: shelves of albums, playlists, artists and
//! songs that the library does not hold.
//!
//! Every row here comes from `LibraryApi::catalog`, so this page knows nothing
//! about which service produced it, holds no credentials, and plays a tile by
//! naming keys the daemon already registered. Images are artwork refs like
//! every other picture in the app.
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use api::{CatalogDetailRequest, CatalogItem, CatalogItemKind, CatalogShelf, TrackInfo};
use components::track_row::TrackRow;
use dioxus::prelude::*;
use tracing::Instrument;

/// The id of the tile that last started playback -- a catalog id for an album
/// or playlist, a track key for a song. Tiles read it to decide whether their
/// overlay shows play or pause, and whether a click should fetch or toggle.
#[derive(Clone, Copy)]
pub struct DiscoverNowPlaying(pub Signal<Option<String>>);

/// Hover-prefetched track lists keyed by the tile's catalog id, so a click
/// after a hover starts playing without a round trip.
#[derive(Clone, Copy)]
pub struct DiscoverPrefetchCache(pub Signal<HashMap<String, Vec<TrackInfo>>>);

/// What a failure says. A source that has not been signed into is the one
/// case worth wording ourselves; everything else is the daemon's message.
fn failure_text(error: &api::ApiError) -> String {
    if error.code == api::ErrorCode::SourceAuthExpired {
        return i18n::t("yt_anon_discover");
    }
    i18n::t_with("discover_failed", &[("error", error.to_string())])
}

fn keys_of(tracks: &[TrackInfo]) -> Vec<String> {
    tracks.iter().map(|track| track.key.clone()).collect()
}

#[component]
#[tracing::instrument(name = "render.discover_home", skip_all)]
pub fn DiscoverPage(
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(String, String)>,
    on_open_artist: EventHandler<(String, String)>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let api = hooks::use_api();
    let caps = hooks::sources::use_capabilities();
    let mut shelves = use_signal(Vec::<CatalogShelf>::new);
    let mut continuation = use_signal(|| None::<String>);
    let mut loading_more = use_signal(|| false);
    let mut initial_loading = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);

    use_effect(move || {
        // Read inside the effect: capabilities arrive with the source list, so
        // the first render can say "no" and the load has to follow the answer.
        if !caps().discover {
            initial_loading.set(false);
            return;
        }
        if !shelves.peek().is_empty() {
            return;
        }
        let home_span = tracing::info_span!("discover.load_home");
        let api = api.clone();
        spawn(
            async move {
                match api.catalog(None).await {
                    Ok(page) => {
                        shelves.write().extend(page.shelves);
                        continuation.set(page.continuation);
                        error.set(None);
                    }
                    Err(failure) => error.set(Some(failure_text(&failure))),
                }
                initial_loading.set(false);
            }
            .instrument(home_span),
        );
    });

    if !caps().discover {
        return rsx! {
            div { class: "flex items-center justify-center h-full text-white/60 p-12 text-center",
                p { "{i18n::t(\"discover_requires_ytmusic\")}" }
            }
        };
    }

    let load_more = move || {
        let Some(token) = continuation.peek().clone() else {
            return;
        };
        if *loading_more.peek() {
            return;
        }
        loading_more.set(true);
        let more_span = tracing::info_span!("discover.load_more");
        let api = hooks::consume_api();
        spawn(
            async move {
                match api.catalog(Some(token)).await {
                    Ok(page) => {
                        shelves.write().extend(page.shelves);
                        continuation.set(page.continuation);
                    }
                    Err(failure) => error.set(Some(failure_text(&failure))),
                }
                loading_more.set(false);
            }
            .instrument(more_span),
        );
    };

    use_effect(move || {
        let mut load_more = load_more;
        spawn(async move {
            let mut eval = document::eval(
                r#"
                const sentinel = document.getElementById('discover-sentinel');
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

    rsx! {
        div { class: "p-6 md:p-10 max-w-[1600px] mx-auto",
            h1 { class: "text-3xl md:text-4xl font-black text-white mb-2", "{i18n::t(\"discover\")}" }
            div { class: "h-px bg-white/10 mb-8" }

            if *initial_loading.read() {
                div { class: "flex justify-center py-24",
                    i { class: "fa-solid fa-arrows-rotate fa-spin text-2xl text-white/60" }
                }
            } else if let Some(err) = error.read().clone() {
                div { class: "py-12 text-rose-400 text-sm", "{err}" }
            }

            for (idx, shelf) in shelves.read().iter().enumerate() {
                ShelfRow {
                    key: "{idx}",
                    shelf: shelf.clone(),
                    scroll_id: format!("discover-shelf-{idx}"),
                    on_select_album: on_select_album,
                    on_select_playlist: on_select_playlist,
                    on_open_artist: on_open_artist,
                    on_search_artist: on_search_artist,
                }
            }

            div { id: "discover-sentinel", class: "h-8" }

            if *loading_more.read() {
                div { class: "flex items-center justify-center gap-3 py-6 text-white/50 text-xs",
                    i { class: "fa-solid fa-arrows-rotate fa-spin" }
                    span { "{i18n::t(\"discover_more_loading\")}" }
                }
            }
        }
    }
}

#[component]
fn ShelfRow(
    shelf: CatalogShelf,
    scroll_id: String,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(String, String)>,
    on_open_artist: EventHandler<(String, String)>,
    on_search_artist: EventHandler<String>,
) -> Element {
    if shelf.list {
        return rsx! { SongListShelf {
            shelf: shelf.clone(),
            on_select_playlist: on_select_playlist,
        } };
    }
    let scroll_left = scroll_id.clone();
    let scroll_right = scroll_id.clone();
    rsx! {
        section { class: "mb-12",
            div { class: "flex items-end justify-between mb-5 gap-4",
                div { class: "min-w-0",
                    if let Some(strap) = shelf.strapline.clone() {
                        p { class: "text-[10px] font-bold mb-0.5 text-white/40", "{strap}" }
                    }
                    h2 { class: "text-2xl md:text-3xl font-bold text-white truncate", "{shelf.title}" }
                }
                div { class: "flex gap-2 shrink-0",
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105 cursor-pointer",
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
            div {
                id: "{scroll_id}",
                class: "flex items-start gap-5 pb-3 pt-1 scrollbar-hide scroll-smooth -mx-2 px-2",
                style: "overflow-x: auto; overflow-y: hidden;",
                for (idx, item) in shelf.items.iter().enumerate() {
                    DiscoverTile {
                        key: "{idx}",
                        item: item.clone(),
                        on_select_album: on_select_album,
                        on_select_playlist: on_select_playlist,
                        on_open_artist: on_open_artist,
                        on_search_artist: on_search_artist,
                    }
                }
            }
        }
    }
}

/// A shelf a source renders as a track list rather than a carousel, which is
/// the artist page's "top songs". Only the first few rows come inline; the
/// shelf's `more_ref` opens the full list in the playlist viewer.
#[component]
fn SongListShelf(
    shelf: CatalogShelf,
    on_select_playlist: EventHandler<(String, String)>,
) -> Element {
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let mut now_playing = use_context::<DiscoverNowPlaying>().0;
    let songs: Vec<TrackInfo> = shelf
        .items
        .iter()
        .filter_map(|item| item.track.clone())
        .collect();
    let title_for_more = shelf.title.clone();
    let more = shelf.more_ref.clone();
    rsx! {
        section { class: "mb-12",
            div { class: "flex items-end justify-between mb-5 gap-4",
                h2 { class: "text-2xl md:text-3xl font-bold text-white truncate", "{shelf.title}" }
                if let Some(more) = more {
                    button {
                        class: "text-xs font-bold text-white/60 hover:text-white cursor-pointer transition-colors",
                        onclick: move |_| {
                            on_select_playlist.call((more.clone(), title_for_more.clone()))
                        },
                        "{i18n::t(\"discover_show_all\")}"
                    }
                }
            }
            div { class: "flex flex-col",
                {
                    // Shared menu / playing state across the rows.
                    let mut active_menu_key = use_signal(|| None::<String>);
                    let mut current_playing_key = use_signal(|| None::<String>);
                    let keys = keys_of(&songs);
                    rsx! {
                        for (idx, info) in songs.iter().enumerate() {
                            {
                                let key = info.key.clone();
                                let key_for_play = key.clone();
                                let key_for_menu = key.clone();
                                let keys = keys.clone();
                                let cover_url = hooks::artwork::url(info.artwork.as_ref(), hooks::artwork::Size::Thumb);
                                let is_current = current_playing_key.read().as_deref() == Some(key.as_str());
                                let is_menu_open = active_menu_key.read().as_deref() == Some(key.as_str());
                                rsx! {
                                    TrackRow {
                                        key: "{idx}",
                                        track: info.clone(),
                                        cover_url,
                                        on_start_radio: components::track_row::radio_handler(key.clone()),
                                        row_num: Some(idx + 1),
                                        is_menu_open,
                                        is_currently_playing: is_current,
                                        hide_delete: true,
                                        on_play: move |_| {
                                            current_playing_key.set(Some(key_for_play.clone()));
                                            // Top songs is a preview: clear the tile
                                            // tag so no album or playlist card claims
                                            // the pause overlay while one of these plays.
                                            now_playing.set(None);
                                            ctrl.set_queue_keys(
                                                keys.clone(),
                                                api::QueueMode::Replace,
                                                Some(idx as u32),
                                            );
                                        },
                                        on_click_menu: move |_| {
                                            if active_menu_key.read().as_deref() == Some(key_for_menu.as_str()) {
                                                active_menu_key.set(None);
                                            } else {
                                                active_menu_key.set(Some(key_for_menu.clone()));
                                            }
                                        },
                                        on_close_menu: move |_| active_menu_key.set(None),
                                        on_add_to_playlist: move |_| {
                                            active_menu_key.set(None);
                                        },
                                        on_delete: move |_| active_menu_key.set(None),
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

#[component]
fn DiscoverTile(
    item: CatalogItem,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(String, String)>,
    on_open_artist: EventHandler<(String, String)>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let now_playing = use_context::<DiscoverNowPlaying>().0;
    let cache = use_context::<DiscoverPrefetchCache>().0;
    let thumbnail = hooks::artwork::url(item.artwork.as_ref(), hooks::artwork::Size::Thumb);
    let subtitle = item.subtitle.clone().unwrap_or_default();
    match item.kind {
        CatalogItemKind::Track => match item.track.clone() {
            Some(track) => rsx! { SongCard { item: item.clone(), track } },
            None => rsx! {},
        },
        CatalogItemKind::Playlist | CatalogItemKind::Album => {
            let kind = item.kind;
            let id = item.id.clone();
            let id_for_click = id.clone();
            let id_for_play = id.clone();
            let title_for_click = item.title.clone();
            rsx! {
                Card {
                    title: item.title.clone(),
                    subtitle,
                    thumbnail,
                    rounded_full: false,
                    onclick: move |_| {
                        if kind == CatalogItemKind::Album {
                            on_select_album.call(id_for_click.clone());
                        } else {
                            on_select_playlist.call((id_for_click.clone(), title_for_click.clone()));
                        }
                    },
                    on_play: EventHandler::new(move |_| {
                        play_catalog(kind, id_for_play.clone(), ctrl, now_playing, cache);
                    }),
                    kind,
                    source_id: Some(id),
                }
            }
        }
        CatalogItemKind::Artist => {
            let id = item.id.clone();
            let name = item.title.clone();
            rsx! {
                Card {
                    title: item.title.clone(),
                    subtitle: String::new(),
                    thumbnail,
                    rounded_full: true,
                    onclick: move |_| on_open_artist.call((id.clone(), name.clone())),
                    on_play: None,
                    kind: CatalogItemKind::Artist,
                    source_id: None,
                }
            }
        }
        CatalogItemKind::Mood | CatalogItemKind::Unknown => rsx! {
            Card {
                title: item.title.clone(),
                subtitle: String::new(),
                thumbnail,
                rounded_full: false,
                onclick: move |_| {},
                on_play: None,
                kind: CatalogItemKind::Mood,
                source_id: None,
            }
        },
    }
}

/// Play everything behind a catalog id. The first page starts the queue and
/// the rest append while it plays, so a long playlist does not hold up the
/// first song; the whole list is cached only when it paged in cleanly.
fn play_catalog(
    kind: CatalogItemKind,
    id: String,
    mut ctrl: hooks::use_player_controller::PlayerController,
    mut now_playing: Signal<Option<String>>,
    mut cache: Signal<HashMap<String, Vec<TrackInfo>>>,
) {
    ctrl.browse_loading.set(true);
    now_playing.set(Some(id.clone()));
    if let Some(tracks) = cache.peek().get(&id).cloned()
        && !tracks.is_empty()
    {
        ctrl.set_queue_keys(keys_of(&tracks), api::QueueMode::Replace, None);
        ctrl.browse_loading.set(false);
        return;
    }
    let play_span = tracing::info_span!("discover.play_catalog", id = %id);
    let api = hooks::consume_api();
    spawn(
        async move {
            let mut started = false;
            let mut collected = Vec::<TrackInfo>::new();
            let mut seen = HashSet::<String>::new();
            let mut cursor = None::<String>;
            let mut complete = false;
            loop {
                let request = CatalogDetailRequest {
                    kind,
                    id: id.clone(),
                    continuation: cursor.clone(),
                };
                let detail = match api.catalog_detail(request).await {
                    Ok(detail) => detail,
                    Err(error) => {
                        tracing::warn!(%error, "catalog play failed");
                        if started {
                            ctrl.playback_error.set(Some(failure_text(&error)));
                        }
                        break;
                    }
                };
                let fresh: Vec<TrackInfo> = detail
                    .tracks
                    .into_iter()
                    .filter(|track| seen.insert(track.key.clone()))
                    .collect();
                if !fresh.is_empty() {
                    let keys = keys_of(&fresh);
                    if started {
                        ctrl.set_queue_keys(keys, api::QueueMode::Append, None);
                    } else {
                        ctrl.set_queue_keys(keys, api::QueueMode::Replace, None);
                        ctrl.browse_loading.set(false);
                        started = true;
                    }
                    collected.extend(fresh);
                }
                match detail.continuation {
                    Some(next) => cursor = Some(next),
                    None => {
                        complete = true;
                        break;
                    }
                }
            }
            // A run that broke mid-way leaves a truncated list; caching it
            // would poison every later click on the same tile.
            if complete && started {
                cache.write().insert(id, collected);
            }
            if !started {
                ctrl.browse_loading.set(false);
                now_playing.set(None);
            }
        }
        .instrument(play_span),
    );
}

#[component]
fn Card(
    title: String,
    subtitle: String,
    thumbnail: Option<utils::CoverUrl>,
    rounded_full: bool,
    onclick: EventHandler<MouseEvent>,
    on_play: Option<EventHandler<()>>,
    kind: CatalogItemKind,
    /// The catalog id this card represents. When it equals
    /// [`DiscoverNowPlaying`] the overlay shows pause and a click toggles the
    /// player instead of fetching again.
    source_id: Option<String>,
) -> Element {
    let img_class = if rounded_full {
        "w-44 h-44 object-cover rounded-full bg-white/5"
    } else {
        "w-44 h-44 object-cover rounded-lg bg-white/5"
    };
    let placeholder_class = if rounded_full {
        "w-44 h-44 rounded-full bg-white/5"
    } else {
        "w-44 h-44 rounded-lg bg-white/5"
    };
    let cover_radius = if rounded_full {
        "rounded-full"
    } else {
        "rounded-lg"
    };
    let now_playing = use_context::<DiscoverNowPlaying>().0;
    let mut cache = use_context::<DiscoverPrefetchCache>().0;
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    // Per-tile hover gate that survives across renders, so the prefetch task
    // can tell whether the cursor is still here after the debounce.
    let mut hover_armed = use_signal(|| false);
    let is_this_source = match (&source_id, now_playing.read().as_ref()) {
        (Some(sid), Some(active)) => sid == active,
        _ => false,
    };
    // Three icon states: play, spinner while this tile is fetching, pause once
    // audio is actually running.
    let is_playing = *ctrl.is_playing.read();
    let is_loading = *ctrl.is_loading.read();
    let show_loading = is_this_source && is_loading;
    let show_pause = is_this_source && is_playing && !is_loading;
    let prefetch_id = source_id.clone();
    rsx! {
        div {
            class: "shrink-0 w-44 text-left cursor-pointer transition-transform duration-200 ease-out hover:scale-[1.03] hover:-translate-y-0.5 group",
            onclick: move |e| onclick.call(e),
            onmouseenter: move |_| {
                let Some(id) = prefetch_id.clone() else { return; };
                if on_play.is_none() {
                    return;
                }
                hover_armed.set(true);
                let prefetch_span = tracing::info_span!("discover.prefetch", id = %id);
                let api = hooks::consume_api();
                spawn(async move {
                    // Short delay so a cursor crossing a shelf does not fire a
                    // dozen requests; leaving the tile disarms it.
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    if !*hover_armed.peek() {
                        return;
                    }
                    if cache.peek().contains_key(&id) {
                        return;
                    }
                    let mut tracks = Vec::<TrackInfo>::new();
                    let mut cursor = None::<String>;
                    loop {
                        let request = CatalogDetailRequest {
                            kind,
                            id: id.clone(),
                            continuation: cursor.clone(),
                        };
                        let Ok(detail) = api.catalog_detail(request).await else {
                            return;
                        };
                        tracks.extend(detail.tracks);
                        match detail.continuation {
                            Some(next) => cursor = Some(next),
                            None => break,
                        }
                    }
                    if !tracks.is_empty() {
                        cache.write().insert(id, tracks);
                    }
                }.instrument(prefetch_span));
            },
            onmouseleave: move |_| {
                hover_armed.set(false);
            },
            div { class: "relative w-44 h-44 mb-3 overflow-hidden {cover_radius}",
                if let Some(url) = thumbnail {
                    img {
                        src: "{url}",
                        class: "{img_class}",
                        loading: "lazy",
                        decoding: "async",
                    }
                } else {
                    div { class: "{placeholder_class}" }
                }
                if let Some(play) = on_play {
                    button {
                        class: "absolute right-3 bottom-3 w-10 h-10 bg-white text-black rounded-full flex items-center justify-center shadow-lg translate-y-4 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300 cursor-pointer",
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            if show_loading {
                                return;
                            }
                            if is_this_source {
                                ctrl.toggle();
                            } else {
                                play.call(());
                            }
                        },
                        i {
                            class: if show_loading {
                                "fa-solid fa-arrows-rotate fa-spin text-sm"
                            } else if show_pause {
                                "fa-solid fa-pause text-sm"
                            } else {
                                "fa-solid fa-play ml-0.5 text-sm"
                            }
                        }
                    }
                }
            }
            div { class: "h-10 flex items-center overflow-hidden",
                p {
                    class: "text-sm font-semibold text-white break-words",
                    style: "display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; text-overflow: ellipsis;",
                    "{title}"
                }
            }
            p {
                class: "text-xs text-white/50 truncate h-4 mt-1",
                "{subtitle}"
            }
        }
    }
}

/// A single song tile. Clicking it starts the source's mix seeded by that
/// song, which the daemon builds and pins the seed to the front of.
#[component]
fn SongCard(item: CatalogItem, track: TrackInfo) -> Element {
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let mut now_playing = use_context::<DiscoverNowPlaying>().0;
    let thumbnail = hooks::artwork::url(item.artwork.as_ref(), hooks::artwork::Size::Thumb);
    let subtitle = item.subtitle.clone().unwrap_or_default();
    let key = track.key.clone();
    let start_radio = components::track_row::radio_handler(key.clone());

    let is_this_source = now_playing.read().as_deref() == Some(key.as_str());
    let is_playing = *ctrl.is_playing.read();
    let is_loading = *ctrl.is_loading.read();
    let show_loading = is_this_source && is_loading;
    let show_pause = is_this_source && is_playing && !is_loading;

    rsx! {
        div {
            class: "shrink-0 w-44 text-left cursor-pointer transition-transform duration-200 ease-out hover:scale-[1.03] hover:-translate-y-0.5 group",
            onclick: {
                let key = key.clone();
                move |_| {
                    if show_loading {
                        return;
                    }
                    if is_this_source {
                        ctrl.toggle();
                        return;
                    }
                    now_playing.set(Some(key.clone()));
                    match &start_radio {
                        Some(radio) => radio.call(()),
                        None => ctrl.set_queue_keys(
                            vec![key.clone()],
                            api::QueueMode::Replace,
                            None,
                        ),
                    }
                }
            },
            div { class: "relative w-44 h-44 mb-3 overflow-hidden rounded-lg",
                if let Some(url) = thumbnail {
                    img {
                        src: "{url}",
                        class: "w-44 h-44 object-cover bg-white/5",
                        loading: "lazy",
                        decoding: "async",
                    }
                } else {
                    div { class: "w-44 h-44 rounded-lg bg-white/5" }
                }
                div { class: "absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 bg-black/40 transition-opacity duration-200",
                    i {
                        class: if show_loading {
                            "fa-solid fa-arrows-rotate fa-spin text-white text-2xl"
                        } else if show_pause {
                            "fa-solid fa-pause text-white text-2xl"
                        } else {
                            "fa-solid fa-play text-white text-2xl"
                        }
                    }
                }
            }
            div { class: "h-10 flex items-center overflow-hidden",
                p {
                    class: "text-sm font-semibold text-white break-words",
                    style: "display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; text-overflow: ellipsis;",
                    "{item.title}"
                }
            }
            p {
                class: "text-xs text-white/50 truncate h-4 mt-1",
                "{subtitle}"
            }
        }
    }
}

/// A catalog playlist or album opened on its own page. Nothing here touches
/// the saved playlists: a browsed list never becomes one of the user's.
#[component]
#[tracing::instrument(name = "render.discover_playlist", skip_all)]
pub fn DiscoverPlaylistDetail(
    selected_playlist_id: Signal<Option<String>>,
    selected_playlist_title: Signal<Option<String>>,
    on_back: EventHandler<()>,
) -> Element {
    let api = hooks::use_api();
    let mut tracks = use_signal(Vec::<TrackInfo>::new);
    let mut artwork = use_signal(|| None::<api::ArtworkRef>);
    let mut loading = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);

    let playlist_id = selected_playlist_id.read().clone();
    let header_title = selected_playlist_title
        .read()
        .clone()
        .unwrap_or_else(String::new);

    // Bumped on every effect run; a spawned fetch checks it before committing,
    // so a slow load for A cannot overwrite B after the user navigated on.
    let mut fetch_gen = use_signal(|| 0u64);
    use_effect(move || {
        let Some(id) = selected_playlist_id.read().clone() else {
            return;
        };
        let my_gen = fetch_gen.with_mut(|generation| {
            *generation += 1;
            *generation
        });
        tracks.set(Vec::new());
        artwork.set(None);
        loading.set(true);
        error.set(None);
        let load_span = tracing::info_span!("playlist.load", playlist_id = %id);
        let api = api.clone();
        spawn(
            async move {
                // Discover routes albums through this viewer too, and an album
                // browse id is not a playlist id, so the kind follows the id.
                let kind = if id.starts_with("MPRE") {
                    CatalogItemKind::Album
                } else {
                    CatalogItemKind::Playlist
                };
                let result = api
                    .catalog_detail(CatalogDetailRequest {
                        kind,
                        id,
                        continuation: None,
                    })
                    .await;
                if *fetch_gen.peek() != my_gen {
                    return;
                }
                match result {
                    Ok(detail) => {
                        tracing::debug!(tracks = detail.tracks.len(), "playlist load complete");
                        artwork.set(detail.artwork);
                        tracks.set(detail.tracks);
                    }
                    Err(failure) => {
                        tracing::warn!(error = %failure, "playlist load failed");
                        error.set(Some(failure_text(&failure)));
                    }
                }
                loading.set(false);
            }
            .instrument(load_span),
        );
    });

    if playlist_id.is_none() {
        return rsx! {
            div { class: "flex items-center justify-center h-full text-white/60 p-12",
                p { "{i18n::t(\"playlist_not_found\")}" }
            }
        };
    }

    // Loading and error keep a lightweight header; the loaded state hands off
    // to the shared track list so a catalog list looks like every other one.
    if *loading.read() {
        return rsx! {
            div { class: "p-6 md:p-10 max-w-[1600px] mx-auto",
                BackButton { on_back }
                div { class: "flex justify-center py-24",
                    i { class: "fa-solid fa-arrows-rotate fa-spin text-2xl text-white/60" }
                }
            }
        };
    }
    if let Some(err) = error.read().clone() {
        return rsx! {
            div { class: "p-6 md:p-10 max-w-[1600px] mx-auto",
                BackButton { on_back }
                div { class: "py-12 text-rose-400 text-sm", "{err}" }
            }
        };
    }

    let track_list = tracks.read().clone();
    let cover_url = hooks::artwork::url(artwork.read().as_ref(), hooks::artwork::Size::Thumb);

    rsx! {
        div { class: "absolute inset-0 flex flex-col overflow-hidden p-8",
            components::track_list_view::TrackListView {
                name: header_title.clone(),
                description: String::new(),
                cover_url,
                tracks: track_list,
                is_album: false,
                on_close: move |_| on_back.call(()),
            }
        }
    }
}

#[component]
fn BackButton(on_back: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: "inline-flex items-center gap-2 text-white/70 hover:text-white text-sm cursor-pointer mb-6 group",
            onclick: move |_| on_back.call(()),
            i { class: "fa-solid fa-chevron-left text-xs transition-transform group-hover:-translate-x-0.5" }
        }
    }
}

/// The source's own artist profile, used wherever the active source presents
/// artists remotely. Its sections are catalog shelves, so they get the same
/// tiles, hover-play and scrolling as the browse home.
///
/// Callers that know the artist's catalog id pass it; callers that only have a
/// name pass that, and the daemon resolves it.
#[component]
pub fn DiscoverArtistPage(
    selected_artist_id: Signal<Option<String>>,
    selected_artist_name: Signal<String>,
    on_back: EventHandler<()>,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(String, String)>,
    on_open_artist: EventHandler<(String, String)>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let api = hooks::use_api();
    let ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let now_playing = use_context::<DiscoverNowPlaying>().0;
    let cache = use_context::<DiscoverPrefetchCache>().0;
    let mut artist = use_signal(|| None::<api::CatalogDetail>);
    let mut loading = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);

    // Generation guard: drop a late answer when the user has moved on.
    let mut fetch_gen = use_signal(|| 0u64);
    use_effect(move || {
        // The selection is (id, name): the id wins, the name is the fallback
        // the daemon resolves.
        let id = selected_artist_id.read().clone();
        let name = selected_artist_name.read().clone();
        let Some(reference) = id.or_else(|| {
            let name = name.trim();
            (!name.is_empty()).then(|| name.to_string())
        }) else {
            return;
        };
        let my_gen = fetch_gen.with_mut(|generation| {
            *generation += 1;
            *generation
        });
        artist.set(None);
        loading.set(true);
        error.set(None);
        let artist_span = tracing::info_span!("artist.load", artist = %reference);
        let api = api.clone();
        spawn(
            async move {
                let result = api
                    .catalog_detail(CatalogDetailRequest {
                        kind: CatalogItemKind::Artist,
                        id: reference,
                        continuation: None,
                    })
                    .await;
                if *fetch_gen.peek() != my_gen {
                    return;
                }
                match result {
                    Ok(detail) => artist.set(Some(detail)),
                    Err(failure) => error.set(Some(failure_text(&failure))),
                }
                loading.set(false);
            }
            .instrument(artist_span),
        );
    });

    if selected_artist_id.read().is_none() && selected_artist_name.read().trim().is_empty() {
        return rsx! {
            div { class: "p-12 text-white/60", "{i18n::t(\"artist_none_selected\")}" }
        };
    }

    rsx! {
        div { class: "max-w-[1600px] mx-auto",
            button {
                class: "inline-flex items-center gap-2 text-white/70 hover:text-white text-sm cursor-pointer mt-6 ml-6 md:ml-10 mb-2 group",
                onclick: move |_| on_back.call(()),
                i { class: "fa-solid fa-chevron-left text-xs transition-transform group-hover:-translate-x-0.5" }
            }

            if *loading.read() {
                div { class: "flex justify-center py-24",
                    i { class: "fa-solid fa-arrows-rotate fa-spin text-2xl text-white/60" }
                }
            } else if let Some(err) = error.read().clone() {
                div { class: "py-12 px-6 md:px-10 text-rose-400 text-sm", "{err}" }
            } else if let Some(detail) = artist.read().clone() {
                {
                    let banner = hooks::artwork::url(detail.artwork.as_ref(), hooks::artwork::Size::Thumb);
                    let banner_style = banner
                        .map(|url| format!("background-image: linear-gradient(to bottom, rgba(0,0,0,0.2) 0%, rgba(0,0,0,0.95) 100%), url('{url}'); background-size: cover; background-position: center; min-height: 360px;"))
                        .unwrap_or_else(|| "min-height: 280px;".to_string());
                    let shuffle_id = detail.playback_id.clone();
                    rsx! {
                        div {
                            class: "relative overflow-hidden flex flex-col justify-end",
                            style: "{banner_style}",
                            div { class: "px-6 md:px-10 pt-16 pb-10 flex flex-col gap-4",
                                h1 { class: "text-4xl md:text-6xl font-black text-white break-words drop-shadow-lg", "{detail.title}" }
                                if let Some(subtitle) = detail.subtitle.clone() {
                                    p { class: "text-sm text-white/70", "{subtitle}" }
                                }
                                if let Some(description) = detail.description.clone() {
                                    p { class: "text-sm text-white/60 max-w-3xl line-clamp-3", "{description}" }
                                }
                                div { class: "flex gap-3 mt-2",
                                    if let Some(id) = shuffle_id {
                                        button {
                                            class: "inline-flex items-center gap-2 bg-white text-black px-6 py-2.5 rounded-full font-bold hover:scale-105 active:scale-95 transition-transform cursor-pointer",
                                            onclick: move |_| {
                                                play_catalog(
                                                    CatalogItemKind::Playlist,
                                                    id.clone(),
                                                    ctrl,
                                                    now_playing,
                                                    cache,
                                                );
                                            },
                                            i { class: "fa-solid fa-shuffle text-[11px]" }
                                            span { class: "text-sm", "{i18n::t(\"shuffle\")}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "px-6 md:px-10 pt-8",
                            for (idx, shelf) in detail.shelves.iter().enumerate() {
                                ShelfRow {
                                    key: "{idx}",
                                    shelf: shelf.clone(),
                                    scroll_id: format!("artist-shelf-{idx}"),
                                    on_select_album: on_select_album,
                                    on_select_playlist: on_select_playlist,
                                    on_open_artist: on_open_artist,
                                    on_search_artist: on_search_artist,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
