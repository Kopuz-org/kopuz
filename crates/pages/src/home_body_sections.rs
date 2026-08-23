//! Rendering for individual home-page sections.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_server_section(
    key: &str,
    config: Signal<AppConfig>,
    edit: bool,
    is_vaxry: bool,
    listen_now_style: ListenNowStyle,
    jellyfin_shuffled: Vec<AlbumCard>,
    hero_cover: Option<String>,
    continue_listening: Vec<(Track, Option<Album>, Option<String>)>,
    hero_entry: Option<(Track, Option<Album>, Option<String>)>,
    artists: Vec<(String, Option<String>)>,
    new_releases: Vec<AlbumCard>,
    made_for_you: (String, Vec<AlbumCard>),
    recently_added: Vec<AlbumCard>,
    recent_playlists: Vec<(String, String, usize, Option<String>)>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
    active_card_menu: Signal<Option<String>>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    match key {
        "hero" => rsx! {
            ServerHeroBanner {
                config,
                edit,
                is_vaxry,
                hero_entry,
                hero_cover,
                on_play_album,
            }
        },
        "continue_listening" => render_continue_listening(
            is_vaxry,
            continue_listening,
            on_select_album,
            on_play_album,
            active_card_menu,
            scroll_container,
        ),
        "listen_now" => render_listen_now(
            is_vaxry,
            listen_now_style,
            jellyfin_shuffled,
            on_select_album,
            on_play_album,
        ),
        "top_artists" => render_top_artists(is_vaxry, artists, on_search_artist, scroll_container),
        "new_releases" => render_albums_row(
            "jelly-albums-scroll",
            i18n::t("new_releases").to_string(),
            i18n::t("albums").to_string(),
            is_vaxry,
            new_releases,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "made_for_you" => {
            let (genre, albums) = made_for_you;
            let eyebrow = if genre.is_empty() {
                i18n::t("music").to_string()
            } else {
                genre
            };
            render_albums_row(
                "jelly-made-for-you-scroll",
                i18n::t("made_for_you").to_string(),
                eyebrow,
                is_vaxry,
                albums,
                on_select_album,
                on_play_album,
                scroll_container,
            )
        }
        "recently_added" => render_albums_row(
            "jelly-recently-added-scroll",
            i18n::t("recently_added").to_string(),
            i18n::t("library").to_string(),
            is_vaxry,
            recently_added,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "playlists" => render_playlists(
            config,
            is_vaxry,
            recent_playlists,
            on_select_playlist,
            active_card_menu,
            scroll_container,
        ),
        _ => rsx! {},
    }
}

#[component]
fn ServerHeroBanner(
    mut config: Signal<AppConfig>,
    edit: bool,
    is_vaxry: bool,
    hero_entry: Option<(Track, Option<Album>, Option<String>)>,
    hero_cover: Option<String>,
    on_play_album: EventHandler<String>,
) -> Element {
    let mut is_resizing = use_signal(|| false);
    let mut start_y = use_signal(|| 0.0_f64);
    let mut start_h = use_signal(|| 0_u32);

    let source = use_active_source();
    // The track's own `album_id` (not the resolved `Album`, which lags behind a
    // separate albums query) — so the play button and the favorite-state heart
    // work the instant the hero track renders, not only once albums load.
    let hero_album_id_val = hero_entry
        .as_ref()
        .map(|(t, _, _)| t.album_id.clone())
        .unwrap_or_default();
    let mut hero_album_id = use_signal(|| hero_album_id_val.clone());
    if *hero_album_id.peek() != hero_album_id_val {
        hero_album_id.set(hero_album_id_val);
    }
    let hero_album_id_memo = use_memo(move || hero_album_id.read().clone());
    let hero_tracks_res = use_album_tracks(source, hero_album_id_memo);
    let favorites_res = use_favorites();

    use_effect(move || {
        if *is_resizing.read() {
            let sy = *start_y.peek();
            let sh = *start_h.peek();
            spawn(async move {
                let mut eval = dioxus::document::eval(
                    r#"
                    const handleMouseMove = (e) => { dioxus.send(e.clientY); };
                    const handleMouseUp = () => {
                        dioxus.send("stop");
                        window.removeEventListener('mousemove', handleMouseMove);
                        window.removeEventListener('mouseup', handleMouseUp);
                    };
                    window.addEventListener('mousemove', handleMouseMove);
                    window.addEventListener('mouseup', handleMouseUp);
                    "#,
                );

                while let Ok(val) = eval.recv::<serde_json::Value>().await {
                    if let Some(y) = val.as_f64() {
                        let delta = y - sy;
                        let new_h = ((sh as f64) + delta).clamp(140.0, 800.0) as u32;
                        config.write().hero_height = new_h;
                    } else if val.as_str() == Some("stop") {
                        is_resizing.set(false);
                        break;
                    }
                }
            });
        }
    });

    let hero_height = config.read().hero_height;
    let section_class = if is_vaxry {
        "relative rounded-xl overflow-hidden mb-10"
    } else {
        "relative rounded-xl overflow-hidden mb-12"
    };
    let section_style = format!("height: {hero_height}px;");

    let show_empty_state = hero_entry.is_none();
    let hero_title = hero_entry
        .as_ref()
        .map(|(track, _, _)| track.title.clone())
        .unwrap_or_default();
    let hero_artist = hero_entry
        .as_ref()
        .map(|(track, album_opt, _)| {
            if !is_unknown_artist(&track.artist) {
                return track.artist.clone();
            }
            album_opt
                .as_ref()
                .map(|a| a.artist.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    rsx! {
        section { class: "{section_class}", style: "{section_style}",
            if !show_empty_state {
                if let Some((_, _album_opt, entry_cover)) = hero_entry.as_ref() {
                    div { class: "absolute inset-0 overflow-hidden",
                        if let Some(url) = hero_cover.clone().or(entry_cover.clone()) {
                            img {
                                src: "{url}",
                                class: "absolute inset-0 w-full h-full object-cover object-center",
                                decoding: "async",
                            }
                        }
                        div { class: "absolute inset-0 bg-gradient-to-r from-black/95 via-black/60 to-black/20" }
                        div { class: "absolute inset-0 bg-gradient-to-t from-black/50 to-transparent" }
                    }
                }
                div { class: "relative h-full flex flex-col justify-center p-8 md:p-12",
                    span { class: "text-indigo-400 font-bold text-[10px] mb-3 flex items-center gap-2",
                        i { class: "fa-solid fa-star text-[8px]" }
                        "{i18n::t(\"featured_album\")}"
                    }
                    h1 { class: "text-3xl md:text-5xl font-semibold tracking-tight text-white mb-4 leading-tight break-words", style: "overflow: hidden; text-overflow:ellipsis;white-space: nowrap;", "{hero_title}" }
                    if !hero_artist.is_empty() {
                        p { class: "text-base md:text-lg text-white/60 mb-8 font-medium line-clamp-1 max-w-md", "{i18n::t_with(\"by_artist_full\", &[(\"artist\", hero_artist.clone())])}" }
                    }
                    div { class: "flex items-center gap-4",
                        button {
                            class: "flex items-center gap-3 bg-white text-black px-8 py-3 rounded-full font-bold hover:bg-white/90 hover:scale-105 active:scale-95 transition-all w-fit",
                            onclick: {
                                let id = hero_entry.as_ref().map(|(t, _, _)| t.album_id.clone());
                                move |_| {
                                    if let Some(id) = id.clone().filter(|s| !s.is_empty()) {
                                        on_play_album.call(id)
                                    }
                                }
                            },
                            i { class: "fa-solid fa-play text-[10px]" }
                            span { class: "text-sm", "{i18n::t(\"start_listening\")}" }
                        }
                        {
                            let jelly_hero_fav = {
                                let tracks = if hero_album_id.read().is_empty() {
                                    Vec::new()
                                } else {
                                    hero_tracks_res.read().clone().unwrap_or_default()
                                };
                                let favs: std::collections::HashSet<String> = favorites_res
                                    .read()
                                    .clone()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .collect();
                                !tracks.is_empty() && tracks.iter().all(|t| {
                                    let id = t.id.key();
                                    !id.is_empty() && favs.contains(id.as_ref())
                                })
                            };
                            let hero_heart_class = if jelly_hero_fav {
                                "w-11 h-11 rounded-full bg-white/10 border border-white/20 flex items-center justify-center text-red-400 hover:bg-white/20 transition-all"
                            } else {
                                "w-11 h-11 rounded-full bg-white/10 border border-white/20 flex items-center justify-center text-white hover:bg-white/20 transition-all"
                            };
                            let hero_heart_icon = if jelly_hero_fav { "fa-solid fa-heart" } else { "fa-regular fa-heart" };
                            rsx! {
                                button {
                                    class: "{hero_heart_class}",
                                    onclick: move |_| {
                                        let tracks: Vec<_> = if hero_album_id.peek().is_empty() {
                                            Vec::new()
                                        } else {
                                            hero_tracks_res.read().clone().unwrap_or_default()
                                        };
                                        hooks::favorites::set_favorite_many(tracks, !jelly_hero_fav);
                                    },
                                    i { class: "{hero_heart_icon}" }
                                }
                            }
                        }
                    }
                }
            }

            if edit {
                div {
                    class: "absolute bottom-0 left-0 right-0 h-3 cursor-ns-resize flex items-center justify-center bg-black/40 hover:bg-indigo-500/40 transition-colors z-10",
                    title: "Drag to resize",
                    onmousedown: move |evt| {
                        evt.stop_propagation();
                        start_y.set(evt.client_coordinates().y);
                        start_h.set(config.peek().hero_height);
                        is_resizing.set(true);
                    },
                    div { class: "w-10 h-1 rounded-full bg-white/60" }
                }
            }
        }
    }
}

/// The overflow menu on a home song card: the subset of the track row's actions
/// that needs no modal of its own, so home can start a radio (and queue a track)
/// without the page growing the row's playlist/metadata plumbing.
#[derive(Clone, Copy)]
enum SongCardAction {
    PlayNext,
    AddToQueue,
    StartRadio,
    Share,
}

fn song_card_actions(can_radio: bool) -> (Vec<MenuAction>, Vec<SongCardAction>) {
    let mut entries = vec![
        (
            MenuAction::new(i18n::t("play_next"), "fa-solid fa-forward-step"),
            SongCardAction::PlayNext,
        ),
        (
            MenuAction::new(i18n::t("add_to_queue"), "fa-solid fa-list-ul"),
            SongCardAction::AddToQueue,
        ),
    ];
    if can_radio {
        entries.push((
            MenuAction::new(
                components::radio_actions::radio_label(),
                components::radio_actions::RADIO_ICON,
            ),
            SongCardAction::StartRadio,
        ));
    }
    entries.push((
        MenuAction::new(i18n::t("share_musicbrainz"), "fa-solid fa-share-nodes"),
        SongCardAction::Share,
    ));
    entries.into_iter().unzip()
}

fn render_continue_listening(
    is_vaxry: bool,
    tracks: Vec<(Track, Option<Album>, Option<String>)>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    mut active_card_menu: Signal<Option<String>>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if tracks.is_empty() {
        return rsx! { div {} };
    }
    let mut ctrl = consume_context::<hooks::PlayerController>();
    let active_source = consume_context::<Signal<::server::source::ActiveSource>>();
    let can_radio = active_source.read().capabilities().radio.track;
    let (song_actions, song_action_kinds) = song_card_actions(can_radio);
    rsx! {
        section { class: if is_vaxry { "mb-10" } else { "mb-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"library\")}" }
                    }
                    h2 { class: "text-2xl font-semibold tracking-tight text-white", "{i18n::t(\"continue_listening\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-continue-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-continue-scroll", 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "jelly-continue-scroll",
                class: "flex overflow-x-auto gap-5 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                ontouchstart: move |evt| evt.stop_propagation(),
                for (track, album_opt, cover_url) in tracks {
                    {
                        let title = track.title.clone();
                        let artist = track.artist.clone();
                        let album_title = album_opt
                            .as_ref()
                            .map(|a| a.title.clone())
                            .unwrap_or_else(|| track.album.clone());
                        let album_id_opt = album_opt.as_ref().map(|a| a.id.clone());
                        let album_id_click = album_id_opt.clone();
                        let album_id_play = album_id_opt.clone();
                        let key = track.id.uid();
                        let actions = song_actions.clone();
                        let action_kinds = song_action_kinds.clone();
                        // Resolved during render, not in the click closure: the
                        // handler reads context, which a closure cannot do.
                        let start_radio = components::radio_actions::track_radio_handler(
                            track.clone(),
                        );
                        let open_key = key.clone();
                        let is_menu_open = active_card_menu.read().as_deref() == Some(key.as_str());
                        let menu_track = track.clone();
                        rsx! {
                            div {
                                key: "{key}",
                                class: "flex-none w-44 group cursor-pointer",
                                onclick: move |_| {
                                    if let Some(id) = album_id_click.clone() {
                                        on_select_album.call(id);
                                    }
                                },
                                div { class: "aspect-square rounded-xl bg-stone-800 mb-3 overflow-hidden relative",
                                    if let Some(url) = cover_url {
                                        img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                                    } else {
                                        div { class: "w-full h-full flex items-center justify-center",
                                            i { class: "fa-solid fa-music text-3xl text-white/20" }
                                        }
                                    }
                                    components::album_play_button::AlbumPlayButton {
                                        album_id: album_id_play.clone(),
                                        on_play_album,
                                        class: "absolute right-2 bottom-2 w-10 h-10 rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all translate-y-2 group-hover:translate-y-0".to_string(),
                                        style: "background: var(--color-indigo-500);".to_string(),
                                        icon_extra: "text-white text-xs".to_string(),
                                    }
                                    div {
                                        class: "absolute right-1 top-1",
                                        onclick: move |evt| evt.stop_propagation(),
                                        DotsMenu {
                                            actions,
                                            is_open: is_menu_open,
                                            on_open: move |_| active_card_menu.set(Some(open_key.clone())),
                                            on_close: move |_| active_card_menu.set(None),
                                            button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                                            anchor: "right".to_string(),
                                            on_action: move |idx: usize| {
                                                active_card_menu.set(None);
                                                match action_kinds.get(idx) {
                                                    Some(SongCardAction::PlayNext) => {
                                                        ctrl.queue_play_next(vec![menu_track.clone()]);
                                                    }
                                                    Some(SongCardAction::AddToQueue) => {
                                                        ctrl.add_to_queue(vec![menu_track.clone()]);
                                                    }
                                                    Some(SongCardAction::StartRadio) => {
                                                        if let Some(handler) = start_radio {
                                                            handler.call(());
                                                        }
                                                    }
                                                    Some(SongCardAction::Share) => {
                                                        let src = active_source.peek().clone();
                                                        components::track_row::share_track(menu_track.clone(), src);
                                                    }
                                                    None => {}
                                                }
                                            },
                                        }
                                    }
                                }
                                h3 { class: "text-white font-semibold truncate text-sm", "{title}" }
                                p { class: "text-xs truncate mt-0.5 text-white/50", "{artist} — {album_title}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_listen_now(
    is_vaxry: bool,
    listen_now_style: ListenNowStyle,
    jellyfin_shuffled: Vec<AlbumCard>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
) -> Element {
    if jellyfin_shuffled.is_empty() {
        return rsx! { div {} };
    }
    let use_cards = listen_now_style == ListenNowStyle::Cards;
    rsx! {
        section { class: if is_vaxry { "mb-10" } else { "mb-12" },
            div { class: "flex items-end justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"music\")}" }
                    }
                    h2 { class: if is_vaxry { "text-2xl font-semibold tracking-tight text-white" } else { "text-3xl font-semibold tracking-tight text-white leading-none" }, "{i18n::t(\"listen_now\")}" }
                }
            }
            if use_cards {
                div { class: "flex overflow-x-auto gap-4 pb-4 scrollbar-hide scroll-smooth -mx-2 px-2",
                    ontouchstart: move |evt| evt.stop_propagation(),
                    for (album_id, title, artist, cover_url) in jellyfin_shuffled.iter().skip(1).take(10).cloned() {
                        div {
                            class: "flex-none w-40 group cursor-pointer",
                            onclick: {
                                let id = album_id.clone();
                                move |_| on_select_album.call(id.clone())
                            },
                            div { class: "aspect-square rounded-xl bg-stone-800 mb-2 overflow-hidden relative",
                                if let Some(url) = cover_url {
                                    img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                                } else {
                                    div { class: "w-full h-full flex items-center justify-center",
                                        i { class: "fa-solid fa-compact-disc text-2xl text-white/20" }
                                    }
                                }
                                components::album_play_button::AlbumPlayButton {
                                    album_id: Some(album_id.clone()),
                                    on_play_album,
                                    class: "absolute right-2 bottom-2 w-9 h-9 rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all translate-y-2 group-hover:translate-y-0".to_string(),
                                    style: "background: var(--color-indigo-500);".to_string(),
                                    icon_extra: "text-white text-xs".to_string(),
                                }
                            }
                            h3 { class: "text-white font-semibold truncate text-sm", "{title}" }
                            p { class: "text-xs truncate mt-0.5", style: "color: rgba(255,255,255,0.45);", "{artist}" }
                        }
                    }
                }
            } else {
                div { class: "grid grid-cols-[repeat(auto-fill,minmax(350px,1fr))] gap-4",
                    for (album_id, title, artist, cover_url) in jellyfin_shuffled.iter().skip(1).take(8).cloned() {
                        div {
                            class: "flex items-center bg-white/5 hover:bg-white/10 border border-white/5 rounded-xl cursor-pointer transition-all duration-300 group overflow-hidden pr-4",
                            onclick: {
                                let id = album_id.clone();
                                move |_| on_select_album.call(id.clone())
                            },
                            div { class: "w-16 h-16 md:w-20 md:h-20 flex-shrink-0 bg-stone-800/50 relative overflow-hidden",
                                if let Some(url) = cover_url {
                                    img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
                                } else {
                                    div { class: "w-full h-full flex items-center justify-center",
                                        i { class: "fa-solid fa-compact-disc text-xl text-white/20" }
                                    }
                                }
                                div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                            }
                            div { class: "p-4 flex-1 min-w-0 flex flex-col justify-center",
                                h3 { class: "text-white font-bold truncate text-sm md:text-base", "{title}" }
                                p { class: "text-xs text-white/50 truncate font-semibold mt-1", "{artist}" }
                            }
                            div { class: "opacity-0 group-hover:opacity-100 transition-all duration-300 translate-x-2 group-hover:translate-x-0",
                                components::album_play_button::AlbumPlayButton {
                                    album_id: Some(album_id.clone()),
                                    on_play_album,
                                    class: "w-8 h-8 rounded-full bg-white/10 flex items-center justify-center hover:bg-white/20 transition-colors".to_string(),
                                    icon_extra: "text-white/80 text-xs".to_string(),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_top_artists(
    is_vaxry: bool,
    artists: Vec<(String, Option<String>)>,
    on_search_artist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if artists.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_vaxry { "mt-10" } else { "mt-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"artists\")}" }
                    }
                    h2 { class: "text-2xl font-semibold tracking-tight text-white", "{i18n::t(\"top_artists\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-artists-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-artists-scroll", 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "jelly-artists-scroll",
                class: "flex overflow-x-auto gap-6 pb-6 pt-2 overflow-y-visible scrollbar-hide scroll-smooth -mx-2 px-2",
                ontouchstart: move |evt| evt.stop_propagation(),
                for (artist, cover_url) in artists {
                    div {
                        class: "flex-none w-32 md:w-40 group cursor-pointer",
                        onclick: {
                            let artist = artist.clone();
                            move |_| on_search_artist.call(artist.clone())
                        },
                        div { class: "w-32 h-32 md:w-40 md:h-40 rounded-full bg-stone-800/80 mb-4 overflow-hidden transition-all duration-500 relative mx-auto",
                            if let Some(url) = cover_url {
                                img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
                            } else {
                                div { class: "w-full h-full flex items-center justify-center",
                                    i { class: "fa-solid fa-microphone text-4xl text-white/20" }
                                }
                            }
                            div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300 rounded-full" }
                        }
                        h3 { class: "text-white font-bold truncate text-center px-2 text-sm md:text-base group-hover:text-indigo-400 transition-colors", "{artist}" }
                    }
                }
            }
        }
    }
}

fn render_albums_row(
    scroll_id: &'static str,
    title: String,
    eyebrow: String,
    is_vaxry: bool,
    albums: Vec<AlbumCard>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if albums.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_vaxry { "mt-10" } else { "mt-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{eyebrow}" }
                    }
                    h2 { class: "text-2xl font-semibold tracking-tight text-white", "{title}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container(scroll_id, -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container(scroll_id, 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "{scroll_id}",
                class: "flex overflow-x-auto gap-5 pb-6 pt-2 overflow-y-visible scrollbar-hide scroll-smooth -mx-2 px-2",
                ontouchstart: move |evt| evt.stop_propagation(),
                for (album_id, title, artist, cover_url) in albums {
                    div {
                        class: "flex-none w-36 md:w-48 group cursor-pointer",
                        onclick: {
                            let id = album_id.clone();
                            move |_| on_select_album.call(id.clone())
                        },
                        div { class: "aspect-square rounded-xl bg-stone-800/80 mb-4 overflow-hidden transition-all duration-300 relative",
                            if let Some(url) = cover_url {
                                img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                            } else {
                                div { class: "w-full h-full flex items-center justify-center border border-white/5 rounded-lg",
                                    i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                }
                            }
                            div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                            components::album_play_button::AlbumPlayButton {
                                album_id: Some(album_id.clone()),
                                on_play_album,
                                class: "absolute right-3 bottom-3 w-10 h-10 bg-white text-black rounded-full flex items-center justify-center translate-y-4 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300".to_string(),
                                icon_extra: "text-sm".to_string(),
                            }
                        }
                        h3 { class: "text-white font-bold truncate text-sm md:text-base px-1", "{title}" }
                        p { class: "text-xs md:text-sm text-white/50 truncate px-1 font-semibold mt-1", "{artist}" }
                    }
                }
            }
        }
    }
}

fn render_playlists(
    _config: Signal<AppConfig>,
    is_vaxry: bool,
    recent_playlists: Vec<(String, String, usize, Option<String>)>,
    on_select_playlist: EventHandler<String>,
    mut active_card_menu: Signal<Option<String>>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if recent_playlists.is_empty() {
        return rsx! { div {} };
    }
    // Radio is the one playlist action a home card can offer without the
    // playlists page's folder/rename state, so the whole menu rides its gate.
    let can_radio = consume_context::<Signal<::server::source::ActiveSource>>()
        .read()
        .capabilities()
        .radio
        .playlist;
    let radio_actions = vec![MenuAction::new(
        components::radio_actions::radio_label(),
        components::radio_actions::RADIO_ICON,
    )];
    rsx! {
        section { class: if is_vaxry { "mt-10" } else { "mt-16" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_vaxry {
                        p { class: "text-[10px] font-bold mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"library\")}" }
                    }
                    h2 { class: "text-2xl font-semibold tracking-tight text-white", "{i18n::t(\"playlists\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-playlists-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-9 h-9 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-colors active:scale-95",
                        onclick: move |_| scroll_container("jelly-playlists-scroll", 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "jelly-playlists-scroll",
                class: "flex overflow-x-auto gap-6 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                ontouchstart: move |evt| evt.stop_propagation(),
                for (id, name, track_count, cover_url) in recent_playlists {
                    {
                        let start_radio = components::radio_actions::playlist_radio_handler(id.clone());
                        let is_menu_open = active_card_menu.read().as_deref() == Some(id.as_str());
                        let open_key = id.clone();
                        let actions = radio_actions.clone();
                        rsx! {
                            div {
                                key: "{id}",
                                class: "flex-none w-40 md:w-48 group cursor-pointer",
                                onclick: {
                                    let id = id.clone();
                                    move |_| on_select_playlist.call(id.clone())
                                },
                                div { class: "aspect-square rounded-xl bg-white/5 mb-4 overflow-hidden transition-all duration-500 relative",
                                    if let Some(url) = cover_url {
                                        img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-700", decoding: "async", loading: "lazy" }
                                    } else {
                                        div { class: "w-full h-full flex items-center justify-center bg-gradient-to-br from-indigo-600/20 to-purple-600/20 group-hover:scale-110 transition-transform duration-700",
                                            i { class: "fa-solid fa-music text-5xl opacity-40 text-white" }
                                        }
                                    }
                                    div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                                    if can_radio {
                                        div {
                                            class: "absolute right-1 top-1",
                                            onclick: move |evt| evt.stop_propagation(),
                                            DotsMenu {
                                                actions,
                                                is_open: is_menu_open,
                                                on_open: move |_| active_card_menu.set(Some(open_key.clone())),
                                                on_close: move |_| active_card_menu.set(None),
                                                button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                                                anchor: "right".to_string(),
                                                on_action: move |_: usize| {
                                                    active_card_menu.set(None);
                                                    if let Some(handler) = start_radio {
                                                        handler.call(());
                                                    }
                                                },
                                            }
                                        }
                                    }
                                }
                                div {
                                    h3 { class: "text-white font-bold truncate text-sm md:text-base px-1 group-hover:text-indigo-400 transition-colors", "{name}" }
                                    p { class: "text-xs md:text-sm text-white/40 truncate px-1 font-semibold mt-1",
                                        {
                                            let track_text = i18n::t_with("music_playlist_count", &[("count", track_count.to_string())]);
                                            rsx! { "{track_text}" }
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
