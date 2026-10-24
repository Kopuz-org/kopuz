//! The optional left column: the playlists and folders that are yours, as a list you can drop onto.

mod order;
mod settings;

use components::focus::{Axis, Roving, Step};
use components::format::t;
use components::{
    DockPanel, DockSize, Edge, EmptyState, NavListRow, ReorderDropZones, TrackDrag, icons, style,
};
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, App, Context, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Pixels, Render, SharedString, Styled, Subscription,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use store::{LayoutModel, Navigator, PlaylistsModel, Route, Target};

use order::Entry;

pub use settings::register as register_settings;

// The bounds the panel drags between, matching the right-hand queue panel's feel.
const MIN_WIDTH: Pixels = px(180.);
const MAX_WIDTH: Pixels = px(420.);
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

/// What one row draws, derived when the catalog changes rather than every frame.
#[derive(Clone)]
struct Row {
    id: SharedString,
    name: SharedString,
    folder: bool,
    detail: SharedString,
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
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl PlaylistsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let playlists = PlaylistsModel::global(cx);
        let layout = LayoutModel::global(cx);
        let subscriptions = vec![
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
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.reread(cx);
        this
    }

    /// What the panel has been dragged to; the shell sizes the slot it puts the panel in.
    pub fn width(&self) -> Pixels {
        self.width
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
                        detail: SharedString::from(f.playlist_ids.len().to_string()),
                    }),
                false => model.playlist(&entry.id).map(|p| Row {
                    id: p.id.clone().into(),
                    name: p.name.clone().into(),
                    folder: false,
                    detail: SharedString::from(p.track_keys.len().to_string()),
                }),
            })
            .collect();
        self.width = px(store::layout(cx).panel.width);
        self.roving.set_len(self.rows.len());
    }

    fn open(&self, index: usize, cx: &mut App) {
        if let Some(row) = self.rows.get(index) {
            Navigator::navigate(row.route(), cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match self.roving.walk(event) {
            Step::Moved(at) => {
                self.scroll
                    .scroll_to_item(at, gpui_kit::ScrollStrategy::Nearest);
                cx.notify();
            }
            Step::Activate(at) => self.open(at, cx),
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
        let zones = ReorderDropZones::new(
            ElementId::from(("panel-drop", index)),
            style::list_row(),
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
            .detail(row.detail.clone())
            .active(current == route)
            .focused(self.roving.is_active(index, window))
            .menu(row.target())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.roving.set_cursor(index);
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

    /// The right edge drags the panel between 180 and 420 px.
    fn on_resize(&mut self, width: Pixels, cx: &mut Context<Self>) {
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
        let count = self.rows.len();
        let body = match count {
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
            .px_2()
            .py_1()
            .into_any_element(),
        };
        let panel = cx.entity().downgrade();

        self.roving
            .group(v_flex().id("playlists-panel"), "PlaylistsPanel")
            .h_full()
            .w_full()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key(event, cx)))
            .child(
                DockPanel::new("playlists-panel", Edge::Right)
                    .size(DockSize::Compact)
                    .header(self.header(cx))
                    .body(v_flex().size_full().child(body))
                    .on_close(|_, cx| toggle(cx))
                    .resizable(self.width, MIN_WIDTH, MAX_WIDTH, move |width, _, cx| {
                        panel.update(cx, |this, cx| this.on_resize(width, cx)).ok();
                    }),
            )
    }
}
