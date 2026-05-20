use config::AppConfig;
use dioxus::document::eval;
use dioxus::prelude::*;
use hooks::PlayerController;
use reader::Library;
use std::fmt;

use crate::reorder_buttons::ReorderButtons;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutMode {
    Rightbar,
    Fullscreen,
}

impl fmt::Display for LayoutMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutMode::Rightbar => write!(f, "rightbar"),
            LayoutMode::Fullscreen => write!(f, "fullscreen"),
        }
    }
}

#[component]
pub fn QueueRow(
    queue_idx: usize,
    track: reader::Track,
    cover_url: Option<utils::CoverUrl>,
    layout: LayoutMode,
    can_move_up: bool,
    can_move_down: bool,
    on_play: Callback,
    on_move_up: EventHandler<MouseEvent>,
    on_move_down: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            id: "{layout}__queue-item-{queue_idx}",
            class: match layout {
               LayoutMode::Fullscreen=> "flex items-center gap-4 px-4 py-3 hover:bg-white/5 cursor-pointer rounded transition-colors group",
               LayoutMode::Rightbar => "flex items-center gap-3 px-2 py-2 hover:bg-white/5 cursor-pointer rounded transition-colors group",
            },
            style:  "content-visibility: auto; contain-intrinsic-size: 0 56px;",
            ondoubleclick: move |_| on_play.call(()),

            div {
                class: "w-4 flex justify-center items-end shrink-0",

                span {
                    class: "queue-item-number text-xs group-hover:hidden text-white/60",
                        "{queue_idx + 1}"
                    }

                div {
                    class: "queue-item-icon hidden group-hover:flex items-center justify-center",
                    i { class: "fa-solid fa-play text-xs text-white/60" }
                }
            }

            div {
                class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                style: match layout {
                    LayoutMode::Fullscreen => "width: 48px; height: 48px;",
                    LayoutMode::Rightbar => "width: 40px; height: 40px;"
                },

                if let Some(ref url) = cover_url {
                    img { src: "{url.as_ref()}", class: "w-full h-full object-cover" }
                } else {
                    div {
                        class: "w-full h-full flex items-center justify-center",
                        i {
                            class: "fa-solid fa-music text-white/20",
                            style: match layout {
                                LayoutMode::Fullscreen => "font-size: 14px;",
                                LayoutMode::Rightbar => "font-size: 12px;",
                            },
                        }
                    }
                }
            }

            div {
                class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                div {
                    class: match layout {
                       LayoutMode::Fullscreen => "queue-item-title text-base text-white truncate font-medium",
                       LayoutMode::Rightbar => "queue-item-title text-sm text-white truncate",
                    },
                    "{track.title}"
                },

                div {
                    class: match layout {
                        LayoutMode::Fullscreen => "text-sm text-white/50 truncate group-hover:text-white/70",
                        LayoutMode::Rightbar => "text-xs text-white/50 truncate group-hover:text-white/70"
                    },
                    "{track.artist}"
                }
            }

            ReorderButtons {
                class: "flex flex-col pr-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity",
                can_move_up,
                can_move_down,
                on_move_up,
                on_move_down,
            }
        }
    }
}

#[component]
pub fn QueueSummary(
    queue_count: usize,
    queue_duration: u64,
    current_queue_index: Signal<usize>,
    layout: LayoutMode,
) -> Element {
    let format_queue_duration = |seconds: u64| {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;
        if hours > 0 {
            format!("{hours}:{minutes:02}:{secs:02}")
        } else {
            format!("{minutes}:{secs:02}")
        }
    };

    let queue_summary = format!(
        "{} • {}",
        i18n::t_with("showcase_song_count", &[("count", queue_count.to_string())]),
        format_queue_duration(queue_duration)
    );

    rsx! {
        div {
            class: match layout {
                LayoutMode::Fullscreen => "pt-2 px-4 pb-3 flex gap-2 justify-between uppercase tracking-[0.18em] text-xs",
                LayoutMode::Rightbar => "pt-1 px-2 pb-2 flex gap-2 justify-between uppercase tracking-[0.18em] text-[11px]",
            },

            span {
                class: "text-white/45",
                "{queue_summary}"
            }

            button {
                class: "text-white/60 cursor-pointer",
                onclick: move |_| { eval(&format!("window.__{layout}_scrollIntoView(null)")); },
                "{*current_queue_index.read() + 1}/{queue_count}"
            }
        }
    }
}

#[component]
pub fn QueueListView(
    items: Vec<reader::Track>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    current_queue_index: Signal<usize>,
    layout: LayoutMode,
    is_open: bool,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    if !is_open {
        return rsx! {};
    }

    // Clear functions when the component is dropped
    use_drop(move || {
        let _cleanup = eval(&format!(
            r#"
                if (window.__{layout}_scrollIntoView) delete window.__{layout}_scrollIntoView;
                if (window.__{layout}_updateActiveQueueItem) delete window.__{layout}_updateActiveQueueItem;
            "#,
        ));
    });

    use_hook(move || {
        let scroll_block = match layout {
            LayoutMode::Fullscreen => "start",
            LayoutMode::Rightbar => "end",
        };

        // Fullscreen behaviot: Scroll into view on next queue item when it becomes active only
        // when the current is in view.
        // Rightbar behavior:  Scrolls into view on next queue item when it becomes active only
        // when the current is in view, while the next is not.
        let scroll_when = match layout {
            LayoutMode::Fullscreen => "currentIsInView",
            LayoutMode::Rightbar => "currentIsInView && !nextIsInView",
        };

        let _scroll_func = eval(&format!(
            r#"
                let isFirst = true;
                let latestItem;

                window.__{layout}_scrollIntoView = (nextItem) =>  {{
                    if (latestItem && nextItem) {{
                        const container = document.getElementById('{layout}-queue-list');
                        const containerRect = container.getBoundingClientRect();

                        const currentRect = latestItem.getBoundingClientRect();
                        const currentIsInView = currentRect.top >= containerRect.top && currentRect.bottom <= containerRect.bottom;

                        const nextRect = nextItem.getBoundingClientRect();
                        const nextIsInView = nextRect.top >= containerRect.top && nextRect.bottom <= containerRect.bottom;

                        if ({scroll_when}) {{
                            nextItem.scrollIntoView({{ behavior: 'smooth', block: '{scroll_block}' }});
                        }}

                        latestItem = nextItem;

                    }} else if (isFirst && nextItem) {{
                        nextItem.scrollIntoView({{ behavior: 'smooth', block: '{scroll_block}' }});
                        latestItem = nextItem;
                        isFirst = false;

                    }} else if (latestItem && !nextItem) {{
                        latestItem.scrollIntoView({{ behavior: 'smooth', block: '{scroll_block}' }});
                    }}
                }}
            "#,
        ));

        // Highlight next queue item when it becomes active and dehighlight the current one
        let _update_func = eval(&format!(
            r#"
                let currentQueueItem;
                window.__{layout}_updateActiveQueueItem = (nextIndex) => {{
                    const nextQueueItem = document.getElementById(`{layout}__queue-item-${{nextIndex}}`);

                    if (currentQueueItem != nextQueueItem) {{

                        if (currentQueueItem) {{
                            currentQueueItem.classList.remove("{layout}__active-queue-item");

                            const icon = currentQueueItem.querySelector("i");
                            if (icon) {{ icon.className = "fa-solid fa-play text-xs text-white/60"; }}
                        }}

                        if (nextQueueItem) {{
                            nextQueueItem.classList.add("{layout}__active-queue-item");

                            const icon = nextQueueItem.querySelector("i");
                            if (icon) {{ icon.className = "fa-solid fa-volume-high text-xs"; }}
                        }}

                        window.__{layout}_scrollIntoView(nextQueueItem);
                        currentQueueItem = nextQueueItem;
                    }}
                }}
            "#,
        ));
    });

    use_effect(move || {
        let current_index = *current_queue_index.read();
        let _update = eval(&format!(
            "if (window.__{layout}_updateActiveQueueItem) window.__{layout}_updateActiveQueueItem({current_index});"
        ));
    });

    let cover_max_width = match layout {
        LayoutMode::Fullscreen => 96,
        LayoutMode::Rightbar => 80,
    };

    let get_track_cover = |track: &reader::Track| -> Option<utils::CoverUrl> {
        // Use `peek()` instead of reactive reads here.
        // Cover lookup should not subscribe to library/config updates.
        let lib = library.peek();
        let conf = config.peek();

        let is_server_track = conf.active_source == config::MusicSource::Server;

        if is_server_track {
            if let Some(server) = &conf.server {
                let path_str = track.path.to_string_lossy();
                let url = match server.service {
                    config::MusicService::Jellyfin => {
                        utils::jellyfin_image::jellyfin_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            cover_max_width,
                            80,
                        )
                    }
                    config::MusicService::Subsonic | config::MusicService::Custom => {
                        utils::subsonic_image::subsonic_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            cover_max_width,
                            80,
                        )
                    }
                };
                return utils::map_cover_url(url);
            }
            None
        } else {
            lib.albums
                .iter()
                .find(|a| a.id == track.album_id)
                .and_then(|album| utils::format_artwork_url(album.cover_path.as_ref()))
        }
    };

    let mut play_song_at_index = move |index: usize| {
        ctrl.play_track_no_history(index);
    };

    let mut move_queue_item = move |from: usize, to: usize| {
        ctrl.move_queue_item(from, to);
    };

    let queue_count = items.len();
    let queue_duration: u64 = items.iter().map(|t| t.duration).sum();

    rsx! {
        style {
            "
            .{layout}__active-queue-item {{
                background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent);
            }}

            .{layout}__active-queue-item .queue-item-title {{
                color: var(--color-indigo-500) !important;
            }}

            .{layout}__active-queue-item .queue-item-number {{
                display: none !important;
            }}

            .{layout}__active-queue-item .queue-item-icon {{
                display: flex !important;
            }}

            .{layout}__active-queue-item .queue-item-icon i {{
                color: var(--color-indigo-500) !important;
            }}
            "
        },

        if items.is_empty() {
            div { class: "text-white/30 text-center py-10 text-sm", "{i18n::t(\"no_more_songs\")}" }
        } else {
            QueueSummary {
                key: "{layout}",
                queue_count,
                queue_duration,
                current_queue_index,
                layout: layout.clone()
            }

            div {
                id: "{layout}-queue-list",
                class: match layout {
                    LayoutMode::Fullscreen => "flex-1 overflow-y-auto px-4 py-2 space-y-1",
                    LayoutMode::Rightbar => "flex-1 overflow-y-auto px-2 py-2 space-y-1",
                },

                for (queue_idx, track) in items.into_iter().enumerate() {
                    {
                        rsx! {
                            QueueRow {
                                key: "{layout}-row-{queue_idx}",
                                queue_idx: queue_idx,
                                cover_url: get_track_cover(&track),
                                track: track,
                                layout: layout.clone(),
                                can_move_up: queue_idx > 0,
                                can_move_down: queue_idx + 1 < queue_count,
                                on_play: move |_| play_song_at_index(queue_idx),
                                on_move_up: move |_| move_queue_item(queue_idx, queue_idx - 1),
                                on_move_down: move |_| move_queue_item(queue_idx, queue_idx + 1),
                            }
                        }
                    }
                }
            }
        }
    }
}
