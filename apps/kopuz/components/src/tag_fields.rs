//! Labelled text boxes and read-only facts: the metadata editors' fields and the details they show.

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Div, Entity, IntoElement, ParentElement as _, RenderOnce, SharedString,
    Styled, Window, div,
};

use crate::style;
use crate::{ErrorBanner, FieldProblem};

/// `text-white/35`, a label over a value or a field.
const LABEL: f32 = 0.35;

/// `text-[10px] font-bold text-white/35`: the label over a value or an edit field.
pub fn caption(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(style::text_10())
        .font_bold()
        .text_color(style::white(LABEL, cx))
        .child(label.into())
}

/// A text box under its label, flagged in the danger colour with the reason it cannot be saved.
#[derive(IntoElement)]
pub struct LabeledInput {
    label: SharedString,
    input: Entity<InputState>,
    problem: Option<SharedString>,
    hint: Option<SharedString>,
    disabled: bool,
    framed: bool,
}

impl LabeledInput {
    /// An empty label leaves the caption out.
    pub fn new(label: impl Into<SharedString>, input: &Entity<InputState>) -> Self {
        Self {
            label: label.into(),
            input: input.clone(),
            problem: None,
            hint: None,
            disabled: false,
            framed: true,
        }
    }

    pub fn problem(mut self, problem: Option<SharedString>) -> Self {
        self.problem = problem;
        self
    }

    /// A quiet line under the box.
    pub fn hint(mut self, hint: Option<SharedString>) -> Self {
        self.hint = hint;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Whether the box wears the raised surface of a dialog's tag field or the input's own look.
    pub fn framed(mut self, framed: bool) -> Self {
        self.framed = framed;
        self
    }
}

impl RenderOnce for LabeledInput {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let danger = cx.theme().danger;
        let flagged = self.problem.is_some();
        v_flex()
            .w_full()
            .gap_1()
            .when(!self.label.is_empty(), |field| {
                field.child(caption(self.label, cx))
            })
            .child(
                Input::new(&self.input)
                    .disabled(self.disabled)
                    .when(self.framed, |input| {
                        input
                            .bg(style::white(style::HOVER, cx))
                            .border_color(if flagged {
                                danger
                            } else {
                                style::white(style::BORDER, cx)
                            })
                            .rounded(style::radius_lg())
                            .px_3()
                            .py_2()
                            .text_sm()
                    }),
            )
            .when_some(self.hint, |field, hint| {
                field.child(
                    div()
                        .text_xs()
                        .text_color(style::white(style::TEXT_MUTED, cx))
                        .child(hint),
                )
            })
            .when_some(self.problem, |field, problem| {
                field.child(FieldProblem::new(problem))
            })
    }
}

/// Facts as captioned values; a row with nothing to say is left out.
#[derive(IntoElement, Default)]
pub struct FactList {
    rows: Vec<(SharedString, SharedString)>,
}

impl FactList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn row(mut self, label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        self.rows.push((label.into(), value.into()));
        self
    }
}

impl RenderOnce for FactList {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex().gap_3().children(
            self.rows
                .into_iter()
                .filter(|(_, value)| !value.trim().is_empty())
                .map(|(label, value)| {
                    v_flex().gap_0p5().child(caption(label, cx)).child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(value),
                    )
                }),
        )
    }
}

/// The tag editor's body, shared by the single and the batch dialogs: fields, why a save failed, and the warning.
#[derive(IntoElement, Default)]
pub struct TagFieldGrid {
    rows: Vec<AnyElement>,
    failure: Option<SharedString>,
    note: Option<SharedString>,
}

impl TagFieldGrid {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, field: LabeledInput) -> Self {
        self.rows.push(field.into_any_element());
        self
    }

    /// Two fields side by side, for numbers that read as a pair.
    pub fn pair(mut self, left: LabeledInput, right: LabeledInput) -> Self {
        self.rows.push(
            h_flex()
                .gap_3()
                .items_start()
                .child(div().flex_1().child(left))
                .child(div().flex_1().child(right))
                .into_any_element(),
        );
        self
    }

    /// Why the last write did not land; the editor stays open holding it.
    pub fn failure(mut self, failure: Option<SharedString>) -> Self {
        self.failure = failure;
        self
    }

    /// The italic caution under the fields.
    pub fn note(mut self, note: impl Into<SharedString>) -> Self {
        self.note = Some(note.into());
        self
    }
}

impl RenderOnce for TagFieldGrid {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .gap_3()
            .children(self.rows)
            .when_some(self.failure, |body, reason| {
                body.child(ErrorBanner::new(reason))
            })
            .children(self.note.map(|note| {
                div()
                    .text_xs()
                    .italic()
                    .text_color(style::white(style::TEXT_DIM, cx))
                    .child(note)
            }))
    }
}
