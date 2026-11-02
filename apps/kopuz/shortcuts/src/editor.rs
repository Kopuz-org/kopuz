//! Settings → Interface → Shortcuts: rebind a key, see what it would collide with, put it back.

use components::focus::{Axis, Roving, Step};
use components::format::{t, t_with};
use components::shortcut::{KeyRecorder, Recorded, ShortcutRow};
use components::style;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, SharedString, Styled, Subscription, Window, div,
};

/// One listed command, flattened out of its group so the arrows walk the whole panel.
struct Row {
    id: &'static str,
    label: SharedString,
    group: Option<&'static str>,
}

pub(crate) struct Shortcuts {
    rows: Vec<Row>,
    roving: Roving,
    recording: Option<usize>,
    refused: Option<SharedString>,
    recorder: Entity<KeyRecorder>,
    _recorded: Subscription,
}

impl Shortcuts {
    pub(crate) fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut rows = Vec::new();
        for (title, entries) in crate::groups(cx) {
            let mut heading = Some(title);
            for entry in entries {
                rows.push(Row {
                    id: entry.id,
                    label: entry.label,
                    group: heading.take(),
                });
            }
        }
        let mut roving = Roving::new(Axis::Column, cx);
        roving.set_len(rows.len());
        let recorder = cx.new(|_| KeyRecorder::new());
        let recorded = cx.subscribe(&recorder, |this, _, event: &Recorded, cx| match event {
            Recorded::Pressed(keystroke) => this.pressed(commands::written(keystroke), cx),
            Recorded::Cancelled => this.stop(cx),
        });
        Self {
            rows,
            roving,
            recording: None,
            refused: None,
            recorder,
            _recorded: recorded,
        }
    }

    /// Take the next keystroke for `index`.
    fn record(&mut self, index: usize, cx: &mut Context<Self>) {
        self.recording = Some(index);
        self.refused = None;
        self.recorder.update(cx, |recorder, cx| recorder.start(cx));
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.recording = None;
        self.recorder.update(cx, |recorder, cx| recorder.stop(cx));
        cx.notify();
    }

    /// What a recorded keystroke does: a collision is refused, anything else binds.
    fn pressed(&mut self, written: String, cx: &mut Context<Self>) {
        let Some(index) = self.recording else {
            return;
        };
        let Some(id) = self.rows.get(index).map(|row| row.id) else {
            self.stop(cx);
            return;
        };
        if let Some(held) = commands::conflict(id, &written, cx) {
            let label = held.label(&store::Target::None, cx);
            self.refused = Some(t_with(
                "shortcuts_conflict",
                &[
                    (
                        "key",
                        components::keycap::written(&written).to_string().into(),
                    ),
                    ("command", label.to_string().into()),
                ],
            ));
            self.stop(cx);
            return;
        }
        commands::rebind(id, Some(written), cx);
        self.stop(cx);
    }

    fn reset(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(id) = self.rows.get(index).map(|row| row.id) {
            commands::rebind(id, None, cx);
        }
        self.refused = None;
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.recording.is_some() {
            return;
        }
        match self.roving.walk(event) {
            Step::Moved(_) => cx.notify(),
            Step::Activate(at) => self.record(at, cx),
            Step::Ignored => cx.propagate(),
        }
    }

    fn row(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.rows.get(index)?;
        if !components::settings::matches(&[row.label.as_str()], cx) {
            return None;
        }
        Some(
            ShortcutRow::new(row.id, row.label.clone())
                .keystroke(commands::keystroke_for(row.id, cx))
                .recording(self.recording == Some(index))
                .changed(store::layout(cx).shortcuts.contains_key(row.id))
                .ringed(self.roving.is_active(index, window))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.roving.set_cursor(index);
                    match this.recording == Some(index) {
                        true => this.stop(cx),
                        false => this.record(index, cx),
                    }
                }))
                .on_reset(cx.listener(move |this, _, _, cx| this.reset(index, cx)))
                .into_any_element(),
        )
    }
}

impl Render for Shortcuts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut blocks: Vec<AnyElement> = Vec::new();
        for index in 0..self.rows.len() {
            let Some(rendered) = self.row(index, window, cx) else {
                continue;
            };
            if let Some(title) = self.rows[index].group {
                blocks.push(
                    div()
                        .w_full()
                        .pt_2()
                        .px_3()
                        .text_xs()
                        .font_semibold()
                        .text_color(style::white(style::TEXT_MUTED, cx))
                        .child(t(title))
                        .into_any_element(),
                );
            }
            blocks.push(rendered);
        }
        let list = self
            .roving
            .group(v_flex().id("shortcut-list"), "ShortcutList")
            .w_full()
            .gap_0p5()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key(event, cx)))
            .children(blocks);
        components::settings::section(crate::title())
            .child(
                div()
                    .w_full()
                    .px_3()
                    .pb_1()
                    .text_xs()
                    .text_color(style::white(style::TEXT_SUBTLE, cx))
                    .child(t("shortcuts_editor_hint")),
            )
            .children(self.refused.clone().map(|said| {
                div()
                    .w_full()
                    .px_3()
                    .pb_1()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(said)
            }))
            .child(list)
    }
}
