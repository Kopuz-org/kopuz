//! A library sync's status line: how far along it is, or how much it brought in, with a refresh.

use std::rc::Rc;

use api::JobKind;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    Window, px,
};
use store::JobsModel;

use crate::format::t;
use crate::{icons, style};

type OnRefresh = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The status of one kind of sync job, read from the jobs the daemon reports, and the button that restarts it.
#[derive(IntoElement)]
pub struct SyncStatus {
    kind: JobKind,
    total: usize,
    on_refresh: Option<OnRefresh>,
}

impl SyncStatus {
    pub fn new(kind: JobKind) -> Self {
        Self {
            kind,
            total: 0,
            on_refresh: None,
        }
    }

    /// How many items there are to sync.
    pub fn total(mut self, total: usize) -> Self {
        self.total = total;
        self
    }

    pub fn on_refresh(
        mut self,
        refresh: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_refresh = Some(Rc::new(refresh));
        self
    }
}

impl RenderOnce for SyncStatus {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let jobs = JobsModel::global(cx);
        let progress = jobs.read(cx).progress(self.kind);
        let syncing = progress.is_some();
        let done = progress.and_then(|job| job.current).unwrap_or(0) as usize;
        let total = self.total;
        let label = store::kind_label(self.kind);
        let refresh = self.on_refresh;
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .gap_3()
            .px_2()
            .text_xs()
            .text_color(style::slate_400(cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(syncing, |row| {
                        row.child(Spinner::new().xsmall().color(cx.theme().primary_hover))
                            .child(SharedString::from(format!("{label} {done} / {total}")))
                    })
                    .when(!syncing && total > 0, |row| {
                        row.child(icons::icon(icons::CHECK).text_color(cx.theme().success))
                            .child(SharedString::from(format!("{label} · {total}")))
                    }),
            )
            .child(
                Button::new("refresh-sync")
                    .custom(style::soft_button(style::white(0.80, cx), cx))
                    .xsmall()
                    .px_3()
                    .py_1()
                    // `Button::rounded` is in pixels, so the Rems helper cannot travel here.
                    .rounded(px(8.))
                    .icon(icons::icon(icons::REFRESH))
                    .label(t("refresh"))
                    .disabled(syncing)
                    .on_click(move |event, window, cx| {
                        if let Some(refresh) = &refresh {
                            refresh(event, window, cx);
                        }
                    }),
            )
    }
}
