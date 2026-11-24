//! The optional left column: the playlists and folders that are yours, as a list you can drop onto.

mod order;
mod settings;

use components::focus::{Axis, Roving, Step};
use components::format::{t, t_with};
use components::{
    DockPanel, DockSize, Edge, EmptyState, NavListRow, ReorderDropZones, TrackDrag, icons, style,
};
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Pixels, Render, SharedString, Styled, Subscription,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use std::collections::{HashMap, HashSet};

use api::ArtworkRef;
use store::layout::{Pin, PinKind};
use store::{LayoutModel, Link, Navigator, PlaylistsModel, Route, Target};

use order::Entry;

pub use settings::register as register_settings;

// The bounds the panel drags between, matching the right-hand queue panel's feel.
const MAX_WIDTH: Pixels = px(420.);
/// Collapsed, the panel is a rail of glyphs this wide.
const RAIL_WIDTH: Pixels = px(72.);
/// Dragged narrower than this it collapses to the rail, and it opens again only past `EXPAND_ABOVE`, so it never flickers between the two.
const COLLAPSE_BELOW: Pixels = px(150.);
const EXPAND_ABOVE: Pixels = px(180.);

/// The width a drag to `dragged` leaves the panel at, from `current`.
fn settle(current: Pixels, dragged: Pixels) -> Pixels {
    match current <= RAIL_WIDTH {
        true if dragged > EXPAND_ABOVE => dragged,
        true => RAIL_WIDTH,
        false if dragged < COLLAPSE_BELOW => RAIL_WIDTH,
        false => dragged,
    }
}
/// A window narrower than this opens with the panel closed.
pub const NARROW_WINDOW: Pixels = px(1100.);

/// Open the panel, or close it if it is already open; what `ctrl-b` does.
pub fn toggle(cx: &mut App) {
    LayoutModel::edit(cx, |layout| layout.panel.shown = !layout.panel.shown);
}

/// One row being dragged to a new place in the list.
#[derive(Clone)]
struct RowDrag {
    id: SharedString,
    label: SharedString,
}

impl Render for RowDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        components::DragChip::new(self.label.clone())
    }
}

#[derive(Clone)]
struct PinDrag {
    kind: PinKind,
    id: String,
    label: SharedString,
}

impl Render for PinDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        components::DragChip::new(self.label.clone())
    }
}

/// What one row draws, derived when the catalog changes rather than every frame.
#[derive(Clone)]
struct Row {
    id: SharedString,
    name: SharedString,
    folder: bool,
    detail: SharedString,
    artwork: Option<ArtworkRef>,
}

impl Row {
    fn target(&self) -> Target {
        match self.folder {
            true => Target::Folder {
                id: self.id.clone(),
                name: self.name.clone(),
            },
            false => Target::Playlist {
                id: self.id.clone(),
                name: self.name.clone(),
            },
        }
    }

    fn route(&self) -> Route {
        match self.folder {
            true => Route::PlaylistFolder(self.id.clone()),
            false => Route::Playlist(self.id.clone()),
        }
    }
}

/// The panel: a list of what is yours, drag-resized and remembered across launches.
pub struct PlaylistsPanel {
    rows: Vec<Row>,
    width: Pixels,
    roving: Roving,
    // The window was too narrow to spare a column at launch; asking for the panel clears it.
    suppressed: bool,
    shown: bool,
    merged: bool,
    pins: Vec<Pin>,
    /// How many pinned rows lead the keyboard walk, before the playlists.
    pinned_rows: usize,
    art: HashMap<(PinKind, String), ArtworkRef>,
    asked: HashSet<(PinKind, String)>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl PlaylistsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let playlists = PlaylistsModel::global(cx);
        let layout = LayoutModel::global(cx);
        let subscriptions = vec![
            cx.observe(&Navigator::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&playlists, |this: &mut Self, _, cx| {
                this.reread(cx);
                cx.notify();
            }),
            cx.observe(&layout, |this: &mut Self, _, cx| {
                let shown = store::layout(cx).panel.shown;
                // Asking for the panel, either way, is what lifts a launch-time suppression.
                if shown != this.shown {
                    this.shown = shown;
                    this.suppressed = false;
                }
                this.reread(cx);
                this.fetch_art(cx);
                cx.notify();
            }),
        ];
        // A window too narrow to spare a column opens without one, without forgetting the setting.
        let shown = store::layout(cx).panel.shown;
        let mut this = Self {
            rows: Vec::new(),
            width: px(store::layout(cx).panel.width),
            roving: Roving::new(Axis::Column, cx),
            suppressed: shown && window.viewport_size().width < NARROW_WINDOW,
            shown,
            merged: false,
            pins: Vec::new(),
            pinned_rows: 0,
            art: HashMap::new(),
            asked: HashSet::new(),
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.reread(cx);
        this.fetch_art(cx);
        this
    }

    /// What the panel has been dragged to; the shell sizes the slot it puts the panel in.
    pub fn width(&self) -> Pixels {
        self.width
    }

    pub fn set_merged(&mut self, merged: bool, cx: &mut Context<Self>) {
        if self.merged != merged {
            self.merged = merged;
            cx.notify();
        }
    }

    fn collapsed(&self) -> bool {
        !self.merged && self.width <= RAIL_WIDTH
    }

    /// Whether the shell should give the panel a slot at all.
    pub fn is_open(&self, cx: &App) -> bool {
        store::layout(cx).panel.shown && !self.suppressed
    }

    fn reread(&mut self, cx: &App) {
        let pinned = store::layout(cx).panel.pinned.clone();
        let model = PlaylistsModel::global(cx).read(cx);
        let mut catalog: Vec<Entry> = model
            .folders()
            .iter()
            .map(|folder| Entry {
                id: folder.id.clone(),
                folder: true,
            })
            .collect();
        catalog.extend(model.playlists().iter().map(|playlist| Entry {
            id: playlist.id.clone(),
            folder: false,
        }));
        self.rows = order::arrange(catalog, &pinned)
            .into_iter()
            .filter_map(|entry| match entry.folder {
                true => model
                    .folders()
                    .iter()
                    .find(|f| f.id == entry.id)
                    .map(|f| Row {
                        id: f.id.clone().into(),
                        name: f.name.clone().into(),
                        folder: true,
                        detail: t_with(
                            "playlist_folder_count",
                            &[("count", f.playlist_ids.len().into())],
                        ),
                        // A folder wears its first playlist's cover, as the Playlists page draws it.
                        artwork: f
                            .playlist_ids
                            .first()
                            .and_then(|id| model.playlist(id))
                            .and_then(|p| p.artwork.clone()),
                    }),
                false => model.playlist(&entry.id).map(|p| Row {
                    id: p.id.clone().into(),
                    name: p.name.clone().into(),
                    folder: false,
                    detail: t_with(
                        "playlist_track_count",
                        &[("count", p.track_keys.len().into())],
                    ),
                    artwork: p.artwork.clone(),
                }),
            })
            .collect();
        self.width = px(store::layout(cx).panel.width);
        self.pins = store::layout(cx).panel.pins.clone();
        self.pinned_rows = match store::layout(cx).panel.show_pins {
            true => self.pins.len(),
            false => 0,
        };
        self.roving.set_len(self.pinned_rows + self.rows.len());
    }

    fn artwork(&self, pin: &Pin, cx: &App) -> Option<ArtworkRef> {
        match pin.kind {
            PinKind::Playlist => PlaylistsModel::global(cx)
                .read(cx)
                .playlist(&pin.id)
                .and_then(|playlist| playlist.artwork.clone()),
            PinKind::Folder => None,
            kind => self.art.get(&(kind, pin.id.clone())).cloned(),
        }
    }

    fn fetch_art(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<Pin> = self
            .pins
            .iter()
            .filter(|pin| matches!(pin.kind, PinKind::Album | PinKind::Artist))
            .filter(|pin| !self.asked.contains(&(pin.kind, pin.id.clone())))
            .cloned()
            .collect();
        for pin in wanted {
            self.asked.insert((pin.kind, pin.id.clone()));
            let id = pin.id.clone();
            let call = match pin.kind {
                PinKind::Album => Link::global(cx).call(move |api| async move {
                    Ok(api.album(id).await?.and_then(|album| album.artwork))
                }),
                _ => Link::global(cx)
                    .call(move |api| async move { Ok(api.artist(id).await?.info.artwork) }),
            };
            cx.spawn(async move |this, cx| {
                let artwork = match call.await {
                    Ok(Some(artwork)) => artwork,
                    Ok(None) => return,
                    // Forgotten, so the next change to the pins asks again rather than never.
                    Err(error) => {
                        tracing::debug!(%error, "no artwork for a pinned item");
                        this.update(cx, |this, _| {
                            this.asked.remove(&(pin.kind, pin.id.clone()));
                        })
                        .ok();
                        return;
                    }
                };
                this.update(cx, |this, cx| {
                    this.art.insert((pin.kind, pin.id.clone()), artwork);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    fn pinned(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.pins.is_empty() || !store::layout(cx).panel.show_pins {
            return None;
        }
        let current = Navigator::global(cx).read(cx).current().clone();
        let collapsed = self.collapsed();
        let rows = self.pins.iter().enumerate().map(|(index, pin)| {
            let route = pin.route();
            let zones = ReorderDropZones::new(
                ElementId::from(("pin-drop", index)),
                components::tall_row(),
                index,
            )
            .on_drop::<PinDrag>(|drag, at, _, cx| {
                let (kind, id) = (drag.kind, drag.id.clone());
                LayoutModel::edit(cx, move |layout| layout.panel.move_pin(kind, &id, at));
            });
            let (glyph, kind) = match pin.kind {
                PinKind::Album => (icons::DISC, t("album")),
                PinKind::Artist => (icons::MIC, t("artist")),
                PinKind::Playlist => (icons::LIST, t("pinned_playlist")),
                PinKind::Folder => (icons::FOLDER, t("pinned_folder")),
            };
            NavListRow::new(
                "pinned-row",
                SharedString::from(format!("{:?}-{}", pin.kind, pin.id)),
                SharedString::from(pin.name.clone()),
            )
            .glyph(glyph)
            .cover(self.artwork(pin, cx), pin.kind == PinKind::Artist)
            .subtitle(kind)
            .active(current == route)
            .collapsed(collapsed)
            .focused(self.roving.is_active(index, window))
            .menu(pin.target())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.roving.set_cursor(index);
                Navigator::navigate(route.clone(), cx);
            }))
            .drag(PinDrag {
                kind: pin.kind,
                id: pin.id.clone(),
                label: SharedString::from(pin.name.clone()),
            })
            .drop_zones(zones)
            .into_any_element()
        });
        Some(
            v_flex()
                .w_full()
                .flex_shrink_0()
                .when(!collapsed, |list| list.child(self.heading(t("pinned"), cx)))
                .child(
                    v_flex()
                        .w_full()
                        .when(!self.merged, |list| list.px_2())
                        .py_1()
                        .children(rows),
                )
                .into_any_element(),
        )
    }

    fn heading(&self, label: SharedString, cx: &App) -> impl IntoElement {
        div()
            .px_2()
            .pt_1()
            .pb_1()
            .text_xs()
            .font_semibold()
            .text_color(style::white(style::TEXT_SECONDARY, cx))
            .child(label)
    }

    fn open(&self, index: usize, cx: &mut App) {
        if let Some(row) = self.rows.get(index) {
            Navigator::navigate(row.route(), cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match self.roving.walk(event) {
            Step::Moved(at) => {
                if let Some(row) = at.checked_sub(self.pinned_rows) {
                    self.scroll
                        .scroll_to_item(row, gpui_kit::ScrollStrategy::Nearest);
                }
                cx.notify();
            }
            Step::Activate(at) => match at.checked_sub(self.pinned_rows) {
                Some(row) => self.open(row, cx),
                None => {
                    if let Some(pin) = self.pins.get(at) {
                        Navigator::navigate(pin.route(), cx);
                    }
                }
            },
            Step::Ignored => cx.propagate(),
        }
    }

    /// The order the list is in now, so a drop writes every row down and the order sticks.
    fn pin(&self, id: &str, at: usize, cx: &mut App) {
        let entries: Vec<Entry> = self
            .rows
            .iter()
            .map(|row| Entry {
                id: row.id.to_string(),
                folder: row.folder,
            })
            .collect();
        let pinned = order::reorder(&entries, id, at);
        LayoutModel::edit(cx, move |layout| layout.panel.pinned = pinned);
    }

    fn row(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(index).cloned() else {
            return div().into_any_element();
        };
        let current = Navigator::global(cx).read(cx).current().clone();
        let id = row.id.to_string();
        let route = row.route();
        let panel = cx.entity().downgrade();
        let covers = store::layout(cx).panel.playlist_covers;
        let zones = ReorderDropZones::new(
            ElementId::from(("panel-drop", index)),
            match covers {
                true => components::tall_row(),
                false => style::list_row(),
            },
            index,
        )
        .on_drop::<RowDrag>(move |drag, at, _, cx| {
            panel.update(cx, |this, cx| this.pin(&drag.id, at, cx)).ok();
        });

        let item = NavListRow::new("panel-row", row.id.clone(), row.name.clone())
            .glyph(match row.folder {
                true => icons::FOLDER,
                false => icons::LIST,
            })
            .when(covers, |item| item.cover(row.artwork.clone(), false))
            .detail(row.detail.clone())
            .active(current == route)
            .collapsed(self.collapsed())
            .focused(self.roving.is_active(self.pinned_rows + index, window))
            .menu(row.target())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.roving.set_cursor(this.pinned_rows + index);
                Navigator::navigate(route.clone(), cx);
            }))
            .drag(RowDrag {
                id: row.id.clone(),
                label: row.name.clone(),
            })
            .drop_zones(zones);
        // A track dropped on a playlist joins it; a folder holds playlists, so it takes none.
        match row.folder {
            true => item,
            false => item.accept::<TrackDrag>(move |drag, _, cx| {
                let (keys, id) = (drag.keys.to_vec(), id.clone());
                PlaylistsModel::global(cx)
                    .update(cx, move |model, cx| model.add_tracks(id, keys, cx));
            }),
        }
        .into_any_element()
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .text_xs()
            .font_semibold()
            .text_color(style::white(style::TEXT_SECONDARY, cx))
            .child(t("playlists"))
    }

    /// The bordering edge drags the panel up to 420 px, or down to the rail.
    fn on_resize(&mut self, dragged: Pixels, cx: &mut Context<Self>) {
        let width = settle(self.width, dragged);
        if width != self.width {
            self.width = width;
            LayoutModel::edit(cx, move |layout| layout.panel.width = width.as_f32());
            cx.notify();
        }
    }
}

impl Render for PlaylistsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("playlists-panel");
        components::scroll::smooth_list("playlists-panel-scroll", &self.scroll, window, cx);
        let pinned = self.pinned(window, cx);
        let count = self.rows.len();
        let collapsed = self.collapsed();
        let body = match count {
            0 if collapsed => div().flex_1().min_h_0().into_any_element(),
            0 if self.merged => div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(style::slate_400(cx))
                .child(t("no_playlists_yet"))
                .into_any_element(),
            0 => div()
                .flex_1()
                .min_h_0()
                .child(EmptyState::new(t("no_playlists_yet")).icon(icons::LIST))
                .into_any_element(),
            _ => uniform_list(
                "playlists-panel-list",
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                    range.map(|index| this.row(index, window, cx)).collect()
                }),
            )
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .when(!self.merged, |list| list.px_2())
            .py_1()
            .into_any_element(),
        };
        if self.merged {
            return self
                .roving
                .group(v_flex().id("playlists-panel"), "PlaylistsPanel")
                .size_full()
                .pt_2()
                .border_t_1()
                .border_color(style::white(style::BORDER_SOFT, cx))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key(event, cx)))
                .children(pinned)
                .when(store::layout(cx).panel.show_playlists, |panel| {
                    panel
                        .child(div().px_2().pt_1().pb_1().child(self.header(cx)))
                        .child(body)
                });
        }
        let panel = cx.entity().downgrade();
        // Right to left the panel sits right of the content, so it borders it on its left.
        let edge =
            match (store::layout(cx).panel.side == store::layout::Side::Left) != i18n::is_rtl() {
                true => Edge::Right,
                false => Edge::Left,
            };

        self.roving
            .group(v_flex().id("playlists-panel"), "PlaylistsPanel")
            .h_full()
            .w_full()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key(event, cx)))
            .child(
                DockPanel::new("playlists-panel", edge)
                    .size(DockSize::Compact)
                    .when(!collapsed, |dock| dock.header(self.header(cx)))
                    .body(
                        v_flex()
                            .size_full()
                            .children(pinned)
                            .when(store::layout(cx).panel.show_playlists, |list| {
                                list.child(body)
                            }),
                    )
                    .on_close(|_, cx| toggle(cx))
                    .resizable(self.width, RAIL_WIDTH, MAX_WIDTH, move |width, _, cx| {
                        panel.update(cx, |this, cx| this.on_resize(width, cx)).ok();
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_under_the_threshold_collapses_to_the_rail() {
        assert_eq!(settle(px(260.), px(149.)), RAIL_WIDTH);
        assert_eq!(settle(px(260.), px(160.)), px(160.));
    }

    #[test]
    fn the_rail_opens_only_past_the_upper_threshold() {
        assert_eq!(settle(RAIL_WIDTH, px(170.)), RAIL_WIDTH);
        assert_eq!(settle(RAIL_WIDTH, px(190.)), px(190.));
    }
}
