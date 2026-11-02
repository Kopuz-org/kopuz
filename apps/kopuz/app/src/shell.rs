//! The window: the top bar, the routed page, the panels, the player bar, and everything over them.

use components::motion;
use gpui_kit::component::{ActiveTheme as _, Root, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled,
    Subscription, Task, Window, div, px,
};
use gpui_kit::{FocusHandle, InteractiveElement as _, actions};
use link::Connection;
use store::{Chrome, ConfigModel, Navigator, PlayerModel, SourcesModel};

use crate::router::Router;

/// The gap a toast keeps above whatever runs along the window's bottom edge.
const TOAST_GAP: gpui_kit::Pixels = gpui_kit::px(16.);
/// Material's surface-container-low: the text colour at 4% over the window.
const PANE_TINT: f32 = 0.04;

/// How dark the scrim under a dialog gets.
const MODAL_SCRIM: f32 = 0.8;

/// Toasts rise from the bottom edge, so they clear the player bar only while it sits there.
fn place_toasts(cx: &mut gpui_kit::App) {
    let at_top = ConfigModel::global(cx)
        .read(cx)
        .config()
        .ui
        .player_bar_position
        == config::PlayerBarPosition::Top;
    let clearance = match at_top {
        true => TOAST_GAP,
        false => {
            now_playing::bar_height(cx) + components::style::chrome().pane_gap * 2. + TOAST_GAP
        }
    };
    theme::place_toasts(clearance, cx);
}

type AppliedTheme = (String, Option<config::CustomTheme>);

/// Render what the daemon's settings say, whichever layer set them; `applied` keeps a repaint from reapplying the theme.
fn follow_config(applied: &mut AppliedTheme, window: &mut Window, cx: &mut gpui_kit::App) {
    let settings = ConfigModel::global(cx).read(cx).config().clone();
    // Every transition in the app asks GPUI whether motion is reduced.
    let reduced = settings.ui.reduce_animations;
    if reduced != cx.reduce_motion() {
        cx.set_reduce_motion(reduced);
    }
    // Re-editing a custom theme keeps its name, so the colors are the key.
    let wanted = (
        settings.ui.theme.clone(),
        settings.ui.custom_themes.get(&settings.ui.theme).cloned(),
    );
    if wanted != *applied {
        *applied = wanted;
        theme::apply_config(&settings.ui, Some(window), cx);
    }
    place_toasts(cx);
}

/// What a page's contents depend on in the source list; a probe's answer changes none of it.
#[derive(Debug, Clone, PartialEq, Default)]
struct SourceIdentity {
    active: Option<String>,
    capabilities: api::SourceCapabilities,
    offline_reads: bool,
}

impl SourceIdentity {
    fn of(sources: &SourcesModel) -> Self {
        Self {
            active: sources.active().map(|source| source.id.clone()),
            capabilities: sources.capabilities(),
            offline_reads: sources.offline_reads(),
        }
    }
}

actions!(
    kopuz,
    [
        TogglePlayback,
        QuickSearch,
        Escape,
        Back,
        Forward,
        TogglePlaylistsPanel
    ]
);

pub struct Shell {
    router: Router,
    focus: FocusHandle,
    connection: Entity<Connection>,
    topbar: Entity<topbar::TopBar>,
    playlists_panel: Entity<playlists_panel::PlaylistsPanel>,
    player_bar: Entity<now_playing::NowPlaying>,
    side_panel: Entity<now_playing::NowPlaying>,
    fullscreen: Entity<now_playing::NowPlaying>,
    quick_search: Entity<palette::Palette>,
    mini_player: Entity<now_playing::NowPlaying>,
    overlays: Entity<overlays::Overlays>,
    first_run: Entity<onboarding::FirstRun>,
    // Up until the daemon first answers and setup's state is read; never again after.
    loader: Option<Entity<status::Loader>>,
    debug_overlay: Entity<debug_overlay::DebugOverlay>,
    // The theme these were applied for, so config edits reapply only on a change.
    applied_theme: AppliedTheme,
    system_artwork: Option<api::ArtworkRef>,
    system_ease: crate::system_ease::SystemEase,
    // The route last drawn and the incoming page's fade, kept here so a cached page fades like a new one.
    page_key: String,
    page_fade: motion::Fade,
    page_leaving: Option<gpui_kit::AnyView>,
    dock_fade: motion::Fade,
    // The modal scrim, fading in under a dialog and out after it closes.
    scrim_fade: motion::Fade,
    // What the active source answers to, so only a real change throws the pages away.
    source: SourceIdentity,
    // The page drawn last, and the one a source switch is handing over from.
    shown: Option<gpui_kit::AnyView>,
    handover: Option<crate::handover::Handover>,
    // On its own timer: GPUI only renders on a change, and an idle app is when covers want releasing.
    _heartbeat: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    /// Reports the heap and releases the covers no longer drawn, whether or not anything repainted.
    fn heartbeat(window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            let mut heartbeat = profile::Heartbeat::new();
            loop {
                cx.background_executor().timer(profile::HEARTBEAT).await;
                let swept = this.update_in(cx, |_, window, cx| {
                    heartbeat.report();
                    store::Residents::sweep(window, cx)
                });
                if swept.is_err() {
                    return;
                }
            }
        })
    }

    pub fn new(started: std::time::Instant, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let connection = link::Link::connection(cx);
        let navigator = Navigator::global(cx);
        let chrome = Chrome::global(cx);
        let config = ConfigModel::global(cx);
        let sources = SourcesModel::global(cx);
        let first_run = cx.new(|cx| onboarding::FirstRun::new(window, cx));
        let booted = connection.read(cx).is_connected()
            && store::SetupModel::global(cx).read(cx).is_decided();

        let subscriptions = vec![
            // Unmounted, the guide's own notify has no ancestor to dirty, so it would never appear.
            cx.observe(&first_run, |_, _, cx| cx.notify()),
            cx.observe(&store::SetupModel::global(cx), |this, _, cx| this.boot(cx)),
            cx.observe(&connection, |this, _, cx| this.boot(cx)),
            cx.observe(&navigator, |_, _, cx| cx.notify()),
            cx.observe(&chrome, |_, _, cx| cx.notify()),
            // System colors follows the cover; the player ticks constantly, so only a new cover redraws.
            cx.observe(&PlayerModel::global(cx), |this, player, cx| {
                if this.applied_theme.0 != theme::SYSTEM {
                    return;
                }
                let artwork = player
                    .read(cx)
                    .track()
                    .and_then(|track| track.artwork.clone());
                if artwork != this.system_artwork {
                    this.system_artwork = artwork;
                    cx.notify();
                }
            }),
            // Pulled before the skip, so the cover's colours are in hand on the frame its track starts.
            cx.observe(&store::QueueModel::global(cx), |_, queue, cx| {
                let neighbours: Vec<api::ArtworkRef> = {
                    let queue = queue.read(cx);
                    let Some(at) = queue.position().map(|at| at as usize) else {
                        return;
                    };
                    [at.checked_sub(1), Some(at), at.checked_add(1)]
                        .into_iter()
                        .flatten()
                        .filter_map(|index| queue.items().get(index)?.artwork.clone())
                        .collect()
                };
                for artwork in &neighbours {
                    store::palette::warm(artwork, cx);
                }
            }),
            cx.observe_in(&config, window, |this, _, window, cx| {
                follow_config(&mut this.applied_theme, window, cx);
                cx.notify();
            }),
            // The skin sizes the player bar, which the toasts have to clear.
            cx.observe(&store::LayoutModel::global(cx), |_, _, cx| place_toasts(cx)),
            // Held from the ask, not the answer: the queue and the mix change before the source list does.
            cx.subscribe(&sources, |this, _, _: &store::SwitchStarted, cx| {
                this.hand_over(true, cx);
                cx.notify();
            }),
            // A source switch makes every page's contents wrong, so none are kept; the shown one is held.
            cx.observe(&sources, |this, sources, cx| {
                let now = SourceIdentity::of(sources.read(cx));
                if now != this.source {
                    this.source = now;
                    // A second change mid-handover would hand over from the half-loaded page, so the first one is kept.
                    match this.handover.as_mut() {
                        Some(handover) => {
                            handover.restart();
                            this.router.clear();
                        }
                        None => this.hand_over(false, cx),
                    }
                }
                cx.notify();
            }),
        ];

        // The window opens after the config has usually loaded, so the observer above may never fire for it.
        let mut applied_theme = AppliedTheme::default();
        follow_config(&mut applied_theme, window, cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // Overlays owns the notices channel now, so the bar only has to hold the cluster.
        let topbar = cx.new(|cx| topbar::TopBar::new(window, cx));
        let cluster = cx.new(|cx| status::StatusCluster::new(window, cx));
        topbar.update(cx, |bar, cx| bar.set_activity(Some(cluster.into()), cx));
        Self {
            router: Router::default(),
            focus,
            connection,
            topbar,
            playlists_panel: cx.new(|cx| playlists_panel::PlaylistsPanel::new(window, cx)),
            player_bar: cx.new(|cx| now_playing::NowPlaying::bar(window, cx)),
            side_panel: cx.new(|cx| now_playing::NowPlaying::panel(window, cx)),
            fullscreen: cx.new(|cx| now_playing::NowPlaying::sheet(window, cx)),
            quick_search: cx.new(|cx| palette::Palette::new(window, cx)),
            mini_player: cx.new(|cx| now_playing::NowPlaying::mini(window, cx)),
            overlays: cx.new(|cx| overlays::Overlays::new(window, cx)),
            first_run,
            // The window often opens after both answers are in, when no observer would fire again.
            loader: (!booted).then(|| cx.new(|cx| status::Loader::new(started, cx))),
            debug_overlay: cx.new(|cx| debug_overlay::DebugOverlay::new(window, cx)),
            applied_theme,
            system_artwork: None,
            system_ease: Default::default(),
            page_key: String::new(),
            page_fade: motion::Fade::hidden(),
            page_leaving: None,
            dock_fade: motion::Fade::hidden(),
            scrim_fade: motion::Fade::hidden(),
            source: SourceIdentity::of(sources.read(cx)),
            shown: None,
            handover: None,
            _heartbeat: Self::heartbeat(window, cx),
            _subscriptions: subscriptions,
        }
    }

    /// The first answer is in: the loader goes for good, and a later disconnect is the banner's.
    fn boot(&mut self, cx: &mut Context<Self>) {
        let connected = self.connection.read(cx).is_connected();
        let decided = store::SetupModel::global(cx).read(cx).is_decided();
        if self.loader.is_some() && connected && decided {
            self.loader = None;
            status::release_logo(cx);
        }
        cx.notify();
    }

    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx));
    }

    pub fn toggle_quick_search(&mut self, cx: &mut Context<Self>) {
        Chrome::global(cx).update(cx, |chrome, cx| {
            let open = chrome.is_quick_search_open();
            chrome.set_quick_search(!open, cx);
        });
    }

    /// The tail of one chain: the palette, a menu, a dialog and a selection all stop Escape first.
    pub fn escape(&mut self, cx: &mut Context<Self>) {
        // The mini window is the innermost thing the Shell can see, and it is a window, not a flag.
        if now_playing::mini_is_open(cx) {
            now_playing::close_mini(cx);
            return;
        }
        Chrome::global(cx).update(cx, |chrome, cx| {
            if chrome.is_quick_search_open() {
                chrome.set_quick_search(false, cx);
            } else if chrome.is_mini() {
                chrome.set_mini(false, cx);
            } else if chrome.is_sheet_open() {
                chrome.set_sheet(false, cx);
            } else if chrome.panel().is_some() {
                chrome.set_panel(None, cx);
            }
        });
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        Navigator::global(cx).update(cx, |navigator, cx| navigator.go_back(cx));
    }

    /// Freeze the page on screen and hold it over a fresh one; `awaiting` while the switch is still in flight.
    fn hand_over(&mut self, awaiting: bool, cx: &mut Context<Self>) {
        if self.handover.is_some() {
            return;
        }
        if let Some(shown) = self.shown.clone() {
            self.router.freeze(&shown, cx);
            self.handover = Some(crate::handover::Handover::new(shown, awaiting));
        }
        self.router.clear();
    }

    /// Across a source switch, the old page over the new one until the new one has something to show.
    fn handover_layer(
        &mut self,
        page: gpui_kit::AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(handover) = self.handover.as_mut() else {
            return page;
        };
        let Some(frame) = handover.frame(window, cx) else {
            self.handover = None;
            return page;
        };
        div()
            .size_full()
            .relative()
            .child(div().size_full().opacity(frame.arriving).child(page))
            .children(frame.leaving.map(|leaving| {
                div()
                    .absolute()
                    .inset_0()
                    .opacity(1. - frame.arriving)
                    .child(leaving)
            }))
            // The held page answers for the old source, and the new one is not up yet, so neither takes a click.
            .child(div().absolute().inset_0().occlude())
            .into_any_element()
    }

    /// The routed page, fading and rising into place over the one it replaces, which fades out under it.
    fn page_layer(
        &mut self,
        key: String,
        page: gpui_kit::AnyView,
        previous: Option<gpui_kit::AnyView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        if key != self.page_key {
            self.page_key = key;
            self.page_fade = motion::Fade::hidden();
            self.page_leaving =
                previous.filter(|previous| previous.entity_id() != page.entity_id());
        }
        let shown = self.page_fade.sample(true, motion::FAST, window, cx);
        if shown >= 1. {
            self.page_leaving = None;
        }
        div()
            .size_full()
            .relative()
            // Without the old page under it, the first frame of the fade would be an empty window.
            .children(self.page_leaving.clone().map(|leaving| {
                div()
                    .absolute()
                    .inset_0()
                    .opacity(1. - shown)
                    .child(leaving)
            }))
            .child(
                div()
                    .size_full()
                    .relative()
                    .top(motion::RISE * (1. - shown))
                    .opacity(shown)
                    .child(page),
            )
            .into_any_element()
    }

    /// `bg-black/80` under the dialogs, black being the theme's background, fading rather than cutting in and out.
    fn dialog_scrim(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        use gpui_kit::component::WindowExt as _;
        let open = window.has_active_dialog(cx);
        let shown = self.scrim_fade.sample(open, motion::FAST, window, cx);
        (shown > 0.).then(|| {
            div()
                .absolute()
                .inset_0()
                .bg(cx.theme().background.opacity(MODAL_SCRIM * shown))
                .into_any_element()
        })
    }

    /// The guide alone: no bars, panels or player, and no "Shell" context, so no shortcut navigates away.
    fn render_first_run(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        components::focus::watch(div())
            .size_full()
            .track_focus(&self.focus)
            .key_context("FirstRun")
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .size_full()
                    .relative()
                    .child(self.first_run.clone())
                    .child(self.overlays.clone()),
            )
            .children(self.dialog_scrim(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
            .into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _rebuild = profile::Rebuild::new();
        // System colors follows the playing cover, which changes without the config changing.
        if self.applied_theme.0 == theme::SYSTEM {
            self.system_ease.follow(window, cx);
        } else {
            // Picking System again snaps to its scheme, as picking any theme does.
            self.system_ease = Default::default();
        }
        if let Some(loader) = self.loader.clone() {
            return div()
                .size_full()
                .child(loader)
                .children(Root::render_notification_layer(window, cx))
                .into_any_element();
        }
        if store::SetupModel::global(cx).read(cx).showing() {
            return self.render_first_run(window, cx);
        }
        // The guide's focused button leaves with it, and with nothing focused no shortcut fires.
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        let connected = self.connection.read(cx).is_connected();
        let chrome = Chrome::global(cx).read(cx);
        // The sheet is a full-window overlay, not a body state: its hero cover flies out of the
        // bar's own cover slot, so the bar has to still be there underneath it.
        let sheet_open = chrome.is_sheet_open() || self.fullscreen.read(cx).sheet_visible();
        // A disconnect outranks compact: a mini player with nothing behind it would keep showing
        // the last track it saw, with no way to learn the daemon had gone.
        let compact = chrome.is_compact() && connected;
        let nav_left = store::layout(cx).nav.position == store::layout::NavPosition::Left;
        let bar = Some(self.topbar.clone());
        let playlists = self.playlists_panel.read(cx);
        let playlists_open = playlists.is_open(cx) && connected;
        let playlists_width = playlists.width();
        let panel_open = chrome.panel().is_some();
        let quick_search_shown =
            chrome.is_quick_search_open() || self.quick_search.read(cx).is_visible();
        // The docked column is drag-resizable, so it owns the slot's width.
        let panel_width = self.side_panel.read(cx).panel_width();
        let dock = self
            .dock_fade
            .sample(panel_open && connected, motion::MEDIUM, window, cx);
        let show_player_bar = connected && !compact;
        let player_at_top = ConfigModel::global(cx)
            .read(cx)
            .config()
            .ui
            .player_bar_position
            == config::PlayerBarPosition::Top;
        let route = Navigator::global(cx).read(cx).current().clone();
        let page = self.router.page(&route, window, cx);
        let previous = self.shown.replace(page.clone());
        let page = self.page_layer(route.key(), page, previous, window, cx);
        let page = self.handover_layer(page, window, cx);
        // A skin with rounded panes sets the page and the side panel apart as tonal cards.
        let skin = components::style::chrome();
        let paned = skin.pane_radius > px(0.);
        let card = |pane: gpui_kit::Div| match paned {
            true => pane
                .rounded(skin.pane_radius)
                .overflow_hidden()
                .bg(components::style::white(PANE_TINT, cx)),
            false => pane,
        };

        // Compact swaps the body and nothing else. It used to return early, above the key context
        // and the dialog, sheet and notification layers, so the mini player had no shortcuts, no
        // toasts and no dialogs — and, being above the `connected` check, went on showing the last
        // track forever when the daemon went away.
        let body = if compact {
            self.mini_player.clone().into_any_element()
        } else {
            v_flex()
                .size_full()
                .w_full()
                .children(bar.clone().filter(|_| !nav_left))
                .when(show_player_bar && player_at_top, |body| {
                    body.child(self.player_bar.clone())
                })
                .child(
                    h_flex()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .items_stretch()
                        .when(paned, |body| {
                            body.gap(skin.pane_gap)
                                .pt(skin.pane_gap)
                                .pr(skin.pane_gap * 1.5)
                                .when(!nav_left, |body| body.pl(skin.pane_gap * 1.5))
                        })
                        .children(bar.filter(|_| nav_left))
                        .when(playlists_open, |body| {
                            body.child(
                                div()
                                    .w(playlists_width)
                                    .flex_shrink_0()
                                    .child(self.playlists_panel.clone()),
                            )
                        })
                        .child(
                            card(div())
                                .h_full()
                                .flex_1()
                                .min_w_0()
                                .relative()
                                // A disconnect dims the page and banners over it rather than
                                // replacing the window, so what was on screen stays on screen.
                                .child(topbar::content_frame().h_full().child(page))
                                .child(self.overlays.clone())
                                .child(self.debug_overlay.clone()),
                        )
                        .when(dock > 0., |body| {
                            // The column opens out and fades in together, its contents laid out at full width throughout.
                            body.child(
                                card(div())
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .w(panel_width * dock)
                                    .opacity(dock)
                                    .child(
                                        div()
                                            .w(panel_width)
                                            .h_full()
                                            .child(self.side_panel.clone()),
                                    ),
                            )
                        }),
                )
                .when(show_player_bar && !player_at_top, |window_body| {
                    window_body.child(self.player_bar.clone())
                })
                .into_any_element()
        };

        components::focus::watch(topbar::mouse_history(div()))
            .size_full()
            .track_focus(&self.focus)
            .key_context("Shell")
            .on_action(cx.listener(|this, _: &TogglePlayback, window, cx| {
                // Space belongs to whatever has focus. A binding is dispatched before the key
                // listeners that turn space into a click, so without this a focused button, switch
                // or checkbox would toggle playback instead of activating. Declining the action
                // lets the keystroke carry on to those listeners.
                if window
                    .focused(cx)
                    .is_some_and(|focused| focused != this.focus)
                {
                    cx.propagate();
                    return;
                }
                this.toggle_playback(cx);
            }))
            .on_action(cx.listener(|this, _: &QuickSearch, _, cx| this.toggle_quick_search(cx)))
            .on_action(cx.listener(|this, _: &Escape, _, cx| this.escape(cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|_, _: &Forward, _, cx| {
                Navigator::global(cx).update(cx, |navigator, cx| navigator.go_forward(cx));
            }))
            .on_action(cx.listener(|_, _: &TogglePlaylistsPanel, _, cx| {
                playlists_panel::toggle(cx);
            }))
            .on_action(
                cx.listener(|this, _: &debug_overlay::ToggleDebugOverlay, _, cx| {
                    this.debug_overlay
                        .update(cx, |overlay, cx| overlay.toggle(cx));
                }),
            )
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .children(match compact {
                true => None,
                false => crate::backdrop::layer(window, cx),
            })
            .child(body)
            .when(sheet_open && connected && !compact, |shell| {
                shell.child(div().absolute().inset_0().child(self.fullscreen.clone()))
            })
            .when(quick_search_shown && !compact, |shell| {
                shell.child(self.quick_search.clone())
            })
            .children(self.dialog_scrim(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
            .into_any_element()
    }
}
