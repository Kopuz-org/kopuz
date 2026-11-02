use std::collections::HashSet;
use std::rc::Rc;

use api::TrackInfo;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled, UniformListScrollHandle,
    WeakEntity, Window, actions, div, uniform_list,
};
use store::{RowEdit, RowScope, Target};

use crate::columns::{self, Layout};
use crate::format::t;
use crate::menus;
use crate::selection::Selection;
use crate::style;
use crate::track_row::TrackRow;

/// Which columns a list draws; an album view numbers its rows and hides the album column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackListOptions {
    pub show_index: bool,
    pub show_cover: bool,
    pub show_album: bool,
    pub track_numbers: bool,
    pub selectable: bool,
}

impl Default for TrackListOptions {
    fn default() -> Self {
        Self {
            show_index: true,
            show_cover: true,
            show_album: true,
            track_numbers: false,
            selectable: true,
        }
    }
}

pub use store::TrackActions;

/// What the page has to answer for; the list owns selection and the cursor, nothing else.
#[derive(Debug, Clone)]
pub enum TrackListEvent {
    /// A row was activated; the page decides what queue that starts.
    Play(usize),
    RemoveFromPlaylist(usize),
    Delete(Vec<String>),
    PlayNext(Vec<String>),
    Queue(Vec<String>),
    ToggleFavorite(Vec<String>),
    Reorder {
        from: usize,
        to: usize,
    },
    SelectionChanged,
}

/// Tracks dragged out of a list, for a drop target such as the queue.
#[derive(Debug, Clone)]
pub struct TrackDrag {
    pub keys: std::sync::Arc<[String]>,
    pub label: SharedString,
}

impl Render for TrackDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::drag::DragChip::new(self.label.clone())
    }
}

/// The shared track list: virtualized rows, selection, and the row menu.
pub struct TrackList {
    rows: Vec<TrackInfo>,
    options: TrackListOptions,
    row_actions: TrackActions,
    me: WeakEntity<TrackList>,
    selection: Selection,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    playing: Option<String>,
    row_images: bool,
    stored: HashSet<String>,
    bound: Vec<gpui_kit::Subscription>,
}

actions!(
    track_list,
    [
        CursorUp,
        CursorDown,
        ActivateRow,
        SelectAll,
        ClearSelection,
        PlayNextSelection,
        QueueSelection,
        FavoriteSelection,
        RemoveSelection
    ]
);

impl EventEmitter<TrackListEvent> for TrackList {}

impl Focusable for TrackList {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TrackList {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            rows: Vec::new(),
            options: TrackListOptions::default(),
            row_actions: TrackActions::default(),
            me: cx.weak_entity(),
            selection: Selection::new(),
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            playing: None,
            row_images: true,
            stored: HashSet::new(),
            bound: Vec::new(),
        }
    }

    /// The key of the track now playing, which its row draws as such.
    pub fn set_playing(&mut self, playing: Option<String>, cx: &mut Context<Self>) {
        if self.playing != playing {
            self.playing = playing;
            cx.notify();
        }
    }

    /// The keys with an offline copy, which their rows mark.
    pub fn set_stored(&mut self, stored: HashSet<String>, cx: &mut Context<Self>) {
        if self.stored != stored {
            self.stored = stored;
            cx.notify();
        }
    }

    /// Whether rows draw a cover thumbnail, subject to the list's own options.
    pub fn set_row_images(&mut self, row_images: bool, cx: &mut Context<Self>) {
        if self.row_images != row_images {
            self.row_images = row_images;
            cx.notify();
        }
    }

    /// Tie subscriptions to this list's lifetime, so an owner's wiring goes when the list does.
    pub fn hold(&mut self, subscriptions: impl IntoIterator<Item = gpui_kit::Subscription>) {
        self.bound.extend(subscriptions);
    }

    pub fn with_options(mut self, options: TrackListOptions) -> Self {
        self.options = options;
        self
    }

    pub fn with_actions(mut self, actions: TrackActions) -> Self {
        self.row_actions = actions;
        self
    }

    pub fn set_options(&mut self, options: TrackListOptions, cx: &mut Context<Self>) {
        self.options = options;
        cx.notify();
    }

    pub fn set_actions(&mut self, actions: TrackActions, cx: &mut Context<Self>) {
        self.row_actions = actions;
        cx.notify();
    }

    pub fn set_rows(&mut self, rows: Vec<TrackInfo>, cx: &mut Context<Self>) {
        let keys: HashSet<String> = rows.iter().map(|row| row.key.clone()).collect();
        let was_selecting = self.selection.is_active();
        self.selection.retain_loaded(&keys, true);
        // Select-all over a list that emptied would leave a "0 selected" pill eating clicks.
        if rows.is_empty() {
            self.selection.clear();
        }
        self.rows = rows;
        cx.notify();
        if was_selecting != self.selection.is_active() {
            cx.emit(TrackListEvent::SelectionChanged);
        }
    }

    pub fn rows(&self) -> &[TrackInfo] {
        &self.rows
    }

    pub fn keys(&self) -> Vec<String> {
        self.rows.iter().map(|row| row.key.clone()).collect()
    }

    pub fn selected_keys(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| self.selection.contains(&row.key))
            .map(|row| row.key.clone())
            .collect()
    }

    pub fn is_selecting(&self) -> bool {
        self.selection.is_active()
    }

    /// What a row's menu acts on: the whole selection when the row is in it, else the row alone.
    fn scope(&self, index: usize) -> RowScope {
        let list = self.me.clone();
        RowScope::new(index, self.rows.len(), self.row_actions, move |edit, cx| {
            list.update(cx, |_, cx| cx.emit(TrackListEvent::from(edit)))
                .ok();
        })
    }

    fn row_target(&self, index: usize) -> Target {
        let Some(row) = self.rows.get(index) else {
            return Target::None;
        };
        if !self.selection.contains(&row.key) {
            return Target::track(row.key.clone(), row.title.clone()).with_row(self.scope(index));
        }
        let keys: Vec<SharedString> = self
            .selected_keys()
            .into_iter()
            .map(SharedString::from)
            .collect();
        Target::selection(keys, row.title.clone()).with_row(self.scope(index))
    }

    /// What the keyboard acts on: the selection when there is one, else the row under the cursor.
    pub fn focus_target(&self) -> Target {
        let index = self
            .rows
            .iter()
            .position(|row| self.selection.contains(&row.key))
            .or(self.selection.cursor());
        index.map_or(Target::None, |index| self.row_target(index))
    }

    fn action_keys(&self) -> Vec<String> {
        match self.selection.is_active() {
            true => self.selected_keys(),
            false => self
                .selection
                .cursor()
                .and_then(|index| self.rows.get(index))
                .map(|row| vec![row.key.clone()])
                .unwrap_or_default(),
        }
    }

    fn emit_keys(&self, event: fn(Vec<String>) -> TrackListEvent, cx: &mut Context<Self>) {
        let keys = self.action_keys();
        if !keys.is_empty() {
            cx.emit(event(keys));
        }
    }

    /// The Delete key: take the cursor row out of its playlist, or off disk once confirmed.
    fn remove_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys = self.action_keys();
        if keys.is_empty() {
            cx.propagate();
            return;
        }
        if self.row_actions.remove_from_playlist {
            if let Some(index) = self.selection.cursor() {
                cx.emit(TrackListEvent::RemoveFromPlaylist(index));
            }
            return;
        }
        if !self.row_actions.delete {
            cx.propagate();
            return;
        }
        let label = self.focus_target().label();
        let list = cx.entity().downgrade();
        crate::dialogs::confirm(
            crate::dialogs::ConfirmKind::Delete,
            t("delete"),
            label,
            move |_, cx| {
                list.update(cx, |_, cx| cx.emit(TrackListEvent::Delete(keys.clone())))
                    .ok();
            },
            window,
            cx,
        );
    }

    /// How many rows are selected, without the per-row allocation `selected_keys` pays.
    pub fn selected_count(&self) -> usize {
        self.selection.count(self.rows.len())
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        cx.notify();
        cx.emit(TrackListEvent::SelectionChanged);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        self.selection.select_everything();
        cx.notify();
        cx.emit(TrackListEvent::SelectionChanged);
    }

    pub fn scroll_to(&self, index: usize) {
        self.scroll
            .scroll_to_item(index, gpui_kit::ScrollStrategy::Center);
    }

    // Select-all is held as a flag, so a single pick needs it spelled out first.
    fn resolve_everything(&mut self) {
        let keys = self.keys();
        self.selection.resolve(keys);
    }

    fn toggle_selected(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(key) = self.rows.get(index).map(|row| row.key.clone()) else {
            return;
        };
        self.resolve_everything();
        self.selection.toggle(&key, index);
        cx.notify();
        cx.emit(TrackListEvent::SelectionChanged);
    }

    /// Move the keyboard cursor, starting from the top when there is none yet.
    fn move_cursor(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        let next = match self.selection.cursor() {
            Some(current) => current.saturating_add_signed(step).min(last),
            None if step < 0 => last,
            None => 0,
        };
        self.selection.set_cursor(Some(next));
        self.scroll_to(next);
        cx.notify();
    }

    /// Enter on the cursor row: play it, or tick it when a selection is being built.
    fn activate_cursor(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selection.cursor() else {
            return;
        };
        if index >= self.rows.len() {
            return;
        }
        if self.selection.is_active() {
            self.toggle_selected(index, cx);
        } else {
            cx.emit(TrackListEvent::Play(index));
        }
    }

    /// Select every row between the last one picked and `index`, the way a shift-click reads.
    fn select_through(&mut self, index: usize, cx: &mut Context<Self>) {
        self.resolve_everything();
        let rows = &self.rows;
        self.selection
            .extend_to(index, |slot| rows.get(slot).map(|row| row.key.clone()));
        cx.notify();
        cx.emit(TrackListEvent::SelectionChanged);
    }

    /// The columns the rows draw, for a page's header to sit over them.
    pub fn layout(&self) -> Layout {
        columns::standard(self.options.show_album)
    }

    fn row(&self, index: usize, columns: &Layout, cx: &mut Context<Self>) -> AnyElement {
        let Some(track) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let key = track.key.clone();
        let playing = self.playing.as_deref() == Some(key.as_str());
        let selected = self.selection.contains(&key);
        let lead = self.options.show_index.then(|| {
            let number = match self.options.track_numbers {
                true => track.track_number.unwrap_or(index as u32 + 1),
                false => index as u32 + 1,
            };
            SharedString::from(number.to_string())
        });
        let target = self.row_target(index);
        // A row inside a selection drags the selection; the queue is where it lands.
        let keys: Vec<String> = match selected {
            true => target.keys().iter().map(ToString::to_string).collect(),
            false => vec![key.clone()],
        };
        let label = target.label();
        let menu = Rc::new(
            move |popup, window: &mut Window, cx: &mut Context<PopupMenu>| {
                menus::build(target.clone(), popup, window, cx)
            },
        );
        TrackRow::new(track, index)
            .columns(columns.clone())
            .height(style::row_height())
            .lead(lead)
            .cover(self.options.show_cover && self.row_images)
            .playing(playing)
            .stored(self.stored.contains(&key))
            .selected(selected)
            .picking(self.options.selectable && self.selection.is_active())
            .cursor(self.selection.cursor() == Some(index))
            .drag(TrackDrag {
                keys: keys.into(),
                label,
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                // A click is also where the arrows should carry on from.
                this.selection.set_cursor(Some(index));
                window.focus(&this.focus, cx);
                let modifiers = event.modifiers();
                // Ctrl/cmd picks one, shift extends from the last pick.
                if modifiers.secondary() {
                    this.toggle_selected(index, cx);
                } else if modifiers.shift && this.selection.is_active() {
                    this.select_through(index, cx);
                } else if this.selection.is_active() {
                    this.toggle_selected(index, cx);
                } else {
                    cx.emit(TrackListEvent::Play(index));
                }
            }))
            .on_pick(cx.listener(move |this, _: &bool, _, cx| this.toggle_selected(index, cx)))
            .menu(menu)
            .into_any_element()
    }
}

impl Render for TrackList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.rows.len();
        crate::scroll::smooth_list("track-list", &self.scroll, window, cx);
        v_flex()
            .size_full()
            // Without a handle here no list took arrows, Enter, select-all or Escape.
            .track_focus(&self.focus)
            .key_context("TrackList")
            .on_action(cx.listener(|this, _: &CursorDown, _, cx| this.move_cursor(1, cx)))
            .on_action(cx.listener(|this, _: &CursorUp, _, cx| this.move_cursor(-1, cx)))
            .on_action(cx.listener(|this, _: &ActivateRow, _, cx| this.activate_cursor(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &PlayNextSelection, _, cx| {
                this.emit_keys(TrackListEvent::PlayNext, cx)
            }))
            .on_action(cx.listener(|this, _: &QueueSelection, _, cx| {
                this.emit_keys(TrackListEvent::Queue, cx)
            }))
            .on_action(cx.listener(|this, _: &FavoriteSelection, _, cx| {
                this.emit_keys(TrackListEvent::ToggleFavorite, cx)
            }))
            .on_action(cx.listener(|this, _: &RemoveSelection, window, cx| {
                this.remove_selection(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearSelection, _, cx| {
                if this.selection.is_active() {
                    this.clear_selection(cx);
                } else {
                    cx.propagate();
                }
            }))
            .child(
                uniform_list(
                    "track-list",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        let columns = this.layout();
                        range.map(|index| this.row(index, &columns, cx)).collect()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .size_full(),
            )
    }
}

impl From<RowEdit> for TrackListEvent {
    fn from(edit: RowEdit) -> Self {
        match edit {
            RowEdit::Reorder { from, to } => TrackListEvent::Reorder { from, to },
            RowEdit::RemoveFromPlaylist(index) => TrackListEvent::RemoveFromPlaylist(index),
            RowEdit::Delete(keys) => TrackListEvent::Delete(keys),
        }
    }
}
