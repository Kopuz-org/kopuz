use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use reader::models::{Library, Track};
use server::ytmusic::discover::{DiscoverHome, DiscoverItem, DiscoverShelf};

#[component]
pub fn DiscoverPage(
    library: Signal<Library>,
    on_select_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let mut config = use_context::<Signal<AppConfig>>();
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
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
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
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105",
                        onclick: move |_| {
                            let _ = document::eval(&format!(
                                "document.getElementById('{}').scrollBy({{ left: -800, behavior: 'smooth' }})",
                                scroll_left
                            ));
                        },
                        i { class: "fa-solid fa-chevron-left text-xs" }
                    }
                    button {
                        class: "w-8 h-8 rounded-full bg-white/5 hover:bg-white/10 flex items-center justify-center text-white transition-all hover:scale-105",
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
                class: "flex overflow-x-auto gap-5 pb-3 pt-1 scrollbar-hide scroll-smooth -mx-2 px-2",
                for (idx, item) in shelf.items.iter().enumerate() {
                    DiscoverTile {
                        key: "{idx}",
                        item: item.clone(),
                        on_select_album: on_select_album,
                        on_select_playlist: on_select_playlist,
                        on_search_artist: on_search_artist,
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
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    match item {
        DiscoverItem::Song(track) => rsx! {
            SongCard { track: track.clone(), on_play: move |t: Track| ctrl.play_queue_linear(vec![t]) }
        },
        DiscoverItem::Playlist { playlist_id, title, subtitle, thumbnail } => rsx! {
            Card {
                title: title,
                subtitle: subtitle,
                thumbnail: thumbnail,
                rounded_full: false,
                onclick: move |_| on_select_playlist.call(playlist_id.clone()),
            }
        },
        DiscoverItem::Album { browse_id, title, subtitle, thumbnail } => rsx! {
            Card {
                title: title,
                subtitle: subtitle,
                thumbnail: thumbnail,
                rounded_full: false,
                onclick: move |_| on_select_album.call(browse_id.clone()),
            }
        },
        DiscoverItem::Artist { name, thumbnail, .. } => rsx! {
            Card {
                title: name.clone(),
                subtitle: String::new(),
                thumbnail: thumbnail,
                rounded_full: true,
                onclick: move |_| on_search_artist.call(name.clone()),
            }
        },
        DiscoverItem::Mood { title, thumbnail, .. } => rsx! {
            Card {
                title: title,
                subtitle: String::new(),
                thumbnail: thumbnail,
                rounded_full: false,
                onclick: move |_| {},
            }
        },
    }
}

#[component]
fn Card(
    title: String,
    subtitle: String,
    thumbnail: Option<String>,
    rounded_full: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let img_class = if rounded_full {
        "w-44 h-44 object-cover rounded-full"
    } else {
        "w-44 h-44 object-cover rounded-lg"
    };
    rsx! {
        button {
            class: "shrink-0 w-44 text-left group",
            onclick: move |e| onclick.call(e),
            div { class: "relative w-44 h-44 mb-3",
                if let Some(url) = thumbnail {
                    img {
                        src: "{url}",
                        class: "{img_class} bg-white/5",
                        loading: "lazy",
                        decoding: "async",
                    }
                } else {
                    div { class: "{img_class} bg-white/5" }
                }
            }
            p { class: "text-sm font-semibold text-white line-clamp-2 break-words group-hover:text-indigo-200 transition-colors", "{title}" }
            if !subtitle.is_empty() {
                p { class: "text-xs text-white/50 line-clamp-2 mt-1 break-words", "{subtitle}" }
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
            class: "shrink-0 w-44 text-left group",
            onclick: {
                let track = track.clone();
                move |_| on_play.call(track.clone())
            },
            div { class: "relative w-44 h-44 mb-3",
                if let Some(url) = thumbnail {
                    img {
                        src: "{url}",
                        class: "w-44 h-44 object-cover rounded-lg bg-white/5",
                        loading: "lazy",
                        decoding: "async",
                    }
                } else {
                    div { class: "w-44 h-44 rounded-lg bg-white/5" }
                }
                div { class: "absolute inset-0 rounded-lg flex items-center justify-center opacity-0 group-hover:opacity-100 bg-black/40 transition-opacity",
                    i { class: "fa-solid fa-play text-white text-xl" }
                }
            }
            p { class: "text-sm font-semibold text-white line-clamp-2 break-words group-hover:text-indigo-200 transition-colors", "{title}" }
            p { class: "text-xs text-white/50 line-clamp-1 mt-1", "{artist}" }
        }
    }
}
