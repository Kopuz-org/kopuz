//! What the app is busy with: the bar's activity, downloads and notification popovers, the reconnect banner and toasts.

mod activity;
mod banner;
mod centre;
mod downloads;
mod loader;
mod logo;
mod network;
mod settings;
mod startup;
mod toasts;
mod update;
mod url_download;

use components::format::t;
use components::{Heading, SectionHeading, icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _,
    Pixels, Render, SharedString, Styled, Subscription, Window, div, px,
};
use store::{DownloadsModel, ItemState, JobsModel, Navigator, Notices};

pub use banner::ConnectionBanner;
pub use loader::Loader;
pub use logo::release as release_logo;
pub use network::NetworkBanner;
pub use settings::register as register_settings;
pub use startup::StartupError;
pub use toasts::Toasts;
pub use update::UpdateBanner;
pub use url_download::register as register_url_download;

/// Lucide names this crate needs that `components::icons` does not carry.
pub const BELL: &str = "icons/bell.svg";
pub const JOBS: &str = "icons/activity.svg";
pub const DONE: &str = "icons/circle-check.svg";
pub const FAILED: &str = "icons/circle-x.svg";
pub const RETRY: &str = "icons/rotate-cw.svg";

const GLYPH: Pixels = px(15.);
const POPOVER_WIDTH: Pixels = px(320.);
/// A list inside a popover scrolls rather than growing past the window.
pub(crate) const LIST_HEIGHT: Pixels = px(260.);
const BADGE: Pixels = px(14.);

/// Which popover of the three is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Activity,
    Downloads,
    Centre,
}

/// The bar's right-hand status cluster: jobs, downloads and the notification centre.
pub struct StatusCluster {
    open: Option<Which>,
    activity: Entity<activity::ActivityList>,
    downloads: Entity<downloads::DownloadsList>,
    centre: Entity<centre::NoticeCentre>,
    // Derived in observers: job progress streams, and render must not do work per tick.
    running: usize,
    pending: usize,
    failed: usize,
    unseen: usize,
    _subscriptions: Vec<Subscription>,
}

impl StatusCluster {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let jobs = JobsModel::global(cx);
        let downloads = DownloadsModel::global(cx);
        let notices = Notices::global(cx);
        let navigator = Navigator::global(cx);
        let list = cx.new(|cx| downloads::DownloadsList::new(window, cx));
        let subscriptions = vec![
            cx.observe(&jobs, |this, _, cx| this.recount(cx)),
            cx.observe(&downloads, |this, _, cx| this.recount(cx)),
            cx.observe(&notices, |this, _, cx| this.recount(cx)),
            cx.observe(&navigator, |this, _, cx| this.honour_intent(cx)),
            // A dialog over an open popover reads as two layers of chrome: the popover stands down.
            cx.subscribe(&list, |this, _, _: &downloads::Dismiss, cx| {
                this.set_open(None, cx)
            }),
        ];
        let mut this = Self {
            open: None,
            activity: cx.new(|cx| activity::ActivityList::new(window, cx)),
            downloads: list,
            centre: cx.new(|cx| centre::NoticeCentre::new(window, cx)),
            running: 0,
            pending: 0,
            failed: 0,
            unseen: 0,
            _subscriptions: subscriptions,
        };
        this.reread(cx);
        this.honour_intent(cx);
        this
    }

    fn reread(&mut self, cx: &App) -> bool {
        let running = JobsModel::global(cx).read(cx).running().len();
        let queue = DownloadsModel::global(cx).read(cx);
        let pending = queue
            .queue()
            .iter()
            .filter(|item| matches!(item.state, ItemState::Queued | ItemState::Downloading))
            .count();
        let failed = queue
            .queue()
            .iter()
            .filter(|item| item.state == ItemState::Failed)
            .count();
        let unseen = Notices::global(cx).read(cx).unseen();
        let changed = (running, pending, failed, unseen)
            != (self.running, self.pending, self.failed, self.unseen);
        self.running = running;
        self.pending = pending;
        self.failed = failed;
        self.unseen = unseen;
        changed
    }

    /// Redraw on a count, not on a progress tick: a running job reports several times a second.
    fn recount(&mut self, cx: &mut Context<Self>) {
        if self.reread(cx) {
            cx.notify();
        }
    }

    /// A route that is no longer a place still said what it wanted; this is where it lands.
    fn honour_intent(&mut self, cx: &mut Context<Self>) {
        let navigator = Navigator::global(cx);
        let wanted = match navigator.read(cx).intent() {
            Some("activity") => Some(Which::Activity),
            Some("downloads") => Some(Which::Downloads),
            _ => None,
        };
        let Some(which) = wanted else { return };
        navigator.update(cx, |navigator, _| navigator.take_intent());
        self.set_open(Some(which), cx);
    }

    /// Open the notification centre, for a command with no route to redirect through.
    pub fn show_notifications(&mut self, cx: &mut Context<Self>) {
        self.set_open(Some(Which::Centre), cx);
    }

    fn set_open(&mut self, open: Option<Which>, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if open == Some(Which::Downloads) {
            self.downloads.update(cx, |list, cx| list.opened(cx));
        }
        if open == Some(Which::Centre) {
            Notices::global(cx).update(cx, |notices, cx| notices.mark_seen(cx));
        }
        cx.notify();
    }

    fn trigger(
        &self,
        id: &'static str,
        glyph: &'static str,
        tip: SharedString,
        cx: &App,
    ) -> Button {
        Button::new(id)
            .custom(style::quiet_button(style::slate_400(cx), cx))
            .with_size(GLYPH * (4. / 3.))
            .size(style::chrome().button)
            .rounded(style::icon_corner(style::chrome().button))
            .icon(icons::icon(glyph))
            .tooltip(tip)
    }

    fn popover(
        &self,
        which: Which,
        id: &'static str,
        glyph: &'static str,
        tip: SharedString,
        body: gpui_kit::AnyView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.open == Some(which);
        Popover::new(id)
            .anchor(Anchor::TopRight)
            .p_0()
            .w(POPOVER_WIDTH)
            .rounded(style::radius_lg())
            .border_1()
            .border_color(style::white(style::BORDER, cx))
            .overflow_hidden()
            .open(open)
            .on_open_change(cx.listener(move |this, open: &bool, _, cx| {
                this.set_open(open.then_some(which), cx);
            }))
            .trigger(self.trigger(id, glyph, tip, cx))
            .content(move |_, _, _| div().w_full().child(body.clone()))
            .into_any_element()
    }

    /// The count over a trigger, so something happening is visible without opening anything.
    fn badge(&self, count: usize, accent: gpui_kit::Hsla, cx: &App) -> Option<AnyElement> {
        if count == 0 {
            return None;
        }
        let label = match count > 9 {
            true => SharedString::new_static("9+"),
            false => SharedString::from(count.to_string()),
        };
        Some(
            div()
                .absolute()
                .top(px(-2.))
                .right(px(-2.))
                .min_w(BADGE)
                .h(BADGE)
                .px_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(accent)
                .text_size(style::text_9())
                .text_color(cx.theme().background)
                .child(label)
                .into_any_element(),
        )
    }
}

impl Render for StatusCluster {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("status-cluster");
        let jobs = self.popover(
            Which::Activity,
            "status-activity",
            JOBS,
            t("activity"),
            self.activity.clone().into(),
            cx,
        );
        let downloads = self.popover(
            Which::Downloads,
            "status-downloads",
            icons::DOWNLOAD,
            t("downloads"),
            self.downloads.clone().into(),
            cx,
        );
        let centre = self.popover(
            Which::Centre,
            "status-notifications",
            BELL,
            t("notifications"),
            self.centre.clone().into(),
            cx,
        );
        let queued = self.badge(self.pending, cx.theme().primary, cx);
        let failed = self
            .badge(self.failed, cx.theme().danger, cx)
            .filter(|_| self.pending == 0);
        h_flex()
            .gap_0p5()
            .flex_shrink_0()
            .items_center()
            .child(div().relative().child(jobs).children(self.badge(
                self.running,
                cx.theme().warning,
                cx,
            )))
            .child(
                div()
                    .relative()
                    .child(downloads)
                    .children(queued)
                    .children(failed),
            )
            .child(div().relative().child(centre).children(self.badge(
                self.unseen,
                cx.theme().primary,
                cx,
            )))
    }
}

/// A popover's own heading, with whatever action belongs beside it.
pub(crate) fn header(title: SharedString, action: Option<AnyElement>, cx: &App) -> AnyElement {
    let mut heading = SectionHeading::new(title).heading(Heading::Label);
    if let Some(action) = action {
        heading = heading.trailing(action);
    }
    div()
        .w_full()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(style::white(style::BORDER, cx))
        .child(heading)
        .into_any_element()
}

/// Let a popover be opened from anywhere: a redirected route is what carries the request.
pub fn open_downloads(cx: &mut App) {
    Navigator::navigate(store::Route::Downloader, cx);
}

pub fn open_activity(cx: &mut App) {
    Navigator::navigate(store::Route::Activity, cx);
}
