use config::{AppConfig, ArtistPhotoSource, ListenNowStyle, UiStyle};
use dioxus::prelude::*;
use rand::seq::SliceRandom;
use rand::thread_rng;
use reader::{Album, FavoritesStore, Library, PlaylistStore, Track};
use std::collections::HashMap;
use std::path::PathBuf;

fn normalize_artist_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn section_label(key: &str) -> String {
    let i18n_key = match key {
        "hero" => "home_section_hero",
        "continue_listening" => "home_section_continue_listening",
        "listen_now" => "home_section_listen_now",
        "top_artists" => "home_section_top_artists",
        "new_releases" => "home_section_new_releases",
        "made_for_you" => "home_section_made_for_you",
        "recently_added" => "home_section_recently_added",
        "playlists" => "home_section_playlists",
        _ => return key.to_string(),
    };
    i18n::t(i18n_key).to_string()
}

#[component]
pub fn LocalHome(
    library: Signal<Library>,
    playlist_store: Signal<PlaylistStore>,
    favorites_store: Signal<FavoritesStore>,
    edit_mode: Signal<bool>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let mut config = use_context::<Signal<AppConfig>>();

    let recent_albums = use_memo(move || {
        let lib = library.read();
        let mut unique_albums = Vec::new();
        let mut seen_titles = std::collections::HashSet::new();
        for album in lib.albums.iter().rev() {
            let title_key = album.title.trim().to_lowercase();
            if seen_titles.insert(title_key) {
                unique_albums.push(album.clone());
            }
            if unique_albums.len() >= 10 {
                break;
            }
        }
        unique_albums
    });

    let new_release_albums = use_memo(move || {
        let lib = library.read();
        let mut albums = lib.albums.clone();
        albums.sort_by(|a, b| b.year.cmp(&a.year));
        let mut unique_albums = Vec::new();
        let mut seen_titles = std::collections::HashSet::new();
        for album in albums {
            let title_key = album.title.trim().to_lowercase();
            if seen_titles.insert(title_key) {
                unique_albums.push(album);
            }
            if unique_albums.len() >= 10 {
                break;
            }
        }
        unique_albums
    });

    let recent_playlists = use_memo(move || {
        let store = playlist_store.read();
        store
            .playlists
            .iter()
            .rev()
            .take(10)
            .cloned()
            .map(|p| {
                (
                    p.id,
                    p.name,
                    p.tracks.len(),
                    p.tracks
                        .first()
                        .and_then(|p| p.to_str())
                        .map(|s| s.to_string()),
                )
            })
            .collect::<Vec<_>>()
    });

    let artists = use_memo(move || {
        let lib = library.read();
        let use_artist_photo = config.read().artist_photo_source == ArtistPhotoSource::ArtistPhoto;
        let normalized_local_artist_images: HashMap<String, PathBuf> = lib
            .local_artist_images
            .iter()
            .map(|(artist, path)| (normalize_artist_key(artist), path.clone()))
            .collect();
        let mut unique_artists = std::collections::HashSet::new();
        let mut artist_list = Vec::new();
        for album in &lib.albums {
            let normalized_artist = normalize_artist_key(&album.artist);
            if unique_artists.insert(normalized_artist.clone()) {
                let cover = if use_artist_photo {
                    normalized_local_artist_images
                        .get(&normalized_artist)
                        .cloned()
                        .or_else(|| album.cover_path.clone())
                } else {
                    album.cover_path.clone()
                };
                artist_list.push((album.artist.clone(), cover));
            }
            if artist_list.len() >= 10 {
                return artist_list;
            }
        }
        if use_artist_photo {
            for (artist, image_path) in &lib.local_artist_images {
                if unique_artists.insert(normalize_artist_key(artist)) {
                    artist_list.push((artist.clone(), Some(image_path.clone())));
                }
                if artist_list.len() >= 10 {
                    break;
                }
            }
        }
        artist_list
    });

    let local_shuffled = use_memo(move || {
        let lib = library.read();
        let mut unique_albums = Vec::new();
        let mut seen_titles = std::collections::HashSet::new();
        for album in &lib.albums {
            let title_key = album.title.trim().to_lowercase();
            if seen_titles.insert(title_key) {
                unique_albums.push(album.clone());
            }
        }
        if unique_albums.is_empty() {
            return Vec::new();
        }
        let mut rng = thread_rng();
        unique_albums.shuffle(&mut rng);
        unique_albums
    });

    let continue_listening = use_memo(move || {
        let lib = library.read();
        let conf = config.read();
        let track_by_path: HashMap<String, &Track> = lib
            .tracks
            .iter()
            .map(|t| (t.path.to_string_lossy().to_string(), t))
            .collect();
        let album_by_id: HashMap<&str, &Album> =
            lib.albums.iter().map(|a| (a.id.as_str(), a)).collect();
        let mut out: Vec<(Track, Option<Album>)> = Vec::new();
        let mut seen_albums = std::collections::HashSet::new();
        for path in conf.recently_played.iter() {
            if let Some(track) = track_by_path.get(path) {
                let album = album_by_id.get(track.album_id.as_str()).copied().cloned();
                if let Some(ref a) = album {
                    if !seen_albums.insert(a.id.clone()) {
                        continue;
                    }
                }
                out.push(((*track).clone(), album));
                if out.len() >= 10 {
                    break;
                }
            }
        }
        out
    });

    let made_for_you = use_memo(move || {
        let lib = library.read();
        let conf = config.read();
        let mut genre_scores: HashMap<String, u64> = HashMap::new();
        let album_genre: HashMap<&str, &str> = lib
            .albums
            .iter()
            .map(|a| (a.id.as_str(), a.genre.as_str()))
            .collect();
        for track in &lib.tracks {
            let path = track.path.to_string_lossy().to_string();
            let plays = conf.listen_counts.get(&path).copied().unwrap_or(0);
            if plays == 0 {
                continue;
            }
            if let Some(genre) = album_genre.get(track.album_id.as_str()) {
                if genre.trim().is_empty() {
                    continue;
                }
                *genre_scores.entry(genre.to_string()).or_insert(0) += plays;
            }
        }
        let top_genre = genre_scores
            .into_iter()
            .max_by_key(|(_, v)| *v)
            .map(|(k, _)| k);
        let Some(top_genre) = top_genre else {
            return (String::new(), Vec::<Album>::new());
        };
        let mut albums: Vec<Album> = lib
            .albums
            .iter()
            .filter(|a| a.genre == top_genre)
            .cloned()
            .collect();
        let mut rng = thread_rng();
        albums.shuffle(&mut rng);
        albums.truncate(12);
        (top_genre, albums)
    });

    let recently_added = use_memo(move || {
        let lib = library.read();
        let mut unique = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for album in lib.albums.iter().rev() {
            if seen.insert(album.title.trim().to_lowercase()) {
                unique.push(album.clone());
            }
            if unique.len() >= 12 {
                break;
            }
        }
        unique
    });

    let conf_snapshot = config.read();
    let is_modern = conf_snapshot.ui_style == UiStyle::Modern;
    let listen_now_style = conf_snapshot.listen_now_style;
    let sections: Vec<(String, bool)> = conf_snapshot
        .home_sections
        .iter()
        .map(|s| (s.key.clone(), s.enabled))
        .collect();
    drop(conf_snapshot);

    let scroll_container = move |id: &str, direction: i32| {
        let script = format!(
            "document.getElementById('{}').scrollBy({{ left: {}, behavior: 'smooth' }})",
            id,
            direction * 300
        );
        let _ = document::eval(&script);
    };

    let edit = *edit_mode.read();
    let total = sections.len();

    rsx! {
        div {
            for (idx, (key, enabled)) in sections.into_iter().enumerate() {
                {
                    let key_for_render = key.clone();
                    let key_toggle = key.clone();
                    let key_up = key.clone();
                    let key_down = key.clone();
                    if !enabled && !edit {
                        rsx! {}
                    } else {
                        rsx! {
                            div {
                                key: "{key}",
                                class: if !enabled { "opacity-40" } else { "" },
                                if edit {
                                    div { class: "flex items-center justify-between gap-2 mb-2 px-2 py-2 rounded-lg bg-white/5 border border-white/10",
                                        div { class: "flex items-center gap-2 text-white/80 text-xs font-bold uppercase tracking-wider",
                                            i { class: "fa-solid fa-grip-vertical text-white/30" }
                                            span { "{section_label(&key)}" }
                                        }
                                        div { class: "flex items-center gap-1",
                                            if key == "listen_now" {
                                                button {
                                                    class: "px-3 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white text-xs font-semibold transition-colors",
                                                    title: i18n::t("listen_now_layout").to_string(),
                                                    onclick: move |_| {
                                                        let mut conf = config.write();
                                                        conf.listen_now_style = match conf.listen_now_style {
                                                            ListenNowStyle::List => ListenNowStyle::Cards,
                                                            ListenNowStyle::Cards => ListenNowStyle::List,
                                                        };
                                                    },
                                                    i { class: if listen_now_style == ListenNowStyle::Cards { "fa-solid fa-grip-horizontal mr-1" } else { "fa-solid fa-list mr-1" } }
                                                    if listen_now_style == ListenNowStyle::Cards { {i18n::t("layout_cards").to_string()} } else { {i18n::t("layout_list").to_string()} }
                                                }
                                            }
                                            button {
                                                class: "w-7 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white transition-colors",
                                                title: i18n::t("move_up").to_string(),
                                                disabled: idx == 0,
                                                onclick: move |_| {
                                                    let mut conf = config.write();
                                                    if let Some(i) = conf.home_sections.iter().position(|s| s.key == key_up) {
                                                        if i > 0 { conf.home_sections.swap(i, i - 1); }
                                                    }
                                                },
                                                i { class: "fa-solid fa-chevron-up text-xs" }
                                            }
                                            button {
                                                class: "w-7 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/70 hover:text-white transition-colors",
                                                title: i18n::t("move_down").to_string(),
                                                disabled: idx + 1 >= total,
                                                onclick: move |_| {
                                                    let mut conf = config.write();
                                                    if let Some(i) = conf.home_sections.iter().position(|s| s.key == key_down) {
                                                        if i + 1 < conf.home_sections.len() { conf.home_sections.swap(i, i + 1); }
                                                    }
                                                },
                                                i { class: "fa-solid fa-chevron-down text-xs" }
                                            }
                                            button {
                                                class: if enabled {
                                                    "px-3 h-7 rounded-md bg-indigo-500/20 hover:bg-indigo-500/30 text-indigo-300 text-xs font-semibold transition-colors"
                                                } else {
                                                    "px-3 h-7 rounded-md bg-white/5 hover:bg-white/15 text-white/60 text-xs font-semibold transition-colors"
                                                },
                                                onclick: move |_| {
                                                    let mut conf = config.write();
                                                    if let Some(s) = conf.home_sections.iter_mut().find(|s| s.key == key_toggle) {
                                                        s.enabled = !s.enabled;
                                                    }
                                                },
                                                i { class: if enabled { "fa-solid fa-eye mr-1" } else { "fa-solid fa-eye-slash mr-1" } }
                                                if enabled { {i18n::t("hide_section").to_string()} } else { {i18n::t("show_section").to_string()} }
                                            }
                                        }
                                    }
                                }
                                {render_local_section(
                                    &key_for_render,
                                    library,
                                    favorites_store,
                                    config,
                                    edit,
                                    is_modern,
                                    listen_now_style,
                                    local_shuffled(),
                                    continue_listening(),
                                    artists(),
                                    new_release_albums(),
                                    made_for_you(),
                                    recently_added(),
                                    recent_albums(),
                                    recent_playlists(),
                                    on_select_album,
                                    on_play_album,
                                    on_select_playlist,
                                    on_search_artist,
                                    scroll_container,
                                )}
                            }
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_local_section(
    key: &str,
    library: Signal<Library>,
    favorites_store: Signal<FavoritesStore>,
    config: Signal<AppConfig>,
    edit: bool,
    is_modern: bool,
    listen_now_style: ListenNowStyle,
    local_shuffled: Vec<Album>,
    continue_listening: Vec<(Track, Option<Album>)>,
    artists: Vec<(String, Option<PathBuf>)>,
    new_release_albums: Vec<Album>,
    made_for_you: (String, Vec<Album>),
    recently_added: Vec<Album>,
    _recent_albums: Vec<Album>,
    recent_playlists: Vec<(String, String, usize, Option<String>)>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    match key {
        "hero" => rsx! {
            LocalHeroBanner {
                library,
                favorites_store,
                config,
                edit,
                is_modern,
                album: local_shuffled.first().cloned(),
                on_play_album,
            }
        },
        "continue_listening" => render_continue_listening(
            is_modern,
            continue_listening,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "listen_now" => render_listen_now(
            is_modern,
            listen_now_style,
            local_shuffled,
            on_select_album,
            on_play_album,
        ),
        "top_artists" => render_top_artists(is_modern, artists, on_search_artist, scroll_container),
        "new_releases" => render_albums_row(
            "albums-scroll",
            i18n::t("new_releases").to_string(),
            i18n::t("albums").to_string(),
            is_modern,
            new_release_albums,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "made_for_you" => render_made_for_you(
            is_modern,
            made_for_you,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "recently_added" => render_albums_row(
            "recently-added-scroll",
            i18n::t("recently_added").to_string(),
            i18n::t("library").to_string(),
            is_modern,
            recently_added,
            on_select_album,
            on_play_album,
            scroll_container,
        ),
        "playlists" => render_playlists(
            library,
            is_modern,
            recent_playlists,
            on_select_playlist,
            scroll_container,
        ),
        _ => rsx! {},
    }
}

#[component]
fn LocalHeroBanner(
    library: Signal<Library>,
    favorites_store: Signal<FavoritesStore>,
    mut config: Signal<AppConfig>,
    edit: bool,
    is_modern: bool,
    album: Option<Album>,
    on_play_album: EventHandler<String>,
) -> Element {
    let mut is_resizing = use_signal(|| false);
    let mut start_y = use_signal(|| 0.0_f64);
    let mut start_h = use_signal(|| 0_u32);

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
    let section_class = if is_modern {
        "relative rounded-xl overflow-hidden mb-10"
    } else {
        "relative rounded-3xl overflow-hidden mb-12"
    };
    let section_style = format!("height: {hero_height}px;");

    rsx! {
        section { class: "{section_class}", style: "{section_style}",
            if let Some(album) = album {
                div { class: "absolute inset-0",
                    if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref()) {
                        img { src: "{url.as_ref()}&hq=1", class: "w-full h-full object-cover", decoding: "async" }
                    }
                    div { class: "absolute inset-0 bg-gradient-to-r from-black/90 via-black/40 to-transparent" }
                }
                div { class: "relative h-full flex flex-col justify-center p-8 md:p-12",
                    span { class: "text-indigo-400 font-bold tracking-widest uppercase text-[10px] mb-3 flex items-center gap-2",
                        i { class: "fa-solid fa-clock-rotate-left text-[8px]" }
                        "{i18n::t(\"jump_back_in\")}"
                    }
                    h1 { class: "text-3xl md:text-5xl font-black text-white mb-4 leading-tight max-w-xl break-words", "{album.title}" }
                    p { class: "text-base md:text-lg text-white/60 mb-8 font-medium line-clamp-1 max-w-md", "{i18n::t_with(\"by_artist_full\", &[(\"artist\", album.artist.to_string())])}" }
                    div { class: "flex items-center gap-4",
                        button {
                            class: "flex items-center gap-3 bg-white text-black px-8 py-3 rounded-full font-bold hover:bg-white/90 hover:scale-105 active:scale-95 transition-all w-fit",
                            onclick: {
                                let id = album.id.clone();
                                move |_| on_play_album.call(id.clone())
                            },
                            i { class: "fa-solid fa-play text-[10px]" }
                            span { class: "text-sm", "{i18n::t(\"start_listening\")}" }
                        }
                        {
                            let local_hero_album_id = album.id.clone();
                            let local_hero_fav = {
                                let lib = library.read();
                                let store = favorites_store.read();
                                let tracks: Vec<_> = lib.tracks.iter()
                                    .filter(|t| t.album_id == album.id)
                                    .collect();
                                !tracks.is_empty() && tracks.iter().all(|t| store.is_local_favorite(&t.path))
                            };
                            let heart_class = if local_hero_fav {
                                "w-11 h-11 rounded-full bg-white/10 border border-white/20 flex items-center justify-center text-red-400 hover:bg-white/20 transition-all"
                            } else {
                                "w-11 h-11 rounded-full bg-white/10 border border-white/20 flex items-center justify-center text-white hover:bg-white/20 transition-all"
                            };
                            let heart_icon = if local_hero_fav { "fa-solid fa-heart" } else { "fa-regular fa-heart" };
                            let mut favorites_store = favorites_store;
                            rsx! {
                                button {
                                    class: "{heart_class}",
                                    onclick: move |_| {
                                        let lib = library.read();
                                        let tracks: Vec<_> = lib.tracks.iter()
                                            .filter(|t| t.album_id == local_hero_album_id)
                                            .cloned()
                                            .collect();
                                        drop(lib);
                                        let new_fav = !local_hero_fav;
                                        for track in tracks {
                                            let currently = favorites_store.read().is_local_favorite(&track.path);
                                            if new_fav && !currently {
                                                favorites_store.write().toggle_local(track.path);
                                            } else if !new_fav && currently {
                                                favorites_store.write().toggle_local(track.path);
                                            }
                                        }
                                    },
                                    i { class: "{heart_icon}" }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "absolute inset-0 bg-gradient-to-br from-indigo-900/40 to-purple-900/40 flex items-center justify-center",
                    div { class: "text-center",
                        i { class: "fa-solid fa-music text-6xl text-white/10 mb-4" }
                        h2 { class: "text-2xl font-bold text-white/40", "{i18n::t(\"add_music_to_get_started\")}" }
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

fn render_continue_listening(
    is_modern: bool,
    tracks: Vec<(Track, Option<Album>)>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if tracks.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_modern { "mb-10" } else { "cv-section mb-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"library\")}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-2xl font-bold text-white tracking-tight" }, "{i18n::t(\"continue_listening\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container("continue-listening-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container("continue-listening-scroll", 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "continue-listening-scroll",
                class: "flex overflow-x-auto gap-5 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                for (track, album_opt) in tracks {
                    {
                        let cover_path = album_opt.as_ref().and_then(|a| a.cover_path.clone());
                        let album_title = album_opt
                            .as_ref()
                            .map(|a| a.title.clone())
                            .unwrap_or_else(|| track.album.clone());
                        let album_id_opt = album_opt.as_ref().map(|a| a.id.clone());
                        let title = track.title.clone();
                        let artist = track.artist.clone();
                        let key = track.path.to_string_lossy().to_string();
                        let album_id_click = album_id_opt.clone();
                        let album_id_play = album_id_opt.clone();
                        rsx! {
                            div {
                                key: "{key}",
                                class: "flex-none w-44 group cursor-pointer",
                                onclick: move |_| {
                                    if let Some(id) = album_id_click.clone() {
                                        on_select_album.call(id);
                                    }
                                },
                                div { class: "aspect-square rounded-xl bg-stone-800 mb-3 overflow-hidden relative gpu-hover",
                                    if let Some(url) = utils::format_artwork_url(cover_path.as_ref()) {
                                        img { src: "{url.as_ref()}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                                    } else {
                                        div { class: "w-full h-full flex items-center justify-center",
                                            i { class: "fa-solid fa-music text-3xl text-white/20" }
                                        }
                                    }
                                    div {
                                        class: "absolute right-2 bottom-2 w-10 h-10 rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all translate-y-2 group-hover:translate-y-0",
                                        style: "background: var(--color-indigo-500);",
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            if let Some(id) = album_id_play.clone() {
                                                on_play_album.call(id);
                                            }
                                        },
                                        i { class: "fa-solid fa-play text-white text-xs ml-0.5" }
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
    is_modern: bool,
    listen_now_style: ListenNowStyle,
    local_shuffled: Vec<Album>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
) -> Element {
    if local_shuffled.is_empty() {
        return rsx! { div {} };
    }
    let use_cards = listen_now_style == ListenNowStyle::Cards;
    rsx! {
        section { class: if is_modern { "mb-10" } else { "cv-section mb-12" },
            div { class: "flex items-end justify-between mb-6 text-white",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"music\")}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-3xl font-extrabold tracking-tight" }, "{i18n::t(\"listen_now\")}" }
                }
            }
            if use_cards {
                div { class: "flex overflow-x-auto gap-4 pb-4 scrollbar-hide scroll-smooth -mx-2 px-2",
                    for album in local_shuffled.iter().skip(1).take(10).cloned() {
                        div {
                            class: "flex-none w-40 group cursor-pointer",
                            onclick: {
                                let id = album.id.clone();
                                move |_| on_select_album.call(id.clone())
                            },
                            div { class: "aspect-square rounded-lg bg-stone-800 mb-2 overflow-hidden relative",
                                if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref()) {
                                    img { src: "{url.as_ref()}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                                } else {
                                    div { class: "w-full h-full flex items-center justify-center",
                                        i { class: "fa-solid fa-compact-disc text-2xl text-white/20" }
                                    }
                                }
                                div {
                                    class: "absolute right-2 bottom-2 w-9 h-9 rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-all translate-y-2 group-hover:translate-y-0",
                                    style: "background: var(--color-indigo-500);",
                                    onclick: {
                                        let id = album.id.clone();
                                        move |evt| { evt.stop_propagation(); on_play_album.call(id.clone()); }
                                    },
                                    i { class: "fa-solid fa-play text-white text-xs ml-0.5" }
                                }
                            }
                            h3 { class: "text-white font-semibold truncate text-sm", "{album.title}" }
                            p { class: "text-xs truncate mt-0.5", style: "color: rgba(255,255,255,0.45);", "{album.artist}" }
                        }
                    }
                }
            } else {
                div { class: "grid grid-cols-[repeat(auto-fill,minmax(350px,1fr))] gap-4",
                    for album in local_shuffled.iter().skip(1).take(8).cloned() {
                        div {
                            class: "flex items-center bg-white/5 hover:bg-white/10 border border-white/5 rounded-2xl cursor-pointer transition-all duration-300 group overflow-hidden pr-4",
                            onclick: {
                                let id = album.id.clone();
                                move |_| on_select_album.call(id.clone())
                            },
                            div { class: "w-16 h-16 md:w-20 md:h-20 flex-shrink-0 bg-stone-800/50 relative overflow-hidden gpu-hover",
                                if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref()) {
                                    img { src: "{url.as_ref()}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                                } else {
                                    div { class: "w-full h-full flex items-center justify-center",
                                        i { class: "fa-solid fa-compact-disc text-xl text-white/20" }
                                    }
                                }
                                div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                            }
                            div { class: "p-4 flex-1 min-w-0 flex flex-col justify-center",
                                h3 { class: "text-white font-bold truncate text-sm md:text-base", "{album.title}" }
                                p { class: "text-xs text-white/50 truncate font-semibold mt-1", "{album.artist}" }
                            }
                            div { class: "opacity-0 group-hover:opacity-100 transition-all duration-300",
                                div {
                                    class: "w-8 h-8 rounded-full bg-white/10 flex items-center justify-center hover:bg-white/20 transition-colors",
                                    onclick: {
                                        let id = album.id.clone();
                                        move |evt| {
                                            evt.stop_propagation();
                                            on_play_album.call(id.clone());
                                        }
                                    },
                                    i { class: "fa-solid fa-play text-white/80 text-xs" }
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
    is_modern: bool,
    artists: Vec<(String, Option<PathBuf>)>,
    on_search_artist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if artists.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_modern { "mt-10" } else { "cv-section mt-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"artists\")}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-2xl font-bold text-white tracking-tight" }, "{i18n::t(\"top_artists\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container("artists-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container("artists-scroll", 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "artists-scroll",
                class: "flex overflow-x-auto gap-6 pb-6 pt-2 overflow-y-visible scrollbar-hide scroll-smooth -mx-2 px-2",
                for (artist, cover_path) in artists {
                    div {
                        class: "flex-none w-32 md:w-36 group cursor-pointer",
                        onclick: {
                            let artist = artist.clone();
                            move |_| on_search_artist.call(artist.clone())
                        },
                        div { class: "aspect-square rounded-full bg-stone-800/80 mb-4 overflow-hidden transition-all duration-300 relative mx-auto gpu-hover",
                            if let Some(path) = cover_path {
                                if let Some(url) = utils::format_artwork_thumb_url(Some(&path), 320) {
                                    img { src: "{url.as_ref()}", class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-700", decoding: "async", loading: "lazy" }
                                }
                            } else {
                                div { class: "w-full h-full flex items-center justify-center border border-white/5 rounded-full",
                                    i { class: "fa-solid fa-microphone text-3xl text-white/20" }
                                }
                            }
                            div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/10 transition-colors duration-300 rounded-full" }
                        }
                        h3 { class: "text-white font-bold truncate text-center px-1 text-sm md:text-base", "{artist}" }
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
    is_modern: bool,
    albums: Vec<Album>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if albums.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_modern { "mt-10" } else { "cv-section mt-12" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{eyebrow}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-2xl font-bold text-white tracking-tight" }, "{title}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container(scroll_id, -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container(scroll_id, 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "{scroll_id}",
                class: "flex overflow-x-auto gap-5 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                for album in albums {
                    div {
                        class: "flex-none w-36 md:w-44 group cursor-pointer",
                        onclick: {
                            let id = album.id.clone();
                            move |_| on_select_album.call(id.clone())
                        },
                        div { class: "aspect-square rounded-2xl bg-stone-800/80 mb-4 overflow-hidden transition-all duration-300 relative gpu-hover",
                            if let Some(url) = utils::format_artwork_url(album.cover_path.as_ref()) {
                                img { src: "{url.as_ref()}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500", decoding: "async", loading: "lazy" }
                            } else {
                                div { class: "w-full h-full flex items-center justify-center border border-white/5 rounded-2xl",
                                    i { class: "fa-solid fa-compact-disc text-3xl text-white/20" }
                                }
                            }
                            div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                            div {
                                class: "absolute right-3 bottom-3 w-10 h-10 bg-white text-black rounded-full flex items-center justify-center translate-y-4 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300",
                                onclick: {
                                    let id = album.id.clone();
                                    move |evt| {
                                        evt.stop_propagation();
                                        on_play_album.call(id.clone());
                                    }
                                },
                                i { class: "fa-solid fa-play text-xs ml-0.5" }
                            }
                        }
                        h3 { class: "text-white font-bold truncate text-sm md:text-base px-1", "{album.title}" }
                        p { class: "text-xs md:text-sm text-white/50 truncate px-1 font-semibold mt-1", "{album.artist}" }
                    }
                }
            }
        }
    }
}

fn render_made_for_you(
    is_modern: bool,
    made_for_you: (String, Vec<Album>),
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    let (genre, albums) = made_for_you;
    if albums.is_empty() {
        return rsx! { div {} };
    }
    let eyebrow = if genre.is_empty() {
        i18n::t("music").to_string()
    } else {
        genre
    };
    render_albums_row(
        "made-for-you-scroll",
        i18n::t("made_for_you").to_string(),
        eyebrow,
        is_modern,
        albums,
        on_select_album,
        on_play_album,
        scroll_container,
    )
}

fn render_playlists(
    library: Signal<Library>,
    is_modern: bool,
    recent_playlists: Vec<(String, String, usize, Option<String>)>,
    on_select_playlist: EventHandler<String>,
    scroll_container: impl Fn(&str, i32) + Copy + 'static,
) -> Element {
    if recent_playlists.is_empty() {
        return rsx! { div {} };
    }
    rsx! {
        section { class: if is_modern { "mt-10" } else { "cv-section mt-16" },
            div { class: "flex items-center justify-between mb-6",
                div {
                    if is_modern {
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5", style: "color: rgba(255,255,255,0.35);", "{i18n::t(\"library\")}" }
                    }
                    h2 { class: if is_modern { "text-2xl font-bold text-white" } else { "text-2xl font-bold text-white tracking-tight" }, "{i18n::t(\"playlists\")}" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container("playlists-scroll", -1),
                        i { class: "fa-solid fa-chevron-left text-sm" }
                    }
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all",
                        onclick: move |_| scroll_container("playlists-scroll", 1),
                        i { class: "fa-solid fa-chevron-right text-sm" }
                    }
                }
            }
            div {
                id: "playlists-scroll",
                class: "flex overflow-x-auto gap-6 pb-6 pt-2 scrollbar-hide scroll-smooth -mx-2 px-2",
                for (id, name, track_count, first_track) in recent_playlists {
                    {
                        let track_count_text = if track_count == 1 {
                            i18n::t("track_count_singular").to_string()
                        } else {
                            i18n::t_with("track_count", &[("count", track_count.to_string())])
                        };
                        let cover_url = if let Some(track_path) = first_track {
                            let lib = library.peek();
                            lib.tracks
                                .iter()
                                .find(|t| t.path.to_string_lossy() == track_path)
                                .and_then(|t| {
                                    lib.albums
                                        .iter()
                                        .find(|a| a.id == t.album_id)
                                        .and_then(|a| a.cover_path.as_ref())
                                        .and_then(|cp| utils::format_artwork_url(Some(cp)))
                                })
                        } else {
                            None
                        };
                        rsx! {
                            div {
                                key: "{id}",
                                class: "flex-none w-40 md:w-48 group cursor-pointer",
                                onclick: {
                                    let id = id.clone();
                                    move |_| on_select_playlist.call(id.clone())
                                },
                                div { class: "aspect-square rounded-2xl bg-white/5 mb-4 overflow-hidden transition-all duration-500 relative gpu-hover",
                                    if let Some(url) = cover_url {
                                        img { src: "{url.as_ref()}", class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-700", decoding: "async", loading: "lazy" }
                                    } else {
                                        div { class: "w-full h-full flex items-center justify-center bg-gradient-to-br from-indigo-600/20 to-purple-600/20 group-hover:scale-110 transition-transform duration-700",
                                            i { class: "fa-solid fa-music text-5xl opacity-40 text-white" }
                                        }
                                    }
                                    div { class: "absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors duration-300" }
                                }
                                div {
                                    h3 { class: "text-white font-bold truncate text-sm md:text-base px-1 group-hover:text-indigo-400 transition-colors", "{name}" }
                                    p { class: "text-xs md:text-sm text-white/40 truncate px-1 font-semibold mt-1", "{track_count_text}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
