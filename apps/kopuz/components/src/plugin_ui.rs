//! What integration plugins add to the app's pages: their panels, drawn from
//! the blocks they answer with, and running their actions.

use api::{
    ActionTarget, PanelBlock, PanelPlace, PanelTarget, PluginAction, PluginPanel, TrackInfo,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div,
};
use store::Query;
use store::plugin_actions::PluginActions;

use crate::{Heading, SectionHeading, style};

/// `text-white/60`, the secondary line of a row.
const DIM: f32 = 0.60;

/// Run a plugin's action, asking its question first when it has one.
pub fn run_action(action: PluginAction, target: ActionTarget, window: &mut Window, cx: &mut App) {
    match action.ask.clone() {
        // On the next frame, so a palette or menu closing in this one does not take the prompt's focus back.
        Some(question) => window.defer(cx, move |window, cx| {
            crate::dialogs::open_rename(
                question,
                "",
                move |answer, cx| {
                    store::plugin_actions::run(&action, target.clone(), Some(answer), cx)
                },
                window,
                cx,
            )
        }),
        None => store::plugin_actions::run(&action, target, None, cx),
    }
}

/// A block, with the rows a track list names read from the library.
#[derive(Clone)]
enum Drawn {
    Heading(String),
    Text(String),
    Tracks(Vec<TrackInfo>),
    Action { label: String, action: String },
}

struct Shown {
    panel: PluginPanel,
    blocks: Entity<Query<Vec<Drawn>>>,
}

/// The plugin panels drawn for one page, or one of them in a sheet.
pub struct PluginPanels {
    place: PanelPlace,
    target: PanelTarget,
    /// Only this plugin's panel, by plugin and panel id.
    only: Option<(String, String)>,
    shown: Vec<Shown>,
    _subscriptions: Vec<Subscription>,
}

fn action_target(target: &PanelTarget) -> ActionTarget {
    match target {
        PanelTarget::Album(album) => ActionTarget::Album(album.clone()),
        PanelTarget::Artist(artist) => ActionTarget::Artist(artist.clone()),
        PanelTarget::Track(track) => ActionTarget::Tracks(vec![track.clone()]),
    }
}

impl PluginPanels {
    pub fn new(target: PanelTarget, cx: &mut Context<Self>) -> Self {
        Self::build(target, None, cx)
    }

    /// The panels for `target`, or only the one `only` names, each fetched once.
    fn build(target: PanelTarget, only: Option<(String, String)>, cx: &mut Context<Self>) -> Self {
        let place = match target {
            PanelTarget::Album(_) => PanelPlace::Album,
            PanelTarget::Artist(_) => PanelPlace::Artist,
            PanelTarget::Track(_) => PanelPlace::NowPlaying,
        };
        let model = PluginActions::global(cx);
        let subscriptions = vec![cx.observe(&model, |this, _, cx| this.sync(cx))];
        let mut this = Self {
            place,
            target,
            only,
            shown: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.sync(cx);
        this
    }

    /// Only `panel`, under the title something around it already shows.
    fn only(target: PanelTarget, panel: (String, String), cx: &mut Context<Self>) -> Self {
        Self::build(target, Some(panel), cx)
    }

    /// Whether every panel has answered at least once.
    fn settled(&self, cx: &App) -> bool {
        self.shown
            .iter()
            .all(|shown| shown.blocks.read(cx).value().is_some())
    }

    /// The panels that have something to show, for a row that opens each in a sheet: one
    /// still loading, failed or empty gets no button.
    pub fn drawn(&self, cx: &App) -> Vec<PluginPanel> {
        self.shown
            .iter()
            .filter(|shown| {
                shown
                    .blocks
                    .read(cx)
                    .value()
                    .is_some_and(|blocks| !blocks.is_empty())
            })
            .map(|shown| shown.panel.clone())
            .collect()
    }

    /// Open `panel` in a sheet of its own, for a page with no room below its list.
    pub fn open_sheet(target: PanelTarget, panel: PluginPanel, window: &mut Window, cx: &mut App) {
        let view = cx.new(|cx| Self::only(target, (panel.plugin.clone(), panel.id.clone()), cx));
        crate::dialogs::FormDialog::new(SharedString::from(panel.title))
            .size(crate::dialogs::DialogSize::Sheet)
            .footer(crate::dialogs::FormFooter::Cancel)
            .open(view, window, cx);
    }

    /// Keep one read per panel the plugins turned on draw here.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let mut panels = PluginActions::global(cx).read(cx).panels_at(self.place, cx);
        if let Some((plugin, id)) = &self.only {
            panels.retain(|panel| panel.plugin == *plugin && panel.id == *id);
        }
        let unchanged = panels.len() == self.shown.len()
            && panels
                .iter()
                .zip(&self.shown)
                .all(|(panel, shown)| *panel == shown.panel);
        if unchanged {
            return;
        }
        self.shown = panels
            .into_iter()
            .map(|panel| {
                let (plugin, id, target) =
                    (panel.plugin.clone(), panel.id.clone(), self.target.clone());
                let blocks = cx.new(|cx| {
                    Query::new(
                        &[api::Table::Plugins, api::Table::Tracks],
                        move |api| {
                            let (plugin, id, target) = (plugin.clone(), id.clone(), target.clone());
                            async move {
                                // A panel that fails is left out, not toasted: the page is still
                                // fine without it, and a stale answer must not linger in its place.
                                let blocks = match api.render_plugin_panel(plugin, id, target).await
                                {
                                    Ok(blocks) => blocks,
                                    Err(error) => {
                                        tracing::debug!(%error, "a plugin panel was not drawn");
                                        return Ok(Vec::new());
                                    }
                                };
                                let mut drawn = Vec::new();
                                for block in blocks {
                                    drawn.push(match block {
                                        PanelBlock::Heading(text) => Drawn::Heading(text),
                                        PanelBlock::Text(text) => Drawn::Text(text),
                                        PanelBlock::Tracks(keys) => Drawn::Tracks(
                                            api.tracks_by_keys(keys).await.unwrap_or_default(),
                                        ),
                                        PanelBlock::Action { label, action } => {
                                            Drawn::Action { label, action }
                                        }
                                    });
                                }
                                Ok(drawn)
                            }
                        },
                        cx,
                    )
                });
                cx.observe(&blocks, |_, _, cx| cx.notify()).detach();
                Shown { panel, blocks }
            })
            .collect();
        cx.notify();
    }

    fn panel(&self, shown: &Shown, cx: &mut Context<Self>) -> Option<AnyElement> {
        let blocks = shown.blocks.read(cx).value()?.clone();
        if blocks.is_empty() {
            return None;
        }
        let plugin = shown.panel.plugin.clone();
        let mut body = v_flex().w_full().gap_3();
        for (at, block) in blocks.into_iter().enumerate() {
            let key = format!("{}-{}-{at}", shown.panel.plugin, shown.panel.id);
            body = body.child(match block {
                Drawn::Heading(text) => SectionHeading::new(text)
                    .heading(Heading::Group)
                    .into_any_element(),
                Drawn::Text(text) => div()
                    .text_sm()
                    .text_color(style::white(DIM, cx))
                    .child(text)
                    .into_any_element(),
                Drawn::Tracks(rows) => self.tracks(key, rows, cx),
                Drawn::Action { label, action } => {
                    let (plugin, target) = (plugin.clone(), action_target(&self.target));
                    h_flex()
                        .child(
                            Button::new(SharedString::from(key))
                                .custom(style::soft_button(cx.theme().foreground, cx))
                                .small()
                                .label(label)
                                .on_click(move |_, window, cx| {
                                    let found = PluginActions::global(cx)
                                        .read(cx)
                                        .find(&plugin, &action, cx);
                                    match found {
                                        Some(found) => run_action(found, target.clone(), window, cx),
                                        None => tracing::warn!(
                                            %plugin, %action,
                                            "a panel button names an action its plugin no longer offers"
                                        ),
                                    }
                                }),
                        )
                        .into_any_element()
                }
            });
        }
        // A sheet already carries the title.
        let titled = self.only.is_none();
        Some(
            v_flex()
                .w_full()
                .gap_4()
                .when(titled, |panel| {
                    panel.child(
                        SectionHeading::new(shown.panel.title.clone()).heading(Heading::Shelf),
                    )
                })
                .child(body)
                .into_any_element(),
        )
    }

    /// Rows that play the list from the one clicked.
    fn tracks(&self, key: String, rows: Vec<TrackInfo>, cx: &mut Context<Self>) -> AnyElement {
        let keys: Vec<String> = rows.iter().map(|row| row.key.clone()).collect();
        v_flex()
            .w_full()
            .children(rows.into_iter().enumerate().map(|(at, row)| {
                let keys = keys.clone();
                h_flex()
                    // By position: the same track may be listed twice.
                    .id(SharedString::from(format!("{key}-{at}")))
                    .w_full()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|row| row.bg(style::white(0.06, cx)))
                    .child(div().text_sm().truncate().child(row.title.clone()))
                    .when(!row.artist.is_empty(), |line| {
                        line.child(
                            div()
                                .text_sm()
                                .text_color(style::white(DIM, cx))
                                .truncate()
                                .child(row.artist.clone()),
                        )
                    })
                    .on_click(move |_, _, cx| {
                        store::actions::play_tracks(keys.clone(), Some(at as u32), cx)
                    })
            }))
            .into_any_element()
    }
}

impl PluginPanels {
    /// Whether nothing is drawn now, so a page can leave out the room for it.
    pub fn is_empty(&self, cx: &App) -> bool {
        self.shown.iter().all(|shown| {
            shown
                .blocks
                .read(cx)
                .value()
                .is_none_or(|blocks| blocks.is_empty())
        })
    }
}

impl Render for PluginPanels {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut column = v_flex().w_full().gap_10();
        let shown: Vec<_> = self
            .shown
            .iter()
            .map(|shown| Shown {
                panel: shown.panel.clone(),
                blocks: shown.blocks.clone(),
            })
            .collect();
        for shown in &shown {
            if let Some(panel) = self.panel(shown, cx) {
                column = column.child(panel);
            }
        }
        column
    }
}

/// The panels plugins draw beside the queue and the lyrics, as tabs.
pub fn now_playing_tabs(cx: &App) -> Vec<PluginPanel> {
    PluginActions::global(cx)
        .read(cx)
        .panels_at(PanelPlace::NowPlaying, cx)
}

/// One plugin's tab beside the queue and the lyrics, drawn again for each
/// track that starts.
pub struct PluginTab {
    panel: (String, String),
    track: Option<String>,
    panels: Option<Entity<PluginPanels>>,
    _subscription: Subscription,
}

impl PluginTab {
    pub fn new(panel: &PluginPanel, cx: &mut Context<Self>) -> Self {
        let player = store::PlayerModel::global(cx);
        // The player notifies on every playhead tick; only a new track matters here.
        let subscription = cx.observe(&player, |this, _, cx| this.follow(cx));
        let mut this = Self {
            panel: (panel.plugin.clone(), panel.id.clone()),
            track: None,
            panels: None,
            _subscription: subscription,
        };
        this.follow(cx);
        this
    }

    /// Whether this is the tab for `panel`.
    pub fn is(&self, panel: &PluginPanel) -> bool {
        self.panel.0 == panel.plugin && self.panel.1 == panel.id
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let track = store::PlayerModel::global(cx)
            .read(cx)
            .track()
            .map(|track| track.key.clone())
            .filter(|key| !key.is_empty());
        if track == self.track {
            return;
        }
        self.track = track.clone();
        self.panels = track.map(|track| {
            let panel = self.panel.clone();
            let panels = cx.new(|cx| PluginPanels::only(PanelTarget::Track(track), panel, cx));
            cx.observe(&panels, |_, _, cx| cx.notify()).detach();
            panels
        });
        cx.notify();
    }
}

impl Render for PluginTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Still asking: nothing yet, rather than "nothing here" for a moment.
        if let Some(panels) = &self.panels
            && !panels.read(cx).settled(cx)
        {
            return div().into_any_element();
        }
        let drawn = self
            .panels
            .as_ref()
            .filter(|panels| !panels.read(cx).is_empty(cx));
        match drawn {
            Some(panels) => div()
                .id("plugin-tab")
                .size_full()
                .overflow_y_scroll()
                .p_4()
                .child(panels.clone())
                .into_any_element(),
            None => crate::EmptyState::new(i18n::t("plugin_tab_empty")).into_any_element(),
        }
    }
}
