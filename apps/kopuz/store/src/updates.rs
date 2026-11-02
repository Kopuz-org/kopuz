use api::{ApiEvent, UpdateStatus};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;

use crate::config::ConfigModel;

static CURRENT: UpdateStatus = UpdateStatus::Current;

/// What the daemon's release check found, and the install this app asked it for.
pub struct UpdateModel {
    status: UpdateStatus,
    // Only the app that asked quits for the update; any other client keeps its window.
    asked: bool,
    _read: Option<Task<()>>,
    _install: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl UpdateModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(
                &Link::connection(cx),
                |this, _, event: &ApiEvent, cx| match event {
                    ApiEvent::Update(status) => this.adopt(status.clone(), cx),
                    ApiEvent::Resync => this.read(cx),
                    _ => {}
                },
            ),
            cx.observe(&ConfigModel::global(cx), |_, _, cx| crate::notify(cx)),
        ];
        Self {
            status: UpdateStatus::Current,
            asked: false,
            _read: None,
            _install: None,
            _subscriptions: subscriptions,
        }
    }

    /// What to say now: nothing while checks are off, unless an install is already under way.
    pub fn status(&self, cx: &App) -> &UpdateStatus {
        let checking = ConfigModel::global(cx)
            .read(cx)
            .config()
            .system
            .auto_check_updates;
        match &self.status {
            UpdateStatus::Downloading { .. } | UpdateStatus::Ready { .. } => &self.status,
            _ if checking => &self.status,
            _ => &CURRENT,
        }
    }

    /// Download and stage the update; the app quits once it is staged, and the daemon starts it again.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        self.asked = true;
        let pid = std::process::id();
        let call =
            Link::global(cx).call(move |api| async move { api.install_update(Some(pid)).await });
        self._install = Some(cx.spawn(async move |this, cx| {
            if let Err(error) = call.await {
                tracing::warn!(%error, "the daemon would not install the update");
                this.update(cx, |this, cx| this.read(cx)).ok();
            }
        }));
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.update_status().await });
        self._read = Some(cx.spawn(async move |this, cx| {
            // An older daemon, or one inside the legacy app, has no updater: nothing to say.
            let status = call.await.unwrap_or_default();
            this.update(cx, |this, cx| this.adopt(status, cx)).ok();
        }));
    }

    fn adopt(&mut self, status: UpdateStatus, cx: &mut Context<Self>) {
        let ready = matches!(status, UpdateStatus::Ready { .. });
        self.status = status;
        crate::notify(cx);
        if ready && self.asked {
            cx.quit();
        }
    }
}
