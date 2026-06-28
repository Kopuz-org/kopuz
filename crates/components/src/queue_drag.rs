use dioxus::document::eval;
use dioxus::prelude::*;
use reader::models::Track;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub const RIGHTBAR_DROPZONE_ID: &str = "rightbar-dropzone";
pub const RIGHTBAR_QUEUE_DROP_TARGET_CLASS: &str = "rightbar-queue-drop-target";
pub static DRAGGED_QUEUE_TRACKS: OnceLock<Mutex<Vec<Track>>> = OnceLock::new();
static QUEUE_DRAG_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn shift_indices_at_or_after(indices: &mut [usize], at: usize, by: usize) {
    for idx in indices {
        if *idx >= at {
            *idx += by;
        }
    }
}

pub fn rightbar_reorder_move_target(
    from: usize,
    drop_index: usize,
    queue_count: usize,
) -> Option<usize> {
    if from >= queue_count || drop_index > queue_count {
        return None;
    }

    let to = if from < drop_index {
        drop_index.saturating_sub(1)
    } else {
        drop_index
    };

    (to < queue_count && to != from).then_some(to)
}

pub fn set_queue_drag_enabled(enabled: bool) {
    QUEUE_DRAG_ENABLED.store(enabled, Ordering::Relaxed);
    if !enabled {
        clear_dragged_queue_track();
    }
}

pub fn is_queue_drag_enabled() -> bool {
    QUEUE_DRAG_ENABLED.load(Ordering::Relaxed)
}

fn dragged_queue_tracks() -> &'static Mutex<Vec<Track>> {
    DRAGGED_QUEUE_TRACKS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn take_dragged_queue_tracks() -> Vec<Track> {
    let tracks = dragged_queue_tracks()
        .lock()
        .map(|mut guard| std::mem::take(&mut *guard))
        .unwrap_or_default();
    hide_queue_drag_preview();
    tracks
}

pub fn take_dragged_queue_track() -> Option<Track> {
    take_dragged_queue_tracks().into_iter().next()
}

pub fn has_dragged_queue_track() -> bool {
    dragged_queue_tracks()
        .lock()
        .map(|guard| !guard.is_empty())
        .unwrap_or(false)
}

pub fn set_dragged_queue_track(
    track: Track,
    cover_url: Option<String>,
    client_x: f64,
    client_y: f64,
) {
    if !is_queue_drag_enabled() {
        return;
    }

    let title = track.title.clone();
    let artist = track.artist.clone();
    if let Ok(mut guard) = dragged_queue_tracks().lock() {
        *guard = vec![track];
    }
    show_queue_drag_preview(&title, &artist, cover_url.as_deref(), client_x, client_y);
}

pub fn set_dragged_queue_tracks(tracks: Vec<Track>, client_x: f64, client_y: f64) {
    if !is_queue_drag_enabled() || tracks.is_empty() {
        return;
    }

    let count = tracks.len();
    if let Ok(mut guard) = dragged_queue_tracks().lock() {
        *guard = tracks;
    }
    show_queue_drag_count_preview(count, client_x, client_y);
}

pub fn cancel_dragged_queue_track() {
    clear_dragged_queue_track();
}

pub fn clear_dragged_queue_track() {
    if let Ok(mut guard) = dragged_queue_tracks().lock() {
        guard.clear();
    }
    hide_queue_drag_preview();
}

pub fn move_queue_drag_preview(client_x: f64, client_y: f64) {
    let _ = eval(&format!(
        "if (window.__kopuzMoveQueueDragPreview) window.__kopuzMoveQueueDragPreview({client_x}, {client_y});"
    ));
}

fn show_queue_drag_count_preview(count: usize, client_x: f64, client_y: f64) {
    let payload = json!({
        "count": count,
        "clientX": client_x,
        "clientY": client_y,
    });
    let _ = eval(&format!(
        "if (window.__kopuzShowQueueDragCountPreview) window.__kopuzShowQueueDragCountPreview({payload});"
    ));
}

fn show_queue_drag_preview(
    title: &str,
    artist: &str,
    cover_url: Option<&str>,
    client_x: f64,
    client_y: f64,
) {
    let payload = json!({
        "title": title,
        "artist": artist,
        "coverUrl": cover_url,
        "clientX": client_x,
        "clientY": client_y,
    });
    let _ = eval(&format!(
        "if (window.__kopuzShowQueueDragPreview) window.__kopuzShowQueueDragPreview({payload});"
    ));
}

fn hide_queue_drag_preview() {
    let _ = eval("if (window.__kopuzHideQueueDragPreview) window.__kopuzHideQueueDragPreview();");
}

pub fn handle_select_click(
    is_selected: bool,
    is_selection_mode: bool,
    on_select: Option<EventHandler<bool>>,
) {
    if is_selection_mode && let Some(handler) = on_select {
        handler.call(!is_selected);
    }
}

// stop dragging from cover url
pub fn install_native_artwork_drag_prevention() {
    let _ = eval(
        r#"
        if (!document.__kopuzNativeArtworkDragPreventionInstalled) {
            document.__kopuzNativeArtworkDragPreventionInstalled = true;

            document.addEventListener('dragstart', (event) => {
                const target = event.target;
                const isTrackRowDrag = !!(target && target.closest && target.closest('.track-row-draggable'));
                if (!isTrackRowDrag) {
                    event.preventDefault();
                    event.stopPropagation();
                }
            }, true);
        }
        "#,
    );
}

pub fn install_rightbar_drag_handlers() {
    let track_count_singular = serde_json::to_string(&i18n::t("track_count_singular").to_string())
        .unwrap_or_else(|_| "\"1 track\"".to_string());
    let track_count_plural = serde_json::to_string(
        &i18n::t_with("track_count", &[("count", "__COUNT__".to_string())]).to_string(),
    )
    .unwrap_or_else(|_| "\"__COUNT__ tracks\"".to_string());
    install_native_artwork_drag_prevention();
    let script = r#"
        if (!document.__kopuzTrackDragInstalled) {{
            document.__kopuzTrackDragInstalled = true;

            const isTrackRowDrag = (event) => {
                return !!(event.target && event.target.closest && event.target.closest('.track-row-draggable'));
            };

            const isRightbarDrop = (event) => {
                const selector = '.rightbar-queue-drop-target';
                const direct = event.target && event.target.closest && event.target.closest(selector);
                if (direct) return true;
                const hovered = document.elementFromPoint(event.clientX, event.clientY);
                return !!(hovered && hovered.closest && hovered.closest(selector));
            };

            const syncQueueDragPreviewTheme = (preview) => {
                const themedRoot = Array.from(document.querySelectorAll('[class*="theme-"]'))
                    .find((el) => el.id !== 'queue-drag-preview' && Array.from(el.classList).some((cls) => cls.startsWith('theme-')));
                Array.from(preview.classList)
                    .filter((cls) => cls.startsWith('theme-'))
                    .forEach((cls) => preview.classList.remove(cls));
                if (themedRoot) {
                    Array.from(themedRoot.classList)
                        .filter((cls) => cls.startsWith('theme-'))
                        .forEach((cls) => preview.classList.add(cls));
                }
            };

            const ensureQueueDragPreview = () => {
                let preview = document.getElementById('queue-drag-preview');
                if (preview) {
                    syncQueueDragPreviewTheme(preview);
                    return preview;
                }

                preview = document.createElement('div');
                preview.id = 'queue-drag-preview';
                preview.style.cssText = `
                    position: fixed;
                    left: 0;
                    top: 0;
                    width: 260px;
                    display: none;
                    align-items: center;
                    gap: 10px;
                    padding: 8px 10px;
                    border-radius: 12px;
                    border: 1px solid rgba(255,255,255,0.12);
                    background-color: var(--color-neutral-900);
                    box-shadow: 0 16px 45px rgba(0,0,0,0.38);
                    backdrop-filter: blur(16px);
                    pointer-events: none;
                    z-index: 2147483647;
                    transform: translate3d(-9999px, -9999px, 0);
                `;
                preview.innerHTML = `
                    <div data-cover style="width:40px;height:40px;border-radius:8px;overflow:hidden;background:rgba(255,255,255,0.06);flex:0 0 auto;display:flex;align-items:center;justify-content:center;"></div>
                    <div style="min-width:0;display:flex;flex-direction:column;gap:2px;">
                        <div data-title style="font-size:13px;font-weight:600;color:var(--color-white);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;"></div>
                        <div data-artist style="font-size:11px;color:var(--color-slate-400);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;"></div>
                    </div>
                `;
                syncQueueDragPreviewTheme(preview);
                document.body.appendChild(preview);
                return preview;
            };

            const moveQueueDragPreview = (clientX, clientY) => {
                const preview = document.getElementById('queue-drag-preview');
                if (!preview || preview.style.display === 'none') return;
                preview.style.transform = `translate3d(${clientX + 14}px, ${clientY + 14}px, 0)`;
            };

            window.__kopuzMoveQueueDragPreview = moveQueueDragPreview;
            window.__kopuzShowQueueDragCountPreview = ({ count, clientX, clientY }) => {
                const preview = ensureQueueDragPreview();
                syncQueueDragPreviewTheme(preview);
                const cover = preview.querySelector('[data-cover]');
                const titleEl = preview.querySelector('[data-title]');
                const artistEl = preview.querySelector('[data-artist]');

                if (titleEl) {{
                    titleEl.textContent =
                        count === 1
                            ? __TRACK_COUNT_SINGULAR__
                            : __TRACK_COUNT_PLURAL__.replace("__COUNT__", count);
                }}

                if (artistEl) artistEl.textContent = '';
                if (cover) {
                    cover.textContent = '';
                    cover.innerHTML = '';
                    const icon = document.createElement('i');
                    icon.className = 'fa-solid fa-list-ul';
                    icon.style.cssText = 'font-size:14px;color:rgba(255,255,255,0.7);';
                    cover.appendChild(icon);
                }

                preview.style.display = 'flex';
                moveQueueDragPreview(clientX, clientY);
            };

            window.__kopuzShowQueueDragPreview = ({ title, artist, coverUrl, clientX, clientY }) => {
                const preview = ensureQueueDragPreview();
                syncQueueDragPreviewTheme(preview);
                const cover = preview.querySelector('[data-cover]');
                const titleEl = preview.querySelector('[data-title]');
                const artistEl = preview.querySelector('[data-artist]');

                if (titleEl) titleEl.textContent = title || '';
                if (artistEl) artistEl.textContent = artist || '';
                if (cover) {
                    cover.textContent = '';
                    cover.innerHTML = '';
                    if (coverUrl) {
                        const img = document.createElement('img');
                        img.src = coverUrl;
                        img.style.cssText = 'width:100%;height:100%;object-fit:cover;display:block;';
                        cover.appendChild(img);
                    } else {
                        const icon = document.createElement('i');
                        icon.className = 'fa-solid fa-music';
                        icon.style.cssText = 'font-size:12px;color:rgba(255,255,255,0.24);';
                        cover.appendChild(icon);
                    }
                }

                preview.style.display = 'flex';
                moveQueueDragPreview(clientX, clientY);
            };

            window.__kopuzHideQueueDragPreview = () => {
                const preview = document.getElementById('queue-drag-preview');
                if (!preview) return;
                preview.style.display = 'none';
                preview.style.transform = 'translate3d(-9999px, -9999px, 0)';
            };

            document.addEventListener('mousemove', (event) => {
                moveQueueDragPreview(event.clientX, event.clientY);
            }, true);

            document.addEventListener('dragstart', (event) => {
                if (!isTrackRowDrag(event) || !event.dataTransfer) return;
                event.dataTransfer.effectAllowed = 'copyMove';
                event.dataTransfer.setData('text/plain', 'track');
                event.dataTransfer.setData('application/x-track', '1');
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
        "#
    .replace("{{", "{")
    .replace("}}", "}")
    .replace("__TRACK_COUNT_SINGULAR__", &track_count_singular)
    .replace("__TRACK_COUNT_PLURAL__", &track_count_plural);
    let _ = eval(&script);
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
