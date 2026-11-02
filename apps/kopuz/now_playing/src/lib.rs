//! One now-playing entity, drawn at three sizes: the bar, the sheet that grows out of it, and the mini window.

mod bar;
mod devices;
mod lyrics_view;
mod meta;
mod mini;
mod panel;
mod queue_view;
mod seek;
mod settings;
mod sheet;
mod swap;
mod transport;
mod volume;
mod window;

use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Global, IntoElement, Pixels,
    Render, Subscription, Task, Window,
};
use store::layout::SheetTab;
use store::{Chrome, ConfigModel, FavoritesModel, PlayerModel, QueueModel, SourcesModel};

pub use bar::{bar_height, utility_metrics};
pub use lyrics_view::LyricsView;
pub use queue_view::QueueView;
pub use settings::register as register_settings;
pub use window::{MiniWindow, close_mini, is_open as mini_is_open, open_mini, toggle_mini};

use seek::{Layout, PlayheadClock, SeekRow};

const VOLUME_STEP: f32 = 0.001;

#[cfg(test)]
mod playhead_tests;

/// Which of the sizes one entity is drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// The bar across the bottom, always there.
    Bar,
    SheetBar,
    /// The sheet the bar grows into, over the page.
    Sheet,
    /// The same queue and lyrics, docked beside the content instead.
    Panel,
    /// The ~380×148 window that floats over everything else.
    Mini,
}

/// Where the bar's cover sits on screen, so the sheet's hero can travel out of it.
#[derive(Default)]
struct CoverSlot(Option<Bounds<Pixels>>);

impl Global for CoverSlot {}

fn remember_cover_slot(bounds: Bounds<Pixels>, cx: &mut App) {
    cx.default_global::<CoverSlot>().0 = Some(bounds);
}

fn cover_slot(cx: &App) -> Option<Bounds<Pixels>> {
    cx.try_global::<CoverSlot>().and_then(|slot| slot.0)
}

/// The player, in whichever size it was built for. Every part is written once and drawn by each.
pub struct NowPlaying {
    presentation: Presentation,
    sheet_focus: sheet::ModalFocus,
    /// Follows the playhead on its own, so a tick never rebuilds this entity.
    seek: Option<Entity<SeekRow>>,
    clock: Option<Entity<PlayheadClock>>,
    volume: Entity<SliderState>,
    /// Whether the pointer is holding the level, which the daemon's own echo must not undo.
    volume_held: bool,
    tab: SheetTab,
    /// What the docked column has been dragged to, once it has been.
    panel_width: Option<Pixels>,
    /// Where the sheet and its hero cover landed last frame, so the cover can travel into it.
    root: Option<Bounds<Pixels>>,
    hero: Option<Bounds<Pixels>>,
    /// The track this surface last drew, and the one still fading out under it.
    swap: swap::TrackSwap,
    /// The sheet's own fade, so it can fold away before the shell takes it down.
    sheet_fade: components::motion::Fade,
    /// The tab body drawn last and the one it is crossfading from.
    tab_drawn: Option<SheetTab>,
    tab_leaving: Option<SheetTab>,
    tab_fade: components::motion::Fade,
    devices_open: bool,
    devices: Vec<api::ExternalDevice>,
    queue: Option<Entity<QueueView>>,
    lyrics: Option<Entity<LyricsView>>,
    sheet_bar: Option<Entity<NowPlaying>>,
    // Around the mini restore button, so keyboard focus shows it as hover does.
    restore_focus: FocusHandle,
    _devices_fetch: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl NowPlaying {
    /// The bar's size preset; every other surface keeps its own measures.
    fn scale(&self, cx: &App) -> f32 {
        match self.presentation {
            Presentation::Bar => store::layout(cx).player.size.scale(),
            _ => 1.,
        }
    }

    /// The bar across the bottom of the window.
    pub fn bar(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(Presentation::Bar, window, cx)
    }

    /// The sheet that grows out of the bar, over the page.
    pub fn sheet(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(Presentation::Sheet, window, cx)
    }

    /// The docked queue-and-lyrics column beside the content.
    pub fn panel(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(Presentation::Panel, window, cx)
    }

    /// The compact surface the mini window draws.
    pub fn mini(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(Presentation::Mini, window, cx)
    }

    fn new(presentation: Presentation, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let level = PlayerModel::global(cx).read(cx).volume();
        let volume = cx.new(|_| fraction_state(VOLUME_STEP, level));
        let holds_tabs = matches!(presentation, Presentation::Sheet | Presentation::Panel);
        let mini = presentation == Presentation::Mini;
        let seek = (presentation != Presentation::Panel).then(|| {
            let layout = if mini {
                Layout::BarOnly
            } else {
                Layout::Clocked
            };
            let scaled = presentation == Presentation::Bar;
            cx.new(|cx| SeekRow::new(layout, scaled, window, cx))
        });
        let clock = seek
            .as_ref()
            .filter(|_| mini)
            .map(|seek| cx.new(|cx| PlayheadClock::new(seek, cx)));

        let subscriptions = vec![
            cx.subscribe_in(&volume, window, Self::on_volume),
            cx.observe_in(&PlayerModel::global(cx), window, |this, _, window, cx| {
                this.sync(window, cx);
            }),
            cx.observe_in(&Chrome::global(cx), window, |this, chrome, window, cx| {
                if this.presentation == Presentation::Sheet {
                    let open = chrome.read(cx).is_sheet_open();
                    // Forgetting the landing makes the cover head for the bar now and fly out of it on every open.
                    if !open {
                        this.hero = None;
                    }
                    this.sheet_focus.set_open(open, window, cx);
                }
                cx.notify();
            }),
            cx.observe(&QueueModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&FavoritesModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&ConfigModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&store::LayoutModel::global(cx), |this, _, cx| {
                this.tab = store::layout(cx).sheet.tab;
                cx.notify();
            }),
            cx.observe(&SourcesModel::global(cx), |this, _, cx| {
                this.forget_devices(cx);
                cx.notify();
            }),
        ];

        Self {
            presentation,
            sheet_focus: sheet::ModalFocus::new(cx),
            seek,
            clock,
            volume,
            volume_held: false,
            tab: store::layout(cx).sheet.tab,
            panel_width: None,
            root: None,
            hero: None,
            sheet_fade: components::motion::Fade::hidden(),
            tab_drawn: None,
            tab_leaving: None,
            tab_fade: components::motion::Fade::shown(),
            swap: {
                let mut swap = swap::TrackSwap::default();
                swap.follow(PlayerModel::global(cx).read(cx).track());
                swap
            },
            devices_open: false,
            devices: Vec::new(),
            queue: holds_tabs.then(|| cx.new(|cx| QueueView::new(window, cx))),
            lyrics: holds_tabs.then(|| cx.new(|cx| LyricsView::new(window, cx))),
            sheet_bar: (presentation == Presentation::Sheet)
                .then(|| cx.new(|cx| Self::new(Presentation::SheetBar, window, cx))),
            restore_focus: cx.focus_handle(),
            _devices_fetch: None,
            _subscriptions: subscriptions,
        }
    }

    /// Carry the daemon's level into the slider the pointer is not holding.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let playing = PlayerModel::global(cx).read(cx).track().cloned();
        self.swap.follow(playing.as_ref());
        if !self.volume_held {
            let shown = PlayerModel::global(cx).read(cx).volume();
            self.volume
                .update(cx, |state, cx| state.set_value(shown, window, cx));
        }
        cx.notify();
    }

    /// Whether the sheet is still on screen, opening, open or folding away.
    pub fn sheet_visible(&self) -> bool {
        self.sheet_fade.is_visible()
    }

    fn track(&self, cx: &App) -> Option<api::TrackInfo> {
        PlayerModel::global(cx).read(cx).track().cloned()
    }
}

/// A slider over the 0..1 fraction of something, since a `SliderState`'s range is fixed at build time.
fn fraction_state(step: f32, start: f32) -> SliderState {
    components::settings::slider_state(0., 1., step, start)
}

fn played_fraction(player: &PlayerModel) -> f32 {
    match track_duration_ms(player) {
        Some(total) => (player.position_ms() as f64 / total as f64).clamp(0., 1.) as f32,
        None => 0.,
    }
}

/// A playable length, so radio (which reports none) and a stopped player both answer `None`.
fn track_duration_ms(player: &PlayerModel) -> Option<u64> {
    player
        .duration_ms()
        .filter(|total| *total > 0 && *total != u64::MAX)
}

impl Render for NowPlaying {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("now_playing");
        match self.presentation {
            Presentation::Bar | Presentation::SheetBar => self.render_bar(window, cx),
            Presentation::Sheet => self.render_sheet(window, cx),
            Presentation::Panel => self.render_panel(window, cx),
            Presentation::Mini => self.render_mini(window, cx),
        }
    }
}

impl NowPlaying {
    fn on_volume(
        &mut self,
        _: &Entity<SliderState>,
        event: &SliderEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = match event {
            SliderEvent::Change(value) => {
                self.volume_held = true;
                value.end()
            }
            SliderEvent::Release(value) => {
                self.volume_held = false;
                value.end()
            }
        };
        PlayerModel::global(cx).update(cx, |player, cx| player.set_volume(shown, cx));
    }
}

fn seekable_duration_ms(cx: &App) -> Option<u64> {
    let player = PlayerModel::global(cx);
    let player = player.read(cx);
    if player.is_loading() {
        return None;
    }
    track_duration_ms(player)
}
