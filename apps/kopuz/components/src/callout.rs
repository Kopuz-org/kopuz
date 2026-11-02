//! A framed notice: a sentence with an optional glyph, and a command it offers to copy.

use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled, Window, div,
};

use crate::format::t;
use crate::{icons, style};

/// `text-white/70`, the sentence of a plain notice.
const INFO_TEXT: f32 = 0.70;
/// `text-white/40`, a plain notice's glyph.
const INFO_ICON: f32 = 0.40;
/// `border-white/30` around the copyable command.
const COMMAND_BORDER: f32 = 0.30;
/// `bg-neutral-900/79` behind it.
const COMMAND_BG: f32 = 0.79;

/// How loud a notice is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CalloutTone {
    /// Says how things are, in the quiet card colours.
    #[default]
    Info,
    /// Something will not work until it is dealt with, in the warning colour.
    Warning,
}

#[derive(IntoElement)]
pub struct Callout {
    id: Option<SharedString>,
    text: SharedString,
    tone: CalloutTone,
    icon: Option<&'static str>,
    command: Option<SharedString>,
}

impl Callout {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            id: None,
            text: text.into(),
            tone: CalloutTone::Info,
            icon: None,
            command: None,
        }
    }

    /// Keys the copy button; two notices with one sentence on screen need distinct ones.
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn tone(mut self, tone: CalloutTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A command shown under the sentence with a button that copies it.
    pub fn copyable(mut self, command: impl Into<SharedString>) -> Self {
        self.command = Some(command.into());
        self
    }
}

impl RenderOnce for Callout {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let warning = self.tone == CalloutTone::Warning;
        let ink = match warning {
            true => cx.theme().warning,
            false => style::white(INFO_TEXT, cx),
        };
        let (border, wash) = match warning {
            true => (
                cx.theme().warning.opacity(0.3),
                cx.theme().warning.opacity(0.1),
            ),
            false => (
                style::white(style::BORDER, cx),
                style::white(style::RAISED, cx),
            ),
        };
        let glyph = self.icon.map(|path| {
            icons::icon(path)
                .with_size(style::text_11())
                .text_color(match warning {
                    true => ink,
                    false => style::white(INFO_ICON, cx),
                })
        });
        let id = self.id.unwrap_or_else(|| self.text.clone());
        v_flex()
            .w_full()
            .gap_2()
            .px_5()
            .py_3()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(border)
            .bg(wash)
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .children(glyph)
                    .child(
                        div()
                            .min_w_0()
                            .map(|line| match warning {
                                true => line.text_xs(),
                                false => line.text_sm(),
                            })
                            .text_color(ink)
                            .child(self.text),
                    ),
            )
            .children(self.command.map(|command| {
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .px_1p5()
                            .py_1()
                            .rounded(style::radius(cx))
                            .border_1()
                            .border_color(style::white(COMMAND_BORDER, cx))
                            .bg(style::black(COMMAND_BG, cx))
                            .text_size(style::text_10())
                            .child(command.clone()),
                    )
                    .child(
                        Clipboard::new(SharedString::from(format!("callout-copy-{id}")))
                            .value(command)
                            .tooltip(t("copy")),
                    )
            }))
    }
}
