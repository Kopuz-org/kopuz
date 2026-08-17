use dioxus::prelude::*;

#[derive(Clone, PartialEq, Debug)]
pub struct VirtualScrollInfo {
    pub start_index: usize,
    pub items_to_render: usize,
    pub top_pad: f64,
    pub bottom_pad: f64,
}

/// `start_index` is clamped to the last full window rather than the last item: a
/// scroll offset left over from a longer list (the signal outlives the DOM
/// element across a re-render, and a playlist's rows arrive after its first
/// paint) would otherwise pin the window to the final row and render a handful
/// of tracks adrift in a viewport-sized spacer.
///
/// `item_height` has to be the row's real height — the pads are laid out in
/// multiples of it, so an estimate that disagrees with the DOM drifts the list
/// out from under the scrollbar.
pub fn use_virtual_scroll(
    scroll_top: f64,
    container_height: f64,
    total_items: usize,
    item_height: f64,
) -> VirtualScrollInfo {
    let window_size = if container_height <= 0.0 || container_height.is_nan() {
        30
    } else {
        (container_height / item_height).ceil() as usize
    };
    let buffer_size = 10;

    let start_index = {
        let max_start = total_items.saturating_sub(window_size + 2 * buffer_size);
        let calc = (scroll_top - (buffer_size as f64) * item_height) / item_height;
        (calc.floor().max(0.0) as usize).min(max_start)
    };

    let end_index = {
        let last_index = start_index + 2 * buffer_size + window_size;
        let last_index_inclusive = last_index.saturating_sub(1);
        if total_items == 0 {
            0
        } else {
            last_index_inclusive.min(total_items - 1)
        }
    };

    let items_to_render = if total_items == 0 {
        0
    } else {
        (end_index + 1).saturating_sub(start_index)
    };

    let top_pad = (start_index as f64) * item_height;

    let bottom_pad = {
        let total_height = (total_items as f64) * item_height;
        let rendered_height = (items_to_render as f64) * item_height;
        (total_height - rendered_height - top_pad).max(0.0)
    };

    VirtualScrollInfo {
        start_index,
        items_to_render,
        top_pad,
        bottom_pad,
    }
}

#[cfg(test)]
mod tests {
    use super::use_virtual_scroll;

    const ROW: f64 = 44.0;

    #[test]
    fn stale_scroll_offset_against_a_short_list_renders_from_the_top() {
        let info = use_virtual_scroll(5_000.0, 660.0, 15, ROW);

        assert_eq!(info.start_index, 0);
        assert_eq!(info.items_to_render, 15);
        assert_eq!(info.top_pad, 0.0);
        assert_eq!(info.bottom_pad, 0.0);
    }

    #[test]
    fn scrolled_to_the_end_of_a_long_list_keeps_a_full_window() {
        let total = 200;
        let info = use_virtual_scroll(total as f64 * ROW - 660.0, 660.0, total, ROW);

        assert!(info.items_to_render >= 30);
        assert_eq!(info.start_index + info.items_to_render, total);
        assert_eq!(
            info.top_pad + info.items_to_render as f64 * ROW + info.bottom_pad,
            total as f64 * ROW
        );
    }

    #[test]
    fn pads_and_rendered_rows_always_sum_to_the_full_list_height() {
        for scroll_top in [0.0, 500.0, 4_000.0, 100_000.0] {
            let info = use_virtual_scroll(scroll_top, 660.0, 120, ROW);
            assert_eq!(
                info.top_pad + info.items_to_render as f64 * ROW + info.bottom_pad,
                120.0 * ROW,
                "scroll_top {scroll_top}"
            );
        }
    }
}

#[component]
pub fn VirtualScrollView(
    id: String,
    #[props(default = "flex-1 overflow-y-auto".to_string())] class: String,
    mut scroll_stat: Signal<f64>,
    mut container_height: Signal<f64>,
    item_height: f64,
    saved_scroll: f64,
    top_pad: f64,
    bottom_pad: f64,
    children: Element,
    #[props(default)] onscroll: Option<EventHandler<f64>>,
    #[props(default)] on_mouse_leave: Option<EventHandler<MouseEvent>>,
    #[props(default)] on_mouse_move: Option<EventHandler<MouseEvent>>,
    #[props(default)] bottom_content: Option<Element>,
) -> Element {
    // The mount-time restore below can race the async row count: at mount the
    // list height is ~0, so the browser clamps scrollTop to 0, and once the
    // spacer grows the viewport sits on a blank pad. Re-apply the restore once
    // when the spacer-driving pads first become non-zero.
    let mut mounted = use_signal(|| false);
    let mut restored = use_signal(|| false);
    let pad_total = use_memo(use_reactive!(|(top_pad, bottom_pad)| top_pad + bottom_pad));
    let id_for_restore = id.clone();
    use_effect(move || {
        let pads = pad_total();
        if saved_scroll > 0.0 && pads > 0.0 && *mounted.read() && !*restored.peek() {
            restored.set(true);
            let safe_id = id_for_restore.replace('\\', "\\\\").replace('\'', "\\'");
            let _ = dioxus::document::eval(&format!(
                "let el = document.getElementById('{}'); if (el) el.scrollTop = {};",
                safe_id, saved_scroll
            ));
        }
    });
    rsx! {
        div {
            id: "{id}",
            class: "{class}",
            onmounted: move |event| {
                spawn(async move {
                    if let Ok(window) = event.get_client_rect().await {
                        container_height.set(window.height());
                    }
                });
                if saved_scroll > 0.0 {
                    let safe_id = id.replace('\\', "\\\\").replace('\'', "\\'");
                    let _ = dioxus::document::eval(&format!(
                        "let el = document.getElementById('{}'); if (el) el.scrollTop = {};",
                        safe_id, saved_scroll
                    ));
                }
                mounted.set(true);
            },
            onscroll: move |event| {
                let new_scroll = event.scroll_top();
                let old_row = (*scroll_stat.peek() / item_height).floor() as i64;
                let new_row = if new_scroll.is_finite() {
                    (new_scroll / item_height).floor() as i64
                } else {
                    old_row
                };
                if new_row != old_row {
                    scroll_stat.set(new_scroll);
                }

                let height = event.client_height() as f64;
                if (height - *container_height.peek()).abs() > 1.0 {
                    container_height.set(height);
                }

                if let Some(handler) = onscroll.as_ref() {
                    handler.call(new_scroll);
                }
            },
            onmouseleave: move |evt| {
                if let Some(handler) = on_mouse_leave.as_ref() {
                    handler.call(evt);
                }
            },
            onmousemove: move |evt| {
                if let Some(handler) = on_mouse_move.as_ref() {
                    handler.call(evt);
                }
            },
            if top_pad > 0.0 {
                div { style: "height: {top_pad}px; flex-shrink: 0;" }
            }
            {children}
            if bottom_pad > 0.0 {
                div { style: "height: {bottom_pad}px; flex-shrink: 0;" }
            }
            {bottom_content}
        }
    }
}
