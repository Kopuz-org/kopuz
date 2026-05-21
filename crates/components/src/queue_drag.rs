use dioxus::document::eval;
use dioxus::prelude::*;
use reader::models::Track;
use std::sync::{Mutex, OnceLock};

pub const RIGHTBAR_DROPZONE_ID: &str = "rightbar-dropzone";
pub const RIGHTBAR_QUEUE_DROP_TARGET_CLASS: &str = "kopuz-rightbar-queue-drop-target";
pub static DRAGGED_QUEUE_TRACK: OnceLock<Mutex<Option<Track>>> = OnceLock::new();

fn dragged_queue_track() -> &'static Mutex<Option<Track>> {
    DRAGGED_QUEUE_TRACK.get_or_init(|| Mutex::new(None))
}

pub fn take_dragged_queue_track() -> Option<Track> {
    dragged_queue_track().lock().ok()?.take()
}

pub fn has_dragged_queue_track() -> bool {
    dragged_queue_track()
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false)
}

pub fn set_dragged_queue_track(track: Track) {
    if let Ok(mut guard) = dragged_queue_track().lock() {
        *guard = Some(track);
    }
}

pub fn cancel_dragged_queue_track() {
    clear_dragged_queue_track();
}

pub fn clear_dragged_queue_track() {
    if let Ok(mut guard) = dragged_queue_track().lock() {
        *guard = None;
    }
}

pub fn handle_select_click(
    is_selected: bool,
    is_selection_mode: bool,
    on_select: Option<EventHandler<bool>>,
) {
    if is_selection_mode {
        if let Some(handler) = on_select {
            handler.call(!is_selected);
        }
    }
}

pub fn install_rightbar_drag_handlers() {
    let _ = eval(
        r#"
        if (!document.__kopuzTrackDragInstalled) {
            document.__kopuzTrackDragInstalled = true;

            const isTrackRowDrag = (event) => {
                return !!(event.target && event.target.closest && event.target.closest('.kopuz-track-row-draggable'));
            };

            const isRightbarDrop = (event) => {
                const selector = '.kopuz-rightbar-queue-drop-target';
                const direct = event.target && event.target.closest && event.target.closest(selector);
                if (direct) return true;
                const hovered = document.elementFromPoint(event.clientX, event.clientY);
                return !!(hovered && hovered.closest && hovered.closest(selector));
            };

            document.addEventListener('dragstart', (event) => {
                if (!isTrackRowDrag(event) || !event.dataTransfer) return;
                event.dataTransfer.effectAllowed = 'copyMove';
                event.dataTransfer.setData('text/plain', 'kopuz-track');
                event.dataTransfer.setData('application/x-kopuz-track', '1');
            }, true);

            let rightbarAutoScrollFrame = null;
            let rightbarAutoScrollY = null;

            window.__kopuzRightbarStopAutoScroll = () => {
                rightbarAutoScrollY = null;
                if (rightbarAutoScrollFrame !== null) {
                    cancelAnimationFrame(rightbarAutoScrollFrame);
                    rightbarAutoScrollFrame = null;
                }
            };

            const rightbarAutoScrollTick = () => {
                const zone = document.getElementById('rightbar-dropzone');
                if (!zone || rightbarAutoScrollY === null) {
                    window.__kopuzRightbarStopAutoScroll();
                    return;
                }

                const rect = zone.getBoundingClientRect();
                const threshold = Math.min(96, Math.max(48, rect.height * 0.18));
                const maxStep = 14;
                let step = 0;

                if (rightbarAutoScrollY < rect.top + threshold) {
                    const distance = Math.max(0, rightbarAutoScrollY - rect.top);
                    const factor = 1 - Math.min(distance / threshold, 1);
                    step = -Math.max(2, maxStep * factor);
                } else if (rightbarAutoScrollY > rect.bottom - threshold) {
                    const distance = Math.max(0, rect.bottom - rightbarAutoScrollY);
                    const factor = 1 - Math.min(distance / threshold, 1);
                    step = Math.max(2, maxStep * factor);
                }

                if (step !== 0) {
                    zone.scrollTop += step;
                    rightbarAutoScrollFrame = requestAnimationFrame(rightbarAutoScrollTick);
                } else {
                    window.__kopuzRightbarStopAutoScroll();
                }
            };

            window.__kopuzRightbarAutoScroll = (clientY) => {
                const zone = document.getElementById('rightbar-dropzone');
                if (!zone) return;

                rightbarAutoScrollY = clientY;
                if (rightbarAutoScrollFrame === null) {
                    rightbarAutoScrollFrame = requestAnimationFrame(rightbarAutoScrollTick);
                }
            };

            const acceptRightbarDrop = (event) => {
                if (!isRightbarDrop(event)) return;
                event.preventDefault();
                window.__kopuzRightbarAutoScroll(event.clientY);
                if (event.dataTransfer) {
                    event.dataTransfer.dropEffect = 'copy';
                }
            };

            window.addEventListener('dragenter', acceptRightbarDrop, true);
            window.addEventListener('dragover', acceptRightbarDrop, true);
            window.addEventListener('drop', acceptRightbarDrop, true);
            window.addEventListener('drop', window.__kopuzRightbarStopAutoScroll, true);
            window.addEventListener('mouseup', window.__kopuzRightbarStopAutoScroll, true);
            window.addEventListener('dragend', window.__kopuzRightbarStopAutoScroll, true);
            document.addEventListener('dragenter', acceptRightbarDrop, true);
            document.addEventListener('dragover', acceptRightbarDrop, true);
            document.addEventListener('drop', acceptRightbarDrop, true);
            document.addEventListener('drop', window.__kopuzRightbarStopAutoScroll, true);
            document.addEventListener('mouseup', window.__kopuzRightbarStopAutoScroll, true);
            document.addEventListener('dragend', window.__kopuzRightbarStopAutoScroll, true);
        }
        "#,
    );
}

pub fn rightbar_auto_scroll(client_y: f64) {
    let _ = eval(&format!(
        "if (window.__kopuzRightbarAutoScroll) window.__kopuzRightbarAutoScroll({client_y});"
    ));
}

pub fn stop_rightbar_auto_scroll() {
    let _ =
        eval("if (window.__kopuzRightbarStopAutoScroll) window.__kopuzRightbarStopAutoScroll();");
}

pub fn clear_rightbar_drop_target(
    mut is_queue_drag_over: Signal<bool>,
    mut queue_drop_index: Signal<Option<usize>>,
) {
    is_queue_drag_over.set(false);
    queue_drop_index.set(None);
}

pub fn clear_rightbar_drag_state(
    is_queue_drag_over: Signal<bool>,
    queue_drop_index: Signal<Option<usize>>,
    mut queue_reorder_from: Signal<Option<usize>>,
    mut queue_reorder_did_move: Signal<bool>,
) {
    clear_rightbar_drop_target(is_queue_drag_over, queue_drop_index);
    queue_reorder_from.set(None);
    queue_reorder_did_move.set(false);
}

pub fn cancel_rightbar_drag(
    is_queue_drag_over: Signal<bool>,
    queue_drop_index: Signal<Option<usize>>,
    queue_reorder_from: Signal<Option<usize>>,
    queue_reorder_did_move: Signal<bool>,
) {
    clear_rightbar_drag_state(
        is_queue_drag_over,
        queue_drop_index,
        queue_reorder_from,
        queue_reorder_did_move,
    );
    cancel_dragged_queue_track();
    stop_rightbar_auto_scroll();
}

pub fn start_rightbar_reorder(
    queue_idx: usize,
    mut queue_drop_index: Signal<Option<usize>>,
    mut queue_reorder_from: Signal<Option<usize>>,
    mut queue_reorder_did_move: Signal<bool>,
) {
    queue_reorder_from.set(Some(queue_idx));
    queue_drop_index.set(Some(queue_idx));
    queue_reorder_did_move.set(false);
}

pub fn update_rightbar_drop_target(
    target_idx: usize,
    queue_reorder_from: Signal<Option<usize>>,
    mut is_queue_drag_over: Signal<bool>,
    mut queue_drop_index: Signal<Option<usize>>,
    mut queue_reorder_did_move: Signal<bool>,
) {
    if let Some(from) = *queue_reorder_from.read() {
        is_queue_drag_over.set(true);
        queue_drop_index.set(Some(target_idx));
        if from != target_idx {
            queue_reorder_did_move.set(true);
        }
    } else if has_dragged_queue_track() {
        is_queue_drag_over.set(true);
        queue_drop_index.set(Some(target_idx));
    }
}

pub fn update_rightbar_end_drop_target(
    end_drop_index: usize,
    queue_reorder_from: Signal<Option<usize>>,
    mut is_queue_drag_over: Signal<bool>,
    mut queue_drop_index: Signal<Option<usize>>,
    mut queue_reorder_did_move: Signal<bool>,
) {
    if let Some(from) = *queue_reorder_from.read() {
        is_queue_drag_over.set(true);
        queue_drop_index.set(Some(end_drop_index));
        if from + 1 < end_drop_index {
            queue_reorder_did_move.set(true);
        }
    } else if has_dragged_queue_track() {
        is_queue_drag_over.set(true);
        queue_drop_index.set(Some(end_drop_index));
    }
}

pub fn rightbar_queue_row_class(is_reorder_source: bool) -> &'static str {
    if is_reorder_source {
        "flex items-center gap-3 px-2 py-2 bg-white/10 cursor-grabbing rounded-lg transition-colors group opacity-70"
    } else {
        "flex items-center gap-3 px-2 py-2 hover:bg-white/5 cursor-grab active:cursor-grabbing rounded-lg transition-colors group"
    }
}
