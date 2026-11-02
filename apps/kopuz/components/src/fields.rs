//! The controls a form row is made of, each usable on its own: radios, a choice with a custom entry, a secret, a path, and the frame around them.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputContentType, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled, Window, div, rems,
};

use crate::folders::pick_paths;
use crate::format::t;
use crate::states::FieldProblem;
use crate::{icons, motion, style};

/// `text-white/50`, the path a path field shows.
const PATH: f32 = 0.50;

type OnValue = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// One option of a radio group: the dot says which, and the whole line is the target.
#[derive(IntoElement)]
pub struct RadioGroup {
    id: SharedString,
    chosen: String,
    options: Vec<(String, SharedString)>,
    locked: bool,
    on_change: Option<OnValue>,
}

impl RadioGroup {
    pub fn new(id: impl Into<SharedString>, chosen: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            chosen: chosen.into(),
            options: Vec::new(),
            locked: false,
            on_change: None,
        }
    }

    pub fn option(mut self, value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        self.options.push((value.into(), label.into()));
        self
    }

    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    pub fn on_change(mut self, change: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(change));
        self
    }
}

impl RenderOnce for RadioGroup {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let accent = cx.theme().primary;
        let locked = self.locked;
        let group = self.id;
        v_flex()
            .gap_1p5()
            .children(self.options.into_iter().map(|(value, label)| {
                let chosen = value == self.chosen;
                let dot = div()
                    .size_3p5()
                    .flex_shrink_0()
                    .rounded_full()
                    .border_1()
                    .border_color(if chosen {
                        accent
                    } else {
                        style::white(style::BORDER, cx)
                    })
                    .when(chosen, |ring| {
                        ring.child(
                            div()
                                .size_full()
                                .p_0p5()
                                .child(div().size_full().rounded_full().bg(accent)),
                        )
                    });
                let change = self.on_change.clone();
                h_flex()
                    .id(SharedString::from(format!("radio-{group}-{value}")))
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .text_color(style::white(style::TEXT_STRONG, cx))
                    .when(!locked, |row| row.cursor_pointer())
                    .child(dot)
                    .child(label)
                    .when(!locked, |row| {
                        row.on_click(move |_, window, cx| {
                            if let Some(change) = &change {
                                change(&value, window, cx);
                            }
                        })
                    })
            }))
    }
}

type OnStart = Rc<dyn Fn(&mut Window, &mut App)>;

/// A dropdown of listed answers, and optionally a "type your own" entry that reveals a text box.
#[derive(IntoElement)]
pub struct ComboChoice {
    id: SharedString,
    value: String,
    options: Vec<(String, SharedString)>,
    custom: Option<Entity<InputState>>,
    locked: bool,
    on_pick: Option<OnValue>,
    on_custom: Option<OnStart>,
}

impl ComboChoice {
    pub fn new(id: impl Into<SharedString>, value: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            value: value.into(),
            options: Vec::new(),
            custom: None,
            locked: false,
            on_pick: None,
            on_custom: None,
        }
    }

    pub fn option(mut self, value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        self.options.push((value.into(), label.into()));
        self
    }

    /// Allow an answer off the list, typed into this box.
    pub fn custom(mut self, input: Entity<InputState>) -> Self {
        self.custom = Some(input);
        self
    }

    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    pub fn on_pick(mut self, pick: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(pick));
        self
    }

    /// The custom entry was chosen: the owner empties and focuses the box.
    pub fn on_custom(mut self, custom: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_custom = Some(Rc::new(custom));
        self
    }
}

impl RenderOnce for ComboChoice {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let listed = self.options.iter().any(|(value, _)| value == &self.value);
        let typed = self.custom.is_some() && !listed;
        let label = self
            .options
            .iter()
            .find(|(value, _)| value == &self.value)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| match typed {
                true => t("custom_manual"),
                false => t("choose"),
            });
        let allow_custom = self.custom.is_some();
        let (options, pick, start) = (self.options, self.on_pick, self.on_custom);
        v_flex()
            .gap_2()
            .child(
                Button::new(SharedString::from(format!("choice-{}", self.id)))
                    .outline()
                    .label(label)
                    .disabled(self.locked)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (value, label) in &options {
                            let (value, pick) = (value.clone(), pick.clone());
                            menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                                move |_, window, cx| {
                                    if let Some(pick) = &pick {
                                        pick(&value, window, cx);
                                    }
                                },
                            ));
                        }
                        if allow_custom {
                            let start = start.clone();
                            menu = menu.item(PopupMenuItem::new(t("custom_manual")).on_click(
                                move |_, window, cx| {
                                    if let Some(start) = &start {
                                        start(window, cx);
                                    }
                                },
                            ));
                        }
                        menu
                    }),
            )
            .children(
                self.custom
                    .filter(|_| typed)
                    .map(|input| Input::new(&input).disabled(self.locked)),
            )
    }
}

/// A write-only box: masked, with a tag saying a value is already held.
#[derive(IntoElement)]
pub struct SecretInput {
    input: Entity<InputState>,
    saved: bool,
    locked: bool,
}

impl SecretInput {
    pub fn new(input: &Entity<InputState>) -> Self {
        Self {
            input: input.clone(),
            saved: false,
            locked: false,
        }
    }

    /// The daemon already holds one, so a blank box reads as kept rather than unset.
    pub fn saved(mut self, saved: bool) -> Self {
        self.saved = saved;
        self
    }

    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }
}

impl RenderOnce for SecretInput {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_center()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.input)
                        .disabled(self.locked)
                        .content_type(InputContentType::Password)
                        .mask_toggle(),
                ),
            )
            .when(self.saved, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(style::text_10())
                        .text_color(style::white(style::TEXT_MUTED, cx))
                        .child(t("secret_saved")),
                )
            })
    }
}

/// What a path field picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PathKind {
    File,
    #[default]
    Directory,
}

type OnPath = Rc<dyn Fn(String, &mut App)>;

/// A path chosen with the platform picker: shown as text, or typed into a box, with a button to browse.
#[derive(IntoElement)]
pub struct PathField {
    id: SharedString,
    value: SharedString,
    kind: PathKind,
    input: Option<Entity<InputState>>,
    choose: Option<SharedString>,
    removable: bool,
    locked: bool,
    on_change: Option<OnPath>,
}

impl PathField {
    pub fn new(id: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            value: value.into(),
            kind: PathKind::Directory,
            input: None,
            choose: None,
            removable: false,
            locked: false,
            on_change: None,
        }
    }

    pub fn kind(mut self, kind: PathKind) -> Self {
        self.kind = kind;
        self
    }

    /// Edit the path by typing as well as browsing.
    pub fn input(mut self, input: &Entity<InputState>) -> Self {
        self.input = Some(input.clone());
        self
    }

    /// Words on the browse button; without them it is the folder glyph alone.
    pub fn choose(mut self, label: impl Into<SharedString>) -> Self {
        self.choose = Some(label.into());
        self
    }

    /// Offer Remove while a path is set, which answers with an empty one.
    pub fn removable(mut self, removable: bool) -> Self {
        self.removable = removable;
        self
    }

    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    pub fn on_change(mut self, change: impl Fn(String, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(change));
        self
    }
}

impl RenderOnce for PathField {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (locked, kind) = (self.locked, self.kind);
        let typed = self.input.is_some();
        let set = !self.value.is_empty();
        let change = self.on_change;
        let browse = Button::new(SharedString::from(format!("{}-choose", self.id)))
            .small()
            .icon(icons::icon(icons::FOLDER))
            .disabled(locked)
            .map(|button| match self.choose {
                Some(label) => button
                    .custom(style::soft_button(cx.theme().foreground, cx))
                    .label(label),
                None => button,
            })
            .on_click({
                let change = change.clone();
                move |_, _, cx| {
                    let change = change.clone();
                    let (files, directories) =
                        (kind == PathKind::File, kind == PathKind::Directory);
                    pick_paths(files, directories, false, cx, move |picked, cx| {
                        if let (Some(change), Some(path)) = (&change, picked.into_iter().next()) {
                            change(path, cx);
                        }
                    });
                }
            });
        h_flex()
            .gap_2()
            .items_center()
            .children(self.input.map(|input| {
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&input).disabled(locked))
            }))
            .when(!typed && set, |row| {
                row.child(
                    div()
                        .max_w(rems(13.75))
                        .truncate()
                        .text_xs()
                        .text_color(style::white(PATH, cx))
                        .child(self.value.clone()),
                )
            })
            .when(self.removable && set, |row| {
                row.child(
                    Button::new(SharedString::from(format!("{}-remove", self.id)))
                        .custom(style::soft_button(cx.theme().danger, cx))
                        .small()
                        .icon(icons::icon(icons::TRASH))
                        .label(t("remove"))
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            if let Some(change) = &change {
                                change(String::new(), cx);
                            }
                        }),
                )
            })
            .child(browse)
    }
}

/// The row a form control sits in: its label, a managed tag, help, a problem, and a wash under the pointer.
#[derive(IntoElement)]
pub struct FieldFrame {
    key: SharedString,
    label: SharedString,
    help: Option<SharedString>,
    problem: Option<SharedString>,
    locked: bool,
    lock_cause: Option<SharedString>,
    note: bool,
    control: Vec<AnyElement>,
}

impl FieldFrame {
    pub fn new(key: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            help: None,
            problem: None,
            locked: false,
            lock_cause: None,
            note: false,
            control: Vec::new(),
        }
    }

    pub fn help(mut self, help: Option<SharedString>) -> Self {
        self.help = help;
        self
    }

    pub fn problem(mut self, problem: Option<SharedString>) -> Self {
        self.problem = problem;
        self
    }

    /// A managed setting: tagged, and its control is already disabled by the owner.
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// What pins the field, cited in place of the generic tag.
    pub fn lock_cause(mut self, cause: Option<SharedString>) -> Self {
        self.lock_cause = cause;
        self
    }

    /// A note carries no answer: its whole row is the sentence, drawn where the label would be.
    pub fn note(mut self, note: bool) -> Self {
        self.note = note;
        self
    }
}

impl ParentElement for FieldFrame {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.control.extend(elements);
    }
}

impl RenderOnce for FieldFrame {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // `.settings-row` washes to white/5 under the pointer.
        let (over, pointer) = motion::hovered(motion::id("field-hover", &self.key), window, cx);
        let wash = motion::color(
            motion::id("field-wash", &self.key),
            style::white(if over { style::RAISED } else { 0. }, cx),
            window,
            cx,
        );
        let sentence = self
            .help
            .clone()
            .filter(|help| !help.is_empty())
            .unwrap_or_else(|| self.label.clone());
        v_flex()
            .id(SharedString::from(format!("field-{}", self.key)))
            .gap_1()
            .px_5()
            .py_2p5()
            .bg(wash)
            .on_hover(motion::track(pointer))
            .when(self.note, |field| {
                field.child(
                    div()
                        .text_xs()
                        .text_color(style::white(style::TEXT_SECONDARY, cx))
                        .child(sentence),
                )
            })
            .when(!self.note, |field| {
                field
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(style::white(style::TEXT_STRONG, cx))
                                    .child(self.label),
                            )
                            .when(self.locked, |row| {
                                let cause = self
                                    .lock_cause
                                    .clone()
                                    .unwrap_or_else(|| t("setting_managed_by_system"));
                                row.child(
                                    div()
                                        .text_size(style::text_10())
                                        .text_color(style::white(style::TEXT_SUBTLE, cx))
                                        .child(cause),
                                )
                            }),
                    )
                    .children(self.control)
                    .when_some(self.help, |field, help| {
                        field.child(
                            div()
                                .text_xs()
                                .text_color(style::white(style::TEXT_MUTED, cx))
                                .child(help),
                        )
                    })
            })
            .when_some(self.problem, |field, problem| {
                field.child(FieldProblem::new(problem))
            })
    }
}
