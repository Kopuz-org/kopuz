//! The dialog shell every form uses, and the confirmation that guards what cannot be taken back.

use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogButtonProps, DialogClose, DialogFooter};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyView, App, ParentElement as _, Pixels, SharedString, Styled as _, Window, div, px,
};

use crate::format::t;

/// Ask for a rename through whichever feature registered the prompt.
pub fn open_rename(
    title: impl Into<SharedString>,
    value: impl Into<SharedString>,
    save: impl Fn(String, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let rename = store::Rename {
        title: title.into(),
        value: value.into(),
        save: Rc::new(save),
    };
    store::RENAME.open(rename, window, cx);
}

/// What a confirmation is asking leave to do; it names the button that says yes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    /// Throw away edits that were never saved.
    Discard,
    /// Remove something for good; the description should name what goes.
    Delete,
    /// Empty something that refills, such as a cache; the description should say how much goes.
    Clear,
}

impl ConfirmKind {
    fn ok_text(self) -> SharedString {
        match self {
            ConfirmKind::Discard => t("discard_changes"),
            ConfirmKind::Delete => t("delete"),
            ConfirmKind::Clear => t("clear"),
        }
    }
}

/// Ask before something that cannot be taken back; `go` runs only when the answer is yes.
pub fn confirm(
    kind: ConfirmKind,
    title: SharedString,
    description: impl Into<SharedString>,
    go: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let description = description.into();
    let go = Rc::new(go);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let go = go.clone();
        alert
            .title(title.clone())
            .description(description.clone())
            .button_props(
                DialogButtonProps::default()
                    .show_cancel(true)
                    .ok_text(kind.ok_text())
                    .ok_variant(ButtonVariant::Danger)
                    .cancel_text(t("cancel")),
            )
            .on_ok(move |_, window, cx| {
                go(window, cx);
                true
            })
    });
}

/// How wide a dialog is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DialogSize {
    /// `w-80`: a one-field prompt or a short pick.
    Prompt,
    /// The dialog's own 448px: a short form.
    #[default]
    Form,
    /// `max-w-lg`: a form in sections, such as a source definition or a track's tags.
    Wide,
    /// A reference sheet that lays its contents out in columns.
    Sheet,
}

impl DialogSize {
    fn width(self) -> Pixels {
        match self {
            DialogSize::Prompt => px(320.),
            DialogSize::Form => px(448.),
            DialogSize::Wide => px(512.),
            DialogSize::Sheet => px(720.),
        }
    }
}

/// What sits under a dialog's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FormFooter {
    /// Nothing at all, for a dialog whose body ends flush.
    None,
    /// An empty band: the body carries its own buttons, and the dialog's OK and Cancel stay hidden.
    #[default]
    Empty,
    /// One Cancel that closes it.
    Cancel,
    /// Cancel, and a Save that submits.
    CancelSave,
}

type Submit = Rc<dyn Fn(&mut Window, &mut App)>;
type Read<T> = Rc<dyn Fn(&App) -> T>;

/// The one dialog opener: a title, a width, a body view, what submitting does, and the footer under it.
pub struct FormDialog {
    title: Read<SharedString>,
    size: DialogSize,
    closable: Read<bool>,
    footer: FormFooter,
    submit: Option<Submit>,
}

impl FormDialog {
    pub fn new(title: impl Into<SharedString>) -> Self {
        let title = title.into();
        Self {
            title: Rc::new(move |_| title.clone()),
            size: DialogSize::Form,
            closable: Rc::new(|_| false),
            footer: FormFooter::Empty,
            submit: None,
        }
    }

    /// A title read each frame, for a dialog whose title follows its body's state.
    pub fn title_with(mut self, title: impl Fn(&App) -> SharedString + 'static) -> Self {
        self.title = Rc::new(title);
        self
    }

    pub fn size(mut self, size: DialogSize) -> Self {
        self.size = size;
        self
    }

    /// Whether a click on the backdrop closes it; off by default, since a stray click must not drop typed input.
    pub fn closable(self, closable: bool) -> Self {
        self.closable_with(move |_| closable)
    }

    /// The same, read each frame.
    pub fn closable_with(mut self, closable: impl Fn(&App) -> bool + 'static) -> Self {
        self.closable = Rc::new(closable);
        self
    }

    pub fn footer(mut self, footer: FormFooter) -> Self {
        self.footer = footer;
        self
    }

    /// What Enter and Save do; the dialog stays open, so `submit` closes it once its work lands.
    pub fn on_submit(mut self, submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.submit = Some(Rc::new(submit));
        self
    }

    pub fn open(self, body: impl Into<AnyView>, window: &mut Window, cx: &mut App) {
        let body = body.into();
        let FormDialog {
            title,
            size,
            closable,
            footer,
            submit,
        } = self;
        window.open_dialog(cx, move |dialog, _, cx| {
            let dialog = dialog
                .title(title(cx))
                .w(size.width())
                .overlay_closable(closable(cx))
                .child(body.clone())
                .when_some(submit.clone(), |dialog, submit| {
                    dialog.on_ok(move |_, window, cx| {
                        submit(window, cx);
                        false
                    })
                });
            let cancel = || DialogClose::new().child(Button::new("cancel").label(t("cancel")));
            match footer {
                FormFooter::None => dialog,
                FormFooter::Empty => dialog.footer(div()),
                FormFooter::Cancel => dialog.footer(DialogFooter::new().child(cancel())),
                FormFooter::CancelSave => {
                    let submit = submit.clone();
                    dialog.footer(DialogFooter::new().gap_2().child(cancel()).child(
                        Button::new("save").primary().label(t("save")).on_click(
                            move |_, window, cx| {
                                if let Some(submit) = &submit {
                                    submit(window, cx);
                                }
                            },
                        ),
                    ))
                }
            }
        });
    }
}
