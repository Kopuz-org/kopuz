//! Toasts: which notices interrupt, how long they stay, and what a batch says when it ends.

use std::collections::HashMap;
use std::time::Duration;

use api::NoticeLevel;
use components::format::{t, t_with};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::{
    App, Context, ElementId, IntoElement, Render, SharedString, Subscription, Task, Window, div,
};
use store::{BatchFinished, DownloadsModel, Notice, Notices, Retracted, Undo};

/// How long an undo stays offered.
const UNDO: Duration = Duration::from_secs(6);

/// The type the toast ids are scoped by, so a message replaces its own earlier toast.
struct Toast;

/// Turns notices into toasts, honouring `layout.notifications`. It draws nothing itself.
pub struct Toasts {
    _undos: HashMap<SharedString, Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Toasts {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let notices = Notices::global(cx);
        let downloads = DownloadsModel::global(cx);
        let subscriptions = vec![
            cx.subscribe_in(&notices, window, |this, _, notice: &Notice, window, cx| {
                this.show(notice, window, cx);
            }),
            cx.subscribe_in(&notices, window, |_, _, gone: &Retracted, window, cx| {
                for id in &gone.0 {
                    window.remove_notification1::<Toast>(ElementId::from(id.clone()), cx);
                }
            }),
            cx.subscribe_in(
                &downloads,
                window,
                |this, _, batch: &BatchFinished, window, cx| {
                    this.summarize(*batch, window, cx);
                },
            ),
        ];
        Self {
            _undos: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Whether this notice interrupts, or only lands in the centre behind the bell.
    fn wanted(notice: &Notice, cx: &App) -> bool {
        let notifications = &store::layout(cx).notifications;
        if let Some(kind) = notice.kind.as_ref()
            && !notifications.toasts(kind)
        {
            return false;
        }
        notice.level != NoticeLevel::Error || notifications.errors
    }

    fn show(&mut self, notice: &Notice, window: &mut Window, cx: &mut Context<Self>) {
        if !Self::wanted(notice, cx) {
            return;
        }
        let id = notice.id();
        let message = notice.message.clone();
        let toast = match notice.level {
            // An error that fades in five seconds is one nobody read.
            NoticeLevel::Error => Notification::error(message).autohide(false),
            NoticeLevel::Warning => Notification::warning(message),
            _ => Notification::info(message),
        };
        let toast = toast.id1::<Toast>(ElementId::from(id.clone()));
        match notice.undo.clone() {
            Some(undo) => self.offer_undo(toast, undo, id, window, cx),
            None => window.push_notification(toast, cx),
        }
    }

    /// An undoable step: the same toast, with a button, and gone again six seconds later.
    fn offer_undo(
        &mut self,
        toast: Notification,
        undo: Undo,
        id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = ElementId::from(id.clone());
        let toast = toast.action(move |_, _, _| {
            let undo = undo.clone();
            let key = key.clone();
            Button::new("undo")
                .ghost()
                .xsmall()
                .label(undo.label.clone())
                .on_click(move |_, window, cx| {
                    undo.run(cx);
                    window.remove_notification1::<Toast>(key.clone(), cx);
                })
        });
        window.push_notification(toast, cx);
        let key = ElementId::from(id.clone());
        let held = id.clone();
        let expiry = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(UNDO).await;
            cx.update(|window, cx| window.remove_notification1::<Toast>(key.clone(), cx))
                .ok();
            this.update(cx, |this, _| this._undos.remove(&held)).ok();
        });
        self._undos.insert(id, expiry);
    }

    /// A batch that has ended is worth one line, with a way back to the rows it left.
    fn summarize(&mut self, batch: BatchFinished, window: &mut Window, cx: &mut Context<Self>) {
        let summary = t_with(
            "downloads_summary",
            &[
                ("done", batch.downloaded.into()),
                ("failed", batch.failed.into()),
            ],
        );
        let toast = Notification::info(summary)
            .id1::<Toast>(ElementId::from(SharedString::new_static("downloads-batch")))
            .action(|_, _, _| {
                Button::new("view-downloads")
                    .ghost()
                    .xsmall()
                    .label(t("view"))
                    .on_click(|_, _, cx| crate::open_downloads(cx))
            });
        window.push_notification(toast, cx);
    }
}

impl Render for Toasts {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
