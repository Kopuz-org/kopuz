use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use reader::models::{Library, Track};
use server::ytmusic::discover::{DiscoverHome, DiscoverItem, DiscoverShelf, YtArtist};

#[component]
pub fn DiscoverPage(
    library: Signal<Library>,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(String, String)>,
    on_open_artist: EventHandler<(String, String)>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let mut shelves = use_signal(Vec::<DiscoverShelf>::new);
    let mut continuation = use_signal(|| None::<String>);
    let mut loading_more = use_signal(|| false);
    let mut initial_loading = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);

    let is_ytmusic = config
        .read()
        .server
        .as_ref()
        .map(|s| s.service == MusicService::YtMusic)
        .unwrap_or(false);

    use_effect(move || {
        if !is_ytmusic {
            initial_loading.set(false);
            return;
        }
        if !shelves.peek().is_empty() {
            return;
        }
        spawn(async move {
            let token = config
                .peek()
                .server
                .as_ref()
                .and_then(|s| s.access_token.clone());
            let Some(token) = token else {
                error.set(Some("not signed in".to_string()));
                initial_loading.set(false);
                return;
            };
            let yt = ::server::ytmusic::YouTubeMusicClient::with_cookies(token);
            match yt.discover_home().await {
                Ok(home) => {
                    apply_home(home, &mut shelves, &mut continuation);
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
            initial_loading.set(false);
        });
    });

    if !is_ytmusic {
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
        spawn(async move {
            let cookies = config
                .peek()
                .server
                .as_ref()
                .and_then(|s| s.access_token.clone());
            if let Some(cookies) = cookies {
                let yt = ::server::ytmusic::YouTubeMusicClient::with_cookies(cookies);
                match yt.discover_continuation(&token).await {
                    Ok(home) => apply_home(home, &mut shelves, &mut continuation),
                    Err(e) => error.set(Some(e)),
                }
            }
            loading_more.set(false);
        });
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
                div { class: "py-12 text-rose-400 text-sm",
                    "{i18n::t_with(\"discover_failed\", &[(\"error\", err.clone())])}"
                }
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

fn apply_home(
    home: DiscoverHome,
    shelves: &mut Signal<Vec<DiscoverShelf>>,
    continuation: &mut Signal<Option<String>>,
) {
    shelves.write().extend(home.shelves);
    continuation.set(home.continuation);
}

#[component]
fn ShelfRow(
    shelf: DiscoverShelf,
    scroll_id: String,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(String, String)>,
    on_open_artist: EventHandler<(String, String)>,
    on_search_artist: EventHandler<String>,
) -> Element {
    if shelf.is_song_list {
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
                        p { class: "text-[10px] font-bold tracking-widest uppercase mb-0.5 text-white/40", "{strap}" }
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

/// Vertical song-list shelf for the artist page "Top songs" section.
/// YT only returns the first 5 rows inline and ships a `more_browse_id`
/// (a `VL…` playlist id) that points at the full songs playlist; we
/// expose that as a "Show all songs" button which navigates through
/// `on_select_playlist` into the existing `DiscoverPlaylistDetail`
/// viewer (which already paginates).
#[component]
fn SongListShelf(
    shelf: DiscoverShelf,
    on_select_playlist: EventHandler<(String, String)>,
) -> Element {
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let tracks: Vec<Track> = shelf
        .items
        .iter()
        .filter_map(|i| match i {
            DiscoverItem::Song(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    let title_for_more = shelf.title.clone();
    let more = shelf.more_browse_id.clone();
    rsx! {
        section { class: "mb-12",
            div { class: "flex items-end justify-between mb-5 gap-4",
                h2 { class: "text-2xl md:text-3xl font-bold text-white truncate", "{shelf.title}" }
                if let Some(more) = more {
                    button {
                        class: "text-xs font-bold tracking-widest uppercase text-white/60 hover:text-white cursor-pointer transition-colors",
                        onclick: move |_| {
                            on_select_playlist.call((more.clone(), title_for_more.clone()))
                        },
                        "{i18n::t(\"discover_show_all\")}"
                    }
                }
            }
            div { class: "flex flex-col",
                for (idx, track) in tracks.iter().enumerate() {
                    DiscoverPlaylistRow {
                        key: "{idx}",
                        track: track.clone(),
                        index: idx + 1,
                        on_play: {
                            let tracks_for_row = tracks.clone();
                            move |t: Track| {
                                let mut queue = tracks_for_row.clone();
                                let start = queue.iter().position(|x| x.path == t.path).unwrap_or(0);
                                queue.rotate_left(start);
                                ctrl.play_queue_linear(queue);
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn DiscoverTile(
    item: DiscoverItem,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(String, String)>,
    on_open_artist: EventHandler<(String, String)>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let config = use_context::<Signal<AppConfig>>();
    match item {
        DiscoverItem::Song(track) => {
            let mut ctrl_for_song = ctrl;
            rsx! {
                SongCard {
                    track: track.clone(),
                    on_play: move |t: Track| ctrl_for_song.play_queue_linear(vec![t]),
                }
            }
        },
        DiscoverItem::Playlist { playlist_id, title, subtitle, thumbnail } => {
            let title_for_click = title.clone();
            let pid_for_play = playlist_id.clone();
            rsx! {
                Card {
                    title: title,
                    subtitle: subtitle,
                    thumbnail: thumbnail,
                    rounded_full: false,
                    onclick: move |_| {
                        on_select_playlist.call((playlist_id.clone(), title_for_click.clone()))
                    },
                    on_play: EventHandler::new(move |_| {
                        play_playlist_async(pid_for_play.clone(), config, ctrl);
                    }),
                }
            }
        },
        DiscoverItem::Album { browse_id, title, subtitle, thumbnail } => {
            let title_for_click = title.clone();
            let bid_for_play = browse_id.clone();
            rsx! {
                Card {
                    title: title,
                    subtitle: subtitle,
                    thumbnail: thumbnail,
                    rounded_full: false,
                    onclick: move |_| {
                        on_select_playlist.call((browse_id.clone(), title_for_click.clone()))
                    },
                    on_play: EventHandler::new(move |_| {
                        play_playlist_async(bid_for_play.clone(), config, ctrl);
                    }),
                }
            }
        },
        DiscoverItem::Artist { channel_id, name, thumbnail } => {
            let cid = channel_id.clone();
            let name_for_click = name.clone();
            rsx! {
                Card {
                    title: name.clone(),
                    subtitle: String::new(),
                    thumbnail: thumbnail,
                    rounded_full: true,
                    onclick: move |_| on_open_artist.call((cid.clone(), name_for_click.clone())),
                    on_play: None,
                }
            }
        },
        DiscoverItem::Mood { title, thumbnail, .. } => rsx! {
            Card {
                title: title,
                subtitle: String::new(),
                thumbnail: thumbnail,
                rounded_full: false,
                onclick: move |_| {},
                on_play: None,
            }
        },
    }
}

/// Shared "play whatever this id resolves to" used by both Playlist
/// and Album tiles. MPRE… ids go through the album browse endpoint,
/// everything else through the playlist entries endpoint. The fetch
/// happens off the click — slight delay before audio starts but no
/// blocking the UI.
fn play_playlist_async(
    id: String,
    config: Signal<AppConfig>,
    mut ctrl: hooks::use_player_controller::PlayerController,
) {
    spawn(async move {
        let Some(cookies) = config
            .peek()
            .server
            .as_ref()
            .and_then(|s| s.access_token.clone())
        else {
            return;
        };
        let yt = ::server::ytmusic::YouTubeMusicClient::with_cookies(cookies);
        let tracks = if id.starts_with("MPRE") {
            yt.fetch_album_tracks(&id).await
        } else {
            yt.get_playlist_entries(&id).await
        };
        if let Ok(tracks) = tracks
            && !tracks.is_empty()
        {
            ctrl.play_queue_linear(tracks);
        }
    });
}

#[component]
fn Card(
    title: String,
    subtitle: String,
    thumbnail: Option<String>,
    rounded_full: bool,
    onclick: EventHandler<MouseEvent>,
    on_play: Option<EventHandler<()>>,
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
    let cover_radius = if rounded_full { "rounded-full" } else { "rounded-lg" };
    rsx! {
        div {
            class: "shrink-0 w-44 text-left cursor-pointer transition-transform duration-200 ease-out hover:scale-[1.03] hover:-translate-y-0.5 group",
            onclick: move |e| onclick.call(e),
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
                        class: "absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 bg-black/40 transition-opacity duration-200 cursor-pointer",
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            play.call(());
                        },
                        i { class: "fa-solid fa-play text-white text-2xl" }
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

#[component]
fn SongCard(track: Track, on_play: EventHandler<Track>) -> Element {
    let thumbnail = utils::jellyfin_image::track_cover_url_with_album_fallback(
        &track.path.to_string_lossy(),
        &track.album_id,
        "",
        None,
        320,
        80,
    );
    let title = track.title.clone();
    let artist = track.artist.clone();
    rsx! {
        button {
            class: "shrink-0 w-44 text-left cursor-pointer transition-transform duration-200 ease-out hover:scale-[1.03] hover:-translate-y-0.5 group",
            onclick: {
                let track = track.clone();
                move |_| on_play.call(track.clone())
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
                    i { class: "fa-solid fa-play text-white text-2xl" }
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
                "{artist}"
            }
        }
    }
}

/// Standalone viewer for a YT Music playlist discovered from the home
/// feed. Tracks are pulled directly from YT via get_playlist_entries
/// (which handles continuationItemRenderer pagination internally) —
/// nothing about this view touches `playlist_store`, so a discover
/// playlist never pollutes the user's saved Library Playlists.
#[component]
pub fn DiscoverPlaylistDetail(
    selected_playlist_id: Signal<Option<String>>,
    selected_playlist_title: Signal<Option<String>>,
    on_back: EventHandler<()>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let mut tracks = use_signal(Vec::<Track>::new);
    let mut loading = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);

    let playlist_id = selected_playlist_id.read().clone();
    let header_title = selected_playlist_title
        .read()
        .clone()
        .unwrap_or_else(String::new);

    use_effect(move || {
        let Some(pid) = selected_playlist_id.read().clone() else {
            return;
        };
        tracks.set(Vec::new());
        loading.set(true);
        error.set(None);
        spawn(async move {
            let cookies = config
                .peek()
                .server
                .as_ref()
                .and_then(|s| s.access_token.clone());
            let Some(cookies) = cookies else {
                error.set(Some("not signed in".to_string()));
                loading.set(false);
                return;
            };
            let yt = ::server::ytmusic::YouTubeMusicClient::with_cookies(cookies);
            // Discover routes both playlists and albums through this viewer;
            // MPRE… ids are albums and need the browse-album endpoint instead.
            let result = if pid.starts_with("MPRE") {
                yt.fetch_album_tracks(&pid).await
            } else {
                yt.get_playlist_entries(&pid).await
            };
            match result {
                Ok(ts) => tracks.set(ts),
                Err(e) => error.set(Some(e)),
            }
            loading.set(false);
        });
    });

    if playlist_id.is_none() {
        return rsx! {
            div { class: "flex items-center justify-center h-full text-white/60 p-12",
                p { "{i18n::t(\"playlist_not_found\")}" }
            }
        };
    }

    rsx! {
        div { class: "p-6 md:p-10 max-w-[1600px] mx-auto",
            button {
                class: "inline-flex items-center gap-2 text-white/70 hover:text-white text-sm cursor-pointer mb-6 group",
                onclick: move |_| on_back.call(()),
                i { class: "fa-solid fa-chevron-left text-xs transition-transform group-hover:-translate-x-0.5" }
                span { "{i18n::t(\"back\")}" }
            }
            div { class: "flex items-end gap-6 mb-8",
                div { class: "min-w-0",
                    p { class: "text-[10px] font-bold tracking-widest uppercase text-white/40 mb-2", "{i18n::t(\"playlist\")}" }
                    h1 { class: "text-3xl md:text-5xl font-black text-white break-words", "{header_title}" }
                    if !*loading.read() {
                        p { class: "text-sm text-white/50 mt-3",
                            "{i18n::t_with(\"playlist_track_count\", &[(\"count\", tracks.read().len().to_string())])}"
                        }
                    }
                }
                button {
                    class: "shrink-0 inline-flex items-center gap-3 bg-white text-black px-8 py-3 rounded-full font-bold hover:bg-white/90 hover:scale-105 active:scale-95 transition-all cursor-pointer disabled:opacity-40 disabled:cursor-default",
                    disabled: *loading.read() || tracks.read().is_empty(),
                    onclick: move |_| {
                        let all = tracks.read().clone();
                        if !all.is_empty() {
                            ctrl.play_queue_linear(all);
                        }
                    },
                    i { class: "fa-solid fa-play text-[10px]" }
                    span { class: "text-sm", "{i18n::t(\"start_listening\")}" }
                }
            }

            if *loading.read() {
                div { class: "flex justify-center py-24",
                    i { class: "fa-solid fa-arrows-rotate fa-spin text-2xl text-white/60" }
                }
            } else if let Some(err) = error.read().clone() {
                div { class: "py-12 text-rose-400 text-sm",
                    "{i18n::t_with(\"discover_failed\", &[(\"error\", err.clone())])}"
                }
            } else {
                div { class: "flex flex-col",
                    for (idx, track) in tracks.read().iter().enumerate() {
                        DiscoverPlaylistRow {
                            key: "{idx}",
                            track: track.clone(),
                            index: idx + 1,
                            on_play: move |t: Track| {
                                let mut queue = tracks.read().clone();
                                let start_idx = queue
                                    .iter()
                                    .position(|x| x.path == t.path)
                                    .unwrap_or(0);
                                queue.rotate_left(start_idx);
                                ctrl.play_queue_linear(queue);
                            },
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DiscoverPlaylistRow(track: Track, index: usize, on_play: EventHandler<Track>) -> Element {
    let thumbnail = utils::jellyfin_image::track_cover_url_with_album_fallback(
        &track.path.to_string_lossy(),
        &track.album_id,
        "",
        None,
        96,
        80,
    );
    let title = track.title.clone();
    let artist = track.artist.clone();
    let track_for_click = track.clone();
    rsx! {
        button {
            class: "group flex items-center gap-4 px-3 py-2 rounded-lg hover:bg-white/5 transition-colors text-left cursor-pointer w-full",
            onclick: move |_| on_play.call(track_for_click.clone()),
            span { class: "w-8 text-right text-white/40 text-xs tabular-nums group-hover:hidden", "{index}" }
            i { class: "w-8 text-center fa-solid fa-play text-white text-xs hidden group-hover:inline-block" }
            if let Some(url) = thumbnail {
                img {
                    src: "{url}",
                    class: "w-11 h-11 object-cover rounded bg-white/5",
                    loading: "lazy",
                    decoding: "async",
                }
            } else {
                div { class: "w-11 h-11 rounded bg-white/5" }
            }
            div { class: "min-w-0 flex-1",
                p { class: "text-sm text-white font-medium truncate", "{title}" }
                p { class: "text-xs text-white/50 truncate", "{artist}" }
            }
        }
    }
}

/// YT-backed artist profile. Pulls the immersive header (banner +
/// subscribers) and every section shelf from `/browse?browseId=UC…` and
/// hands each one off to the same `ShelfRow` component the Discover home
/// uses, so all sections get hover-play, horizontal scroll, and the
/// existing tile dispatch (artist → artist, album → album viewer, etc.).
#[component]
pub fn DiscoverArtistPage(
    selected_artist_id: Signal<Option<String>>,
    selected_artist_name: Signal<Option<String>>,
    on_back: EventHandler<()>,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<(String, String)>,
    on_open_artist: EventHandler<(String, String)>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let mut artist = use_signal(|| None::<YtArtist>);
    let mut loading = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);

    use_effect(move || {
        let Some(cid) = selected_artist_id.read().clone() else {
            return;
        };
        artist.set(None);
        loading.set(true);
        error.set(None);
        spawn(async move {
            let cookies = config
                .peek()
                .server
                .as_ref()
                .and_then(|s| s.access_token.clone());
            let Some(cookies) = cookies else {
                error.set(Some("not signed in".to_string()));
                loading.set(false);
                return;
            };
            let yt = ::server::ytmusic::YouTubeMusicClient::with_cookies(cookies);
            match yt.fetch_artist(&cid).await {
                Ok(a) => artist.set(Some(a)),
                Err(e) => error.set(Some(e)),
            }
            loading.set(false);
        });
    });

    if selected_artist_id.read().is_none() {
        let fallback_name = selected_artist_name.read().clone().unwrap_or_default();
        return rsx! {
            div { class: "p-12 text-white/60", "No artist selected: {fallback_name}" }
        };
    }

    rsx! {
        div { class: "max-w-[1600px] mx-auto",
            button {
                class: "inline-flex items-center gap-2 text-white/70 hover:text-white text-sm cursor-pointer mt-6 ml-6 md:ml-10 mb-2 group",
                onclick: move |_| on_back.call(()),
                i { class: "fa-solid fa-chevron-left text-xs transition-transform group-hover:-translate-x-0.5" }
                span { "{i18n::t(\"back\")}" }
            }

            if *loading.read() {
                div { class: "flex justify-center py-24",
                    i { class: "fa-solid fa-arrows-rotate fa-spin text-2xl text-white/60" }
                }
            } else if let Some(err) = error.read().clone() {
                div { class: "py-12 px-6 md:px-10 text-rose-400 text-sm",
                    "{i18n::t_with(\"discover_failed\", &[(\"error\", err.clone())])}"
                }
            } else if let Some(a) = artist.read().clone() {
                {
                    let banner = a.banner_thumbnail.clone();
                    let banner_style = banner
                        .map(|u| format!("background-image: linear-gradient(to bottom, rgba(0,0,0,0.2) 0%, rgba(0,0,0,0.95) 100%), url('{u}'); background-size: cover; background-position: center;"))
                        .unwrap_or_default();
                    let shuffle_pid = a.shuffle_playlist_id.clone();
                    rsx! {
                        div {
                            class: "relative overflow-hidden",
                            style: "{banner_style}",
                            div { class: "px-6 md:px-10 pt-16 pb-10 flex flex-col gap-4",
                                h1 { class: "text-4xl md:text-6xl font-black text-white break-words drop-shadow-lg", "{a.name}" }
                                if let Some(s) = a.subscribers.clone() {
                                    p { class: "text-sm text-white/70", "{s}" }
                                }
                                if let Some(d) = a.description.clone() {
                                    p { class: "text-sm text-white/60 max-w-3xl line-clamp-3", "{d}" }
                                }
                                div { class: "flex gap-3 mt-2",
                                    if let Some(pid) = shuffle_pid {
                                        button {
                                            class: "inline-flex items-center gap-2 bg-white text-black px-6 py-2.5 rounded-full font-bold hover:scale-105 active:scale-95 transition-transform cursor-pointer",
                                            onclick: move |_| {
                                                play_playlist_async(pid.clone(), config, ctrl);
                                            },
                                            i { class: "fa-solid fa-shuffle text-[11px]" }
                                            span { class: "text-sm", "{i18n::t(\"shuffle\")}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "px-6 md:px-10 pt-8",
                            for (idx, shelf) in a.sections.iter().enumerate() {
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

