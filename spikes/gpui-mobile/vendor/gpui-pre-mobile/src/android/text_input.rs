//! UI-thread IME events are queued, then applied on GPUI's native thread.

use gpui::{
    DispatchEventResult, KeyDownEvent, Keystroke, Modifiers, PlatformInput, PlatformInputHandler,
};
use parking_lot::Mutex;
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

static SESSION: AtomicU64 = AtomicU64::new(0);
static COMPOSING: AtomicBool = AtomicBool::new(false);
static EVENTS: Mutex<VecDeque<ImeEvent>> = Mutex::new(VecDeque::new());
static BACK_REQUESTS: AtomicUsize = AtomicUsize::new(0);

/// kopuz: back belongs to the window, so a new IME session must not discard it.
pub(super) fn enqueue_back() {
    BACK_REQUESTS.fetch_add(1, Ordering::AcqRel);
    crate::mark_text_input_dirty();
}

static IME_DISMISSED: AtomicBool = AtomicBool::new(false);

pub(super) fn note_ime_dismissed() {
    IME_DISMISSED.store(true, Ordering::Release);
}

/// Whether the user closed the IME with back since the last call. The host decides what that
/// means (kopuz blurs the input, so its next tap opens the keyboard again).
pub fn take_ime_dismissed() -> bool {
    IME_DISMISSED.swap(false, Ordering::AcqRel)
}

pub(super) struct ImeEvent {
    pub session: u64,
    pub kind: i32,
    pub text: String,
    pub start: usize,
    pub end: usize,
}

pub(super) fn new_session() -> u64 {
    let session = SESSION.fetch_add(1, Ordering::AcqRel) + 1;
    EVENTS.lock().clear();
    COMPOSING.store(false, Ordering::Release);
    session
}

pub(super) fn enqueue(event: ImeEvent) {
    if event.session == SESSION.load(Ordering::Acquire) {
        EVENTS.lock().push_back(event);
        crate::mark_text_input_dirty();
    }
}

pub(super) fn drain(
    slot: &Rc<RefCell<Option<PlatformInputHandler>>>,
    input: &mut dyn FnMut(PlatformInput) -> DispatchEventResult,
) {
    for _ in 0..BACK_REQUESTS.swap(0, Ordering::AcqRel) {
        input(PlatformInput::KeyDown(KeyDownEvent {
            keystroke: Keystroke {
                key: "escape".into(),
                key_char: None,
                modifiers: Modifiers::default(),
            },
            is_held: false,
            prefer_character_input: false,
        }));
    }
    let events = std::mem::take(&mut *EVENTS.lock());
    for event in events {
        if event.session != SESSION.load(Ordering::Acquire) {
            continue;
        }
        // 4: the IME closed itself (back); kopuz's 5: its enter or action key.
        if event.kind == 4 {
            finish_composition(slot);
            IME_DISMISSED.store(true, Ordering::Release);
            continue;
        }
        if event.kind == 5 {
            finish_composition(slot);
            input(PlatformInput::KeyDown(KeyDownEvent {
                keystroke: Keystroke {
                    key: "enter".into(),
                    key_char: None,
                    modifiers: Modifiers::default(),
                },
                is_held: false,
                prefer_character_input: false,
            }));
            continue;
        }
        // No RefCell borrow spans a GPUI update (which may replace the handler).
        let Some(mut handler) = slot.borrow_mut().take() else {
            forward_legacy_text(event.kind, &event.text, event.start, |text| {
                crate::dispatch_text_input(text);
            });
            continue;
        };
        match event.kind {
            0 => {
                COMPOSING.store(true, Ordering::Release);
                handler.replace_and_mark_text_in_range(
                    None,
                    &event.text,
                    Some(event.start..event.end),
                );
            }
            1 => {
                COMPOSING.store(false, Ordering::Release);
                handler.replace_text_in_range(None, &event.text);
                handler.unmark_text();
            }
            2 | 3 => {
                if let Some(selection) = handler.selected_text_range(false) {
                    let mut adjusted = None;
                    if let Some(text) = handler.text_for_range(0..usize::MAX, &mut adjusted) {
                        let range = deletion_range(
                            &text,
                            selection.range,
                            event.start,
                            event.end,
                            event.kind == 3,
                        );
                        handler.replace_text_in_range(Some(range), "");
                    }
                }
            }
            _ => {}
        }
        let mut current = slot.borrow_mut();
        if current.is_none() {
            *current = Some(handler);
        }
    }
}

pub(super) fn finish_composition(slot: &Rc<RefCell<Option<PlatformInputHandler>>>) {
    if !COMPOSING.swap(false, Ordering::AcqRel) {
        return;
    }
    let handler = slot.borrow_mut().take();
    if let Some(mut handler) = handler {
        handler.unmark_text();
        let mut current = slot.borrow_mut();
        if current.is_none() {
            *current = Some(handler);
        }
    }
    super::jni::reset_keyboard_composition();
}

/// Keep the original TextInput callback contract for clients without a platform
/// input handler. Its backspace sentinel deletes one character per callback;
/// exact UTF-16 ranges and forward deletion require PlatformInputHandler.
fn forward_legacy_text(kind: i32, text: &str, before: usize, mut dispatch: impl FnMut(&str)) {
    match kind {
        1 => dispatch(text),
        2 | 3 => {
            for _ in 0..before {
                dispatch("\x08");
            }
        }
        _ => {}
    }
}

/// Android offers deletion in either UTF-16 units or Unicode code points.
/// Round UTF-16 endpoints outwards so a surrogate pair is never split.
fn deletion_range(
    text: &str,
    selection: std::ops::Range<usize>,
    before: usize,
    after: usize,
    code_points: bool,
) -> std::ops::Range<usize> {
    let mut boundaries = vec![0];
    for ch in text.chars() {
        boundaries.push(boundaries.last().unwrap() + ch.len_utf16());
    }
    let length = *boundaries.last().unwrap();
    if code_points {
        let start_index = boundaries.partition_point(|offset| *offset < selection.start);
        let end_index = boundaries.partition_point(|offset| *offset < selection.end);
        boundaries[start_index.saturating_sub(before)]
            ..boundaries[end_index.saturating_add(after).min(boundaries.len() - 1)]
    } else {
        let start = selection.start.saturating_sub(before);
        let end = selection.end.saturating_add(after).min(length);
        let start_index = boundaries
            .partition_point(|offset| *offset <= start)
            .saturating_sub(1);
        let end_index = boundaries.partition_point(|offset| *offset < end);
        boundaries[start_index]..boundaries[end_index]
    }
}

#[cfg(test)]
mod tests {
    use super::{deletion_range, forward_legacy_text};
    #[test]
    fn legacy_inputs_receive_commits_and_backspace_sentinels() {
        let mut received = Vec::new();
        for (kind, text, before) in [(0, "ni", 0), (1, "你", 0), (2, "", 1), (3, "", 2)] {
            forward_legacy_text(kind, text, before, |text| received.push(text.to_owned()));
        }
        assert_eq!(received, vec!["你", "\x08", "\x08", "\x08"]);
    }

    #[test]
    fn deletion_preserves_surrogate_pairs() {
        assert_eq!(deletion_range("你😀好", 3..3, 1, 0, false), 1..3);
        assert_eq!(deletion_range("你😀好", 1..1, 0, 1, false), 1..3);
        assert_eq!(deletion_range("你😀好", 3..3, 2, 0, true), 0..3);
        assert_eq!(deletion_range("你😀好", 1..1, 0, 2, true), 1..4);
        assert_eq!(deletion_range("你😀好", 0..0, 99, 99, true), 0..4);
    }
}
