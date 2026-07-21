use config::AppConfig;
use dioxus::document::eval;
use dioxus::prelude::*;
use hooks::PlayerController;
use serde_json::Value;
use std::fmt;

use crate::virtual_scroll::{VirtualScrollView, use_virtual_scroll};

use crate::queue_drag::{
    RIGHTBAR_DROPZONE_ID, RIGHTBAR_QUEUE_DROP_TARGET_CLASS, cancel_rightbar_drag,
    clear_rightbar_drop_target, has_dragged_queue_track, install_rightbar_drag_handlers,
    rightbar_auto_scroll, rightbar_queue_row_class, rightbar_reorder_move_target,
    shift_indices_at_or_after, start_rightbar_reorder, stop_rightbar_auto_scroll,
    take_dragged_queue_tracks, update_rightbar_drop_target, update_rightbar_end_drop_target,
};
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
    is_reorder_source: bool,
    is_active: bool,
    on_play: Callback,
    on_row_mouse_down: EventHandler<MouseEvent>,
    on_row_mouse_move: EventHandler<MouseEvent>,
    on_move_up: EventHandler<MouseEvent>,
    on_move_down: EventHandler<MouseEvent>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let base_class = match layout {
        LayoutMode::Fullscreen => {
            if is_reorder_source {
                "flex items-center gap-4 px-4 py-3 bg-white/10 cursor-grabbing rounded transition-colors group opacity-70"
            } else {
                "flex items-center gap-4 px-4 py-3 hover:bg-white/5 cursor-grab active:cursor-grabbing rounded transition-colors group"
            }
        }
        LayoutMode::Rightbar => rightbar_queue_row_class(is_reorder_source),
    };
    let row_class = if is_active {
        format!("{base_class} {layout}__active-queue-item")
    } else {
        base_class.to_string()
    };
    let row_icon_class = if is_active {
        "fa-solid fa-volume-high text-xs"
    } else {
        "fa-solid fa-play text-xs text-white/60"
    };

    rsx! {
        div {
            id: "{layout}__queue-item-{queue_idx}",
            class: "{row_class}",
            style: match layout {
                LayoutMode::Fullscreen => "",
                LayoutMode::Rightbar => {
                    "content-visibility: auto; contain-intrinsic-size: 0 56px;"
                }
            },
            onmousedown: move |evt| on_row_mouse_down.call(evt),
            onmousemove: move |evt| on_row_mouse_move.call(evt),
            ondoubleclick: move |_| on_play.call(()),

            div { class: "w-4 flex justify-center items-end shrink-0",

                span { class: "queue-item-number text-xs group-hover:hidden text-white/60",
                    "{queue_idx + 1}"
                }

                div { class: "queue-item-icon hidden group-hover:flex items-center justify-center",
                    i { class: "{row_icon_class}" }
                }
            }

            if config.read().show_row_images {
                div {
                    class: "rounded-md overflow-hidden flex-shrink-0 shadow-sm",
                    style: match layout {
                        LayoutMode::Fullscreen => "width: 48px; height: 48px; background: url('data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%27400%27 height=%27400%27 viewBox=%270 0 400 400%27%3E%3Crect width=%27400%27 height=%27400%27 fill=%27%231e1b2e%27/%3E%3Ccircle cx=%27200%27 cy=%27180%27 r=%2770%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27/%3E%3Cpath d=%27M155 280 Q200 240 245 280%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27 stroke-linecap=%27round%27/%3E%3C/svg%3E') center/cover no-repeat, rgba(255,255,255,0.05);",
                        LayoutMode::Rightbar => "width: 40px; height: 40px; background: url('data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%27400%27 height=%27400%27 viewBox=%270 0 400 400%27%3E%3Crect width=%27400%27 height=%27400%27 fill=%27%231e1b2e%27/%3E%3Ccircle cx=%27200%27 cy=%27180%27 r=%2770%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27/%3E%3Cpath d=%27M155 280 Q200 240 245 280%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27 stroke-linecap=%27round%27/%3E%3C/svg%3E') center/cover no-repeat, rgba(255,255,255,0.05);",
                    },

                    if let Some(ref url) = cover_url {
                        img {
                            src: "{url.as_ref()}",
                            class: "w-full h-full object-cover",
                        }
                    }
                }
            }

            div { class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                div {
                    class: match layout {
                        LayoutMode::Fullscreen => {
                            "queue-item-title text-base text-white truncate font-medium"
                        }
                        LayoutMode::Rightbar => "queue-item-title text-sm text-white truncate",
                    },
                    "{track.title}"
                }

                div {
                    class: match layout {
                        LayoutMode::Fullscreen => {
                            "text-sm text-white/50 truncate group-hover:text-white/70"
                        }
                        LayoutMode::Rightbar => {
                            "text-xs text-white/50 truncate group-hover:text-white/70"
                        }
                    },
                    "{track.artist}"
                }
            }

            div { onmousedown: move |evt| evt.stop_propagation(),
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
}

#[component]
pub fn QueueSummary(
    queue_count: usize,
    queue_duration: u64,
    current_queue_index: Signal<usize>,
    layout: LayoutMode,
) -> Element {
    let ctrl = use_context::<PlayerController>();
    let is_radio = if let Some(track) = ctrl.get_track_at(*current_queue_index.read()) {
        // As of today, radio tracks have a duration of u64::MAX, if this
        // invariant ever changes, this logic must be updated as well
        track.duration == u64::MAX
    } else {
        false
    };

    if is_radio {
        return rsx! {};
    }

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
                LayoutMode::Fullscreen => {
                    "pt-2 px-4 pb-3 flex gap-2 justify-between text-xs"
                }
                LayoutMode::Rightbar => {
                    "pt-1 px-2 pb-2 flex gap-2 justify-between text-[11px]"
                }
            },

            span { class: "text-white/45", "{queue_summary}" }

            button {
                class: "text-white/60 cursor-pointer",
                onclick: move |_| {
                    eval(&format!("window.__{layout}_scrollIntoView(null)"));
                },
                "{*current_queue_index.read() + 1}/{queue_count}"
            }
        }
    }
}

const RIGHTBAR_ITEM_HEIGHT: f64 = 60.0;
const FULLSCREEN_ITEM_HEIGHT: f64 = 76.0;

#[component]
pub fn QueueListView(
    items: Vec<reader::Track>,
    config: Signal<AppConfig>,
    current_queue_index: Signal<usize>,
    layout: LayoutMode,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut is_queue_drag_over = use_signal(|| false);
    let mut queue_drop_index = use_signal(|| None::<usize>);
    let mut queue_reorder_from = use_signal(|| None::<usize>);
    let mut queue_reorder_did_move = use_signal(|| false);
    let mut pending_queue_reorder = use_signal(|| None::<(usize, f64, f64)>);
    const QUEUE_REORDER_THRESHOLD_PX: f64 = 6.0;
    const QUEUE_ROW_DROP_SPLIT_Y_PX: f64 = 23.0;
    let queue_list_id = match layout {
        LayoutMode::Rightbar => RIGHTBAR_DROPZONE_ID,
        LayoutMode::Fullscreen => "fullscreen-queue-list",
    };

    let item_height = match layout {
        LayoutMode::Rightbar => RIGHTBAR_ITEM_HEIGHT,
        LayoutMode::Fullscreen => FULLSCREEN_ITEM_HEIGHT,
    };
    let scroll_stat = use_signal(|| 0.0_f64);
    let container_height = use_signal(|| 0.0_f64);

    use_effect(move || {
        if layout == LayoutMode::Rightbar {
            install_rightbar_drag_handlers();
        }
    });

    use_effect(move || {
        if layout != LayoutMode::Rightbar {
            return;
        }

        spawn(async move {
            let mut outside_mouseup = eval(
                r#"
                if (!window.__kopuzRightbarOutsideMouseUpInstalled) {
                    window.__kopuzRightbarOutsideMouseUpInstalled = true;
                    document.addEventListener('mouseup', (event) => {
                        const target = event.target;
                        const insideRightbar = !!(target && target.closest && target.closest('#rightbar-root'));
                        const overQueueTarget = !!(target && target.closest && target.closest('.rightbar-queue-drop-target'));
                        if (!insideRightbar || !overQueueTarget) {
                            dioxus.send('cancel');
                        }
                    }, true);
                }
                "#,
            );

            while outside_mouseup.recv::<Value>().await.is_ok() {
                cancel_rightbar_drag(
                    is_queue_drag_over,
                    queue_drop_index,
                    queue_reorder_from,
                    queue_reorder_did_move,
                );
                pending_queue_reorder.set(None);
            }
        });
    });

    // Clear functions when the component is dropped
    use_drop(move || {
        let _cleanup = eval(&format!(
            r#"
                for (const key of ['queueJump', 'updateActiveQueueItem'])
                    delete window[`__{layout}_${{key}}`];
            "#,
        ));
    });

    let mut auto_sync = use_signal(|| true);

    use_hook(move || {
        // The queue is virtualized, so the playing row usually isn't in the
        // DOM — jumping goes through scrollTop math on the row index instead
        // of scrollIntoView. Far jumps snap instantly; near ones glide.
        let _jump_func = eval(&format!(
            r#"
                window.__{layout}_queueJump = (index) => {{
                    const attempt = (tries) => {{
                        const container = document.getElementById('{queue_list_id}');
                        if (!container) {{
                            if (tries > 0) requestAnimationFrame(() => attempt(tries - 1));
                            return;
                        }}
                        const target = Math.max(0, index * {item_height} - container.clientHeight * 0.35);
                        const behavior = Math.abs(container.scrollTop - target) > 3000 ? 'auto' : 'smooth';
                        container.scrollTo({{ top: target, behavior }});
                    }};
                    attempt(10);
                }};
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

                        currentQueueItem = nextQueueItem;
                    }}
                }}
            "#,
        ));
    });

    // Hand scroll control back to the user the moment they scroll the queue
    // themselves; the sync button re-arms auto-follow. Watching input events
    // (wheel / touch / scrollbar grab) instead of `scroll` keeps our own
    // smooth jumps — whose end time the browser doesn't expose — from
    // disarming auto-follow.
    use_future(move || async move {
        let mut listener = eval(&format!(
            r#"
                const attach = () => {{
                    const container = document.getElementById('{queue_list_id}');
                    if (!container) {{ requestAnimationFrame(attach); return; }}
                    const disarm = () => dioxus.send('user_scroll');
                    container.addEventListener('wheel', disarm, {{ passive: true }});
                    container.addEventListener('touchmove', disarm, {{ passive: true }});
                    container.addEventListener('mousedown', (event) => {{
                        if (event.target === container) disarm();
                    }});
                }};
                attach();
            "#
        ));

        while let Ok(val) = listener.recv::<Value>().await {
            if val.as_str() == Some("user_scroll") {
                auto_sync.set(false);
            }
        }
    });

    use_effect(move || {
        let current_index = *current_queue_index.read();
        let _update = eval(&format!(
            "if (window.__{layout}_updateActiveQueueItem) window.__{layout}_updateActiveQueueItem({current_index});"
        ));
        if *auto_sync.read() {
            let _jump = eval(&format!("window.__{layout}_queueJump?.({current_index});"));
        }
    });

    let cover_max_width = match layout {
        LayoutMode::Fullscreen => 96,
        LayoutMode::Rightbar => 80,
    };

    let get_track_cover = |track: &reader::Track| -> Option<utils::CoverUrl> {
        // `peek()`, not a reactive read — cover lookup shouldn't subscribe to
        // config updates. Source-agnostic via the cover seam; the track
        // self-describes its cover (local path projected from its album by the DB).
        server::cover::track(&config.peek(), track, cover_max_width)
    };

    let mut play_song_at_index = move |index: usize| {
        ctrl.play_track_no_history(index);
    };

    let mut move_queue_item = move |from: usize, to: usize| {
        ctrl.move_queue_item(from, to);
    };

    let mut insert_queue_tracks = move |insert_at: usize, tracks: Vec<reader::Track>| {
        if tracks.is_empty() {
            return;
        }
        let count = tracks.len();
        let visual_insert = insert_at;
        /* FCK SHUFFLE */
        if *ctrl.shuffle.peek() {
            let shuffle_order = ctrl.shuffle_order.peek().clone();
            let physical_insert = shuffle_order
                .get(visual_insert)
                .copied()
                .unwrap_or_else(|| ctrl.queue.peek().len());
            ctrl.queue.with_mut(|queue| {
                let insert_pos = physical_insert.min(queue.len());
                for (offset, track) in tracks.into_iter().enumerate() {
                    queue.insert(insert_pos + offset, track);
                }
            });
            ctrl.shuffle_order.with_mut(|order| {
                shift_indices_at_or_after(order, physical_insert, count);
                let insert_pos = visual_insert.min(order.len());
                for i in 0..count {
                    order.insert(insert_pos + i, physical_insert + i);
                }
            });
            let current_idx = *ctrl.current_queue_index.peek();
            if visual_insert <= current_idx {
                ctrl.current_queue_index.set(current_idx + count);
            }
            ctrl.history.with_mut(|history| {
                shift_indices_at_or_after(history, physical_insert, count);
            });
        } else {
            let insert_at = insert_at.min(ctrl.queue.peek().len());
            ctrl.queue.with_mut(|queue| {
                for (offset, track) in tracks.into_iter().enumerate() {
                    queue.insert(insert_at + offset, track);
                }
            });
        }
    };

    let queue_count = items.len();
    let queue_duration: u64 = items
        .iter()
        .filter_map(|t| (t.duration != u64::MAX).then_some(t.duration))
        .fold(0, |acc, d| acc.saturating_add(d));

    let scroll_info = use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        queue_count,
        item_height,
    );
    let start_index = scroll_info.start_index;
    let items_to_render = scroll_info.items_to_render;
    let top_pad = scroll_info.top_pad;
    let bottom_pad = scroll_info.bottom_pad;

    let end_drop_target = if matches!(layout, LayoutMode::Rightbar | LayoutMode::Fullscreen) {
        let end_drop_index = queue_count;
        let is_end_drop_target = *queue_drop_index.read() == Some(end_drop_index);
        Some(rsx! {
            div {
                key: "queue-drop-end-{end_drop_index}",
                class: "{RIGHTBAR_QUEUE_DROP_TARGET_CLASS} px-1 py-2",
                style: match layout {
                    LayoutMode::Rightbar => "min-height: 45vh;",
                    LayoutMode::Fullscreen => "min-height: 8rem;",
                },
                onmouseenter: move |_| {
                    update_rightbar_end_drop_target(
                        end_drop_index,
                        queue_reorder_from,
                        is_queue_drag_over,
                        queue_drop_index,
                        queue_reorder_did_move,
                    );
                },
                onmousemove: move |_| {
                    update_rightbar_end_drop_target(
                        end_drop_index,
                        queue_reorder_from,
                        is_queue_drag_over,
                        queue_drop_index,
                        queue_reorder_did_move,
                    );
                },
                onmouseup: move |evt| {
                    evt.stop_propagation();
                    pending_queue_reorder.set(None);
                    is_queue_drag_over.set(false);
                    let drop_index = queue_drop_index.peek().unwrap_or(end_drop_index);
                    queue_drop_index.set(None);
                    let reorder_from = *queue_reorder_from.read();
                    if let Some(from) = reorder_from {
                        if let Some(to) = rightbar_reorder_move_target(
                            from,
                            drop_index,
                            queue_count,
                        ) {
                            queue_reorder_did_move.set(true);
                            move_queue_item(from, to);
                        }
                        queue_reorder_from.set(None);
                        return;
                    }
                    insert_queue_tracks(end_drop_index, take_dragged_queue_tracks());
                },
                ondragenter: move |evt| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    is_queue_drag_over.set(true);
                    queue_drop_index.set(Some(end_drop_index));
                },
                ondragover: move |evt| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    is_queue_drag_over.set(true);
                    queue_drop_index.set(Some(end_drop_index));
                },
                ondrop: move |evt| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    pending_queue_reorder.set(None);
                    is_queue_drag_over.set(false);
                    queue_drop_index.set(None);
                    insert_queue_tracks(end_drop_index, take_dragged_queue_tracks());
                },
                if is_end_drop_target {
                    div { class: "pointer-events-none",
                        div {
                            class: "w-full rounded-full",
                            style: "height: 3px; background: var(--color-indigo-500); box-shadow: 0 0 10px rgba(129, 140, 248, 0.8);",
                        }
                    }
                }
            }
        })
    } else {
        None
    };

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
        }

        if items.is_empty() {
            div { class: "text-white/30 text-center py-10 text-sm", "{i18n::t(\"no_more_songs\")}" }
        } else {
            div { class: "relative flex flex-col flex-1 min-h-0",
            QueueSummary {
                key: "{layout}",
                queue_count,
                queue_duration,
                current_queue_index,
                layout,
            }

            VirtualScrollView {
                id: queue_list_id.to_string(),
                class: match layout {
                    LayoutMode::Fullscreen => "flex-1 overflow-y-auto px-4 py-2".to_string(),
                    LayoutMode::Rightbar => "flex-1 overflow-y-auto px-2 py-2 relative".to_string(),
                },
                scroll_stat,
                container_height,
                item_height,
                saved_scroll: 0.0,
                top_pad,
                bottom_pad,
                bottom_content: end_drop_target,
                on_mouse_leave: move |_| {
                    clear_rightbar_drop_target(is_queue_drag_over, queue_drop_index);
                    pending_queue_reorder.set(None);
                    if layout == LayoutMode::Rightbar {
                        stop_rightbar_auto_scroll();
                    }
                },
                on_mouse_move: move |evt: MouseEvent| {
                    if layout == LayoutMode::Rightbar
                        && (has_dragged_queue_track() || queue_reorder_from.read().is_some())
                    {
                        rightbar_auto_scroll(evt.client_coordinates().y);
                    }
                },
                for (i, track) in items.iter().enumerate().skip(start_index).take(items_to_render) {
                    {
                        let queue_idx = i;
                        let track = track.clone();
                        let cover_url = get_track_cover(&track);
                        let can_move_up = queue_idx > 0;
                        let can_move_down = queue_idx + 1 < queue_count;
                        let is_reorder_source = *queue_reorder_from.read() == Some(queue_idx);
                        let is_active = *current_queue_index.read() == queue_idx;
                        let is_drop_target = *queue_drop_index.read() == Some(queue_idx);

                        rsx! {
                            if matches!(layout, LayoutMode::Rightbar | LayoutMode::Fullscreen) {
                                div {
                                    style: "height: {item_height}px; box-sizing: border-box;",
                                    key: "{layout}-drop-target-{queue_idx}",
                                    class: RIGHTBAR_QUEUE_DROP_TARGET_CLASS,
                                    onmouseenter: move |evt: MouseEvent| {
                                        let point = evt.element_coordinates();
                                        let row_drop_index = if point.y >= QUEUE_ROW_DROP_SPLIT_Y_PX {
                                            queue_idx + 1
                                        } else {
                                            queue_idx
                                        };
                                        update_rightbar_drop_target(
                                            row_drop_index,
                                            queue_reorder_from,
                                            is_queue_drag_over,
                                            queue_drop_index,
                                            queue_reorder_did_move,
                                        );
                                    },
                                    onmousemove: move |evt: MouseEvent| {
                                        let point = evt.element_coordinates();
                                        let row_drop_index = if point.y >= QUEUE_ROW_DROP_SPLIT_Y_PX {
                                            queue_idx + 1
                                        } else {
                                            queue_idx
                                        };
                                        update_rightbar_drop_target(
                                            row_drop_index,
                                            queue_reorder_from,
                                            is_queue_drag_over,
                                            queue_drop_index,
                                            queue_reorder_did_move,
                                        );
                                    },
                                    onmouseup: move |evt| {
                                        evt.stop_propagation();
                                        pending_queue_reorder.set(None);
                                        is_queue_drag_over.set(false);
                                        let drop_index = queue_drop_index.peek().unwrap_or(queue_idx);
                                        queue_drop_index.set(None);
                                        let reorder_from = *queue_reorder_from.read();
                                        if let Some(from) = reorder_from {
                                            if let Some(to) = rightbar_reorder_move_target(
                                                from,
                                                drop_index,
                                                queue_count,
                                            ) {
                                                queue_reorder_did_move.set(true);
                                                move_queue_item(from, to);
                                            }
                                            queue_reorder_from.set(None);
                                            return;
                                        }
                                        insert_queue_tracks(drop_index, take_dragged_queue_tracks());
                                    },
                                    ondragenter: move |evt| {
                                        evt.prevent_default();
                                        evt.stop_propagation();
                                        let point = evt.element_coordinates();
                                        let row_drop_index = if point.y >= QUEUE_ROW_DROP_SPLIT_Y_PX {
                                            queue_idx + 1
                                        } else {
                                            queue_idx
                                        };
                                        update_rightbar_drop_target(
                                            row_drop_index,
                                            queue_reorder_from,
                                            is_queue_drag_over,
                                            queue_drop_index,
                                            queue_reorder_did_move,
                                        );
                                    },
                                    ondragover: move |evt| {
                                        evt.prevent_default();
                                        evt.stop_propagation();
                                        let point = evt.element_coordinates();
                                        let row_drop_index = if point.y >= QUEUE_ROW_DROP_SPLIT_Y_PX {
                                            queue_idx + 1
                                        } else {
                                            queue_idx
                                        };
                                        update_rightbar_drop_target(
                                            row_drop_index,
                                            queue_reorder_from,
                                            is_queue_drag_over,
                                            queue_drop_index,
                                            queue_reorder_did_move,
                                        );
                                    },
                                    ondrop: move |evt| {
                                        evt.prevent_default();
                                        evt.stop_propagation();
                                        pending_queue_reorder.set(None);
                                        is_queue_drag_over.set(false);
                                        let point = evt.element_coordinates();
                                        let row_drop_index = if point.y >= QUEUE_ROW_DROP_SPLIT_Y_PX {
                                            queue_idx + 1
                                        } else {
                                            queue_idx
                                        };
                                        let drop_index = queue_drop_index.peek().unwrap_or(row_drop_index);
                                        queue_drop_index.set(None);
                                        insert_queue_tracks(drop_index, take_dragged_queue_tracks());
                                    },
                                    if is_drop_target {
                                        div { class: "px-1 py-2 pointer-events-none",
                                            div {
                                                class: "w-full rounded-full",
                                                style: "height: 3px; background: var(--color-indigo-500); box-shadow: 0 0 10px rgba(129, 140, 248, 0.8);",
                                            }
                                        }
                                    }
                                    QueueRow {
                                        queue_idx,
                                        cover_url,
                                        track,
                                        layout,
                                        can_move_up,
                                        can_move_down,
                                        is_reorder_source,
                                        is_active,
                                        on_play: move |_| {
                                            if !*queue_reorder_did_move.read() {
                                                play_song_at_index(queue_idx);
                                            }
                                            queue_reorder_did_move.set(false);
                                        },
                                        on_row_mouse_down: move |evt: MouseEvent| {
                                            evt.stop_propagation();
                                            let coords = evt.client_coordinates();
                                            pending_queue_reorder.set(Some((queue_idx, coords.x, coords.y)));
                                            queue_reorder_did_move.set(false);
                                        },
                                        on_row_mouse_move: move |evt: MouseEvent| {
                                            evt.stop_propagation();
                                            let point = evt.element_coordinates();
                                            let row_drop_index = if point.y >= QUEUE_ROW_DROP_SPLIT_Y_PX {
                                                queue_idx + 1
                                            } else {
                                                queue_idx
                                            };

                                            if queue_reorder_from.read().is_some() {
                                                is_queue_drag_over.set(true);
                                                queue_drop_index.set(Some(row_drop_index));
                                                if let Some(from) = *queue_reorder_from.read()
                                                    && rightbar_reorder_move_target(
                                                        from,
                                                        row_drop_index,
                                                        queue_count,
                                                    )
                                                    .is_some()
                                                {
                                                    queue_reorder_did_move.set(true);
                                                }
                                                return;
                                            }
                                            let pending = *pending_queue_reorder.read();
                                            if let Some((from_idx, start_x, start_y)) = pending
                                                && from_idx == queue_idx
                                            {
                                                let coords = evt.client_coordinates();
                                                let dx = coords.x - start_x;
                                                let dy = coords.y - start_y;
                                                if dx.hypot(dy) >= QUEUE_REORDER_THRESHOLD_PX {
                                                    pending_queue_reorder.set(None);
                                                    start_rightbar_reorder(
                                                        queue_idx,
                                                        queue_drop_index,
                                                        queue_reorder_from,
                                                        queue_reorder_did_move,
                                                    );
                                                    queue_drop_index.set(Some(row_drop_index));
                                                    if rightbar_reorder_move_target(
                                                        queue_idx,
                                                        row_drop_index,
                                                        queue_count,
                                                    )
                                                    .is_some()
                                                    {
                                                        queue_reorder_did_move.set(true);
                                                    }
                                                }
                                            }
                                        },
                                        on_move_up: move |_| {
                                            if let Some(prev_idx) = queue_idx.checked_sub(1) {
                                                move_queue_item(queue_idx, prev_idx);
                                            }
                                        },
                                        on_move_down: move |_| move_queue_item(queue_idx, queue_idx + 1),
                                    }
                                }
                            } else {
                                QueueRow {
                                    key: "{layout}-row-{queue_idx}",
                                    queue_idx,
                                    cover_url,
                                    track,
                                    layout,
                                    can_move_up,
                                    can_move_down,
                                    is_reorder_source: false,
                                    is_active,
                                    on_play: move |_| play_song_at_index(queue_idx),
                                    on_row_mouse_down: move |_: MouseEvent| {},
                                    on_row_mouse_move: move |_: MouseEvent| {},
                                    on_move_up: move |_| move_queue_item(queue_idx, queue_idx - 1),
                                    on_move_down: move |_| move_queue_item(queue_idx, queue_idx + 1),
                                }
                            }
                        }
                    }
                }
            }

            if !auto_sync() {
                button {
                    class: "absolute bottom-4 right-4 z-10 flex items-center justify-center w-9 h-9 rounded-full bg-black/40 hover:bg-black/60 backdrop-blur text-white/90 shadow-lg ring-1 ring-white/10 transition-colors",
                    "aria-label": i18n::t("jump_to_current_song").to_string(),
                    title: i18n::t("jump_to_current_song").to_string(),
                    onclick: move |_| auto_sync.set(true),
                    svg {
                        class: "w-5 h-5",
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        path { d: "M21 12a9 9 0 1 1-2.64-6.36" }
                        polyline { points: "21 3 21 9 15 9" }
                    }
                }
            }
            }
        }
    }
}
