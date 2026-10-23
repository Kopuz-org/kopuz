//! Rebinding a key: the recorder that waits for one keystroke, and the row that shows what a command is bound to.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Context, EventEmitter, InteractiveElement as _, IntoElement, Keystroke,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div,
};

use crate::format::t;
use crate::{DirtyDot, Keycap, focus, icons, motion, style};

/// A lone modifier is not a shortcut, so a recording waits for a real key.
const MODIFIERS: &[&str] = &[
    "shift", "control", "ctrl", "alt", "platform", "cmd", "function",
];

/// How a recording ended; it has already stopped by the time this is heard.
#[derive(Debug, Clone)]
pub enum Recorded {
    Pressed(Keystroke),
    Cancelled,
}

/// Takes the next keystroke before any binding can claim it; escape abandons the recording.
#[derive(Default)]
pub struct KeyRecorder {
    capture: Option<Subscription>,
}

impl EventEmitter<Recorded> for KeyRecorder {}

impl KeyRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_recording(&self) -> bool {
        self.capture.is_some()
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        self.capture = Some(cx.intercept_keystrokes(move |event, _, cx| {
            if MODIFIERS.contains(&event.keystroke.key.as_str()) {
                return;
            }
            cx.stop_propagation();
            let keystroke = event.keystroke.clone();
            this.update(cx, |this, cx| this.finish(keystroke, cx)).ok();
        }));
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.capture.take().is_some() {
            cx.notify();
        }
    }

    fn finish(&mut self, keystroke: Keystroke, cx: &mut Context<Self>) {
        self.capture = None;
        cx.emit(match keystroke.key.as_str() {
            "escape" => Recorded::Cancelled,
            _ => Recorded::Pressed(keystroke),
        });
        cx.notify();
    }
}

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

const HOVER: u64 = 0;
const WASH: u64 = 1;

/// One command and its key: the keycap, "recording" while it waits, and a reset once it has been changed.
#[derive(IntoElement)]
pub struct ShortcutRow {
    id: SharedString,
    label: SharedString,
    keystroke: Option<String>,
    recording: bool,
    changed: bool,
    ringed: bool,
    on_click: Option<OnClick>,
    on_reset: Option<OnClick>,
}

impl ShortcutRow {
    /// Keyed by the command's own id.
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            keystroke: None,
            recording: false,
            changed: false,
            ringed: false,
            on_click: None,
            on_reset: None,
        }
    }

    /// The key as the keymap writes it, such as `ctrl-shift-p`; none reads as unbound.
    pub fn keystroke(mut self, keystroke: Option<String>) -> Self {
        self.keystroke = keystroke;
        self
    }

    pub fn recording(mut self, recording: bool) -> Self {
        self.recording = recording;
        self
    }

    /// The binding differs from the default, so the row wears a dot and offers a reset.
    pub fn changed(mut self, changed: bool) -> Self {
        self.changed = changed;
        self
    }

    /// The keyboard is on this row.
    pub fn ringed(mut self, ringed: bool) -> Self {
        self.ringed = ringed;
        self
    }

    pub fn on_click(
        mut self,
        click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(click));
        self
    }

    pub fn on_reset(
        mut self,
        reset: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_reset = Some(Rc::new(reset));
        self
    }
}

impl RenderOnce for ShortcutRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = self.id.clone();
        let (over, pointer) = motion::hovered(motion::channel(&key, HOVER), window, cx);
        let wash = motion::color(
            motion::channel(&key, WASH),
            style::white(if over { style::RAISED } else { 0. }, cx),
            window,
            cx,
        );
        let chip = match (self.recording, self.keystroke.as_deref()) {
            (true, _) => div()
                .px_2()
                .rounded(style::radius(cx))
                .bg(style::accent(style::ACTIVE, cx))
                .text_size(style::text_10())
                .text_color(cx.theme().foreground)
                .child(t("shortcuts_recording"))
                .into_any_element(),
            (false, Some(keystroke)) => Keycap::new(keystroke).into_any_element(),
            (false, None) => div()
                .text_size(style::text_10())
                .text_color(style::white(style::TEXT_SUBTLE, cx))
                .child(t("shortcuts_unbound"))
                .into_any_element(),
        };
        let (click, reset) = (self.on_click, self.on_reset);
        h_flex()
            .id(SharedString::from(format!("shortcut-row-{}", self.id)))
            .relative()
            .w_full()
            .gap_3()
            .px_3()
            .py_1p5()
            .items_center()
            .rounded(style::radius_lg())
            .cursor_pointer()
            .bg(wash)
            .on_hover(motion::track(pointer))
            .on_click(move |event, window, cx| {
                if let Some(click) = &click {
                    click(event, window, cx);
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(self.label),
            )
            // The dot a changed key wears, so a rebind is visible without opening the file.
            .when(self.changed, |row| row.child(DirtyDot))
            .child(chip)
            .when(self.changed, |row| {
                row.child(
                    Button::new(SharedString::from(format!("shortcut-reset-{}", self.id)))
                        .ghost()
                        .xsmall()
                        .icon(icons::icon(icons::REFRESH))
                        .tooltip(t("reset_home"))
                        .on_click(move |event, window, cx| {
                            if let Some(reset) = &reset {
                                reset(event, window, cx);
                            }
                        }),
                )
            })
            .when(self.ringed, |row| {
                row.child(focus::ring(style::radius_lg(), cx))
            })
    }
}
