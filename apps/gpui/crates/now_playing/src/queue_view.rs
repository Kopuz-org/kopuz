//! The queue: what has been played, what is playing, and what is next, in one list.

use components::format::{duration, t};
use components::{
    EmptyState, Heading, Link, QueueRow, ReorderDropZones, SectionHeading, Tone, TrackDrag,
    dialogs, icons, style,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Render, ScrollStrategy, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled, Subscription, UniformListScrollHandle, Window, div, px, uniform_list,
};
use store::{ConfigModel, PlaylistsModel, QueueModel, Target};

// Above this point in a row a drop lands before it, below it after.
const DROP_SPLIT: Pixels = px(23.);
const TOOL_GLYPH: Pixels = px(13.);

const CLEAR: &str = "icons/list-x.svg";
const SAVE: &str = "icons/bookmark-plus.svg";

/// One line of the list: a section heading, or the queue row at that play-order index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    History(usize),
    Now,
    Next(usize),
    Track(usize),
}

/// A queue row dragged over the list, to reorder it in place.
#[derive(Clone)]
struct QueueRowDrag {
    from: u32,
    label: SharedString,
}

impl Render for QueueRowDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        components::DragChip::new(self.label.clone())
    }
}

/// An insertion point turned into the index `QueueEdit::Move` wants, or nothing when it is a no-op.
fn move_target(from: u32, insert_at: u32, count: u32) -> Option<u32> {
    if from >= count || insert_at > count {
        return None;
    }
    let to = if from < insert_at {
        insert_at.saturating_sub(1)
    } else {
        insert_at
    };
    (to < count && to != from).then_some(to)
}

fn insert_at(at: u32, keys: Vec<String>, cx: &mut App) {
    QueueModel::global(cx).update(cx, move |queue, cx| queue.insert(at, keys, cx));
}

/// The queue, with its history folded away and its upcoming rows under your hand.
pub struct QueueView {
    scroll: UniformListScrollHandle,
    lines: Vec<Line>,
    auto_follow: bool,
    history_open: bool,
    followed: Option<u32>,
    _subscriptions: Vec<Subscription>,
}

impl QueueView {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let queue = QueueModel::global(cx);
        let subscriptions = vec![
            cx.observe(&queue, |this, model, cx| {
                let position = model.read(cx).position();
                this.rebuild(cx);
                if position != this.followed {
                    this.followed = position;
                    this.follow();
                }
                cx.notify();
            }),
            cx.observe(&store::PlayerModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&ConfigModel::global(cx), |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            scroll: UniformListScrollHandle::new(),
            lines: Vec::new(),
            auto_follow: true,
            history_open: false,
            followed: None,
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
    }

    /// The section layout, recomputed where the queue changes rather than once a frame.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let queue = QueueModel::global(cx);
        let queue = queue.read(cx);
        let count = queue.len();
        let current = queue.position().map(|at| at as usize);
        self.lines.clear();
        let Some(current) = current else {
            self.lines.extend((0..count).map(Line::Track));
            return;
        };
        if current > 0 {
            self.lines.push(Line::History(current));
            if self.history_open {
                self.lines.extend((0..current).map(Line::Track));
            }
        }
        self.lines.push(Line::Now);
        self.lines.push(Line::Track(current));
        let upcoming = count.saturating_sub(current + 1);
        if upcoming > 0 {
            self.lines.push(Line::Next(upcoming));
            self.lines.extend((current + 1..count).map(Line::Track));
        }
    }

    fn follow(&self) {
        let Some(index) = self.followed else { return };
        if !self.auto_follow {
            return;
        }
        if let Some(line) = self
            .lines
            .iter()
            .position(|line| *line == Line::Track(index as usize))
        {
            self.scroll
                .scroll_to_item_strict(line, ScrollStrategy::Center);
        }
    }

    fn rearm(&mut self, cx: &mut Context<Self>) {
        self.auto_follow = true;
        self.follow();
        cx.notify();
    }

    fn clear_upcoming(&mut self, cx: &mut Context<Self>) {
        let model = QueueModel::global(cx);
        let keys: Vec<String> = model
            .read(cx)
            .upcoming()
            .iter()
            .map(|track| track.key.clone())
            .collect();
        if keys.is_empty() {
            return;
        }
        let count = keys.len();
        model.update(cx, |queue, cx| queue.clear_upcoming(cx));
        store::Notices::undo(
            components::format::t_with("cleared_upcoming", &[("count", count.into())]),
            move |cx| {
                let keys = keys.clone();
                QueueModel::global(cx).update(cx, |queue, cx| queue.replace_upcoming(keys, cx));
            },
            cx,
        );
    }

    fn shuffle_upcoming(&mut self, cx: &mut Context<Self>) {
        QueueModel::global(cx).update(cx, |queue, cx| queue.shuffle_upcoming(cx));
    }

    fn save_as_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<String> = QueueModel::global(cx)
            .read(cx)
            .items()
            .iter()
            .map(|track| track.key.clone())
            .collect();
        if keys.is_empty() {
            return;
        }
        dialogs::open_rename(
            t("save_queue_as_playlist"),
            t("up_next"),
            move |name, cx| {
                let keys = keys.clone();
                PlaylistsModel::global(cx)
                    .update(cx, |playlists, cx| playlists.create(name, keys, cx));
            },
            window,
            cx,
        );
    }

    /// The row a list slot holds: a heading, or the queue row at that play-order index.
    fn line(&self, slot: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.lines.get(slot).copied() {
            Some(Line::Track(index)) => self.row(index, cx),
            Some(Line::History(count)) => self.history_heading(count, cx),
            Some(Line::Now) => heading(SectionHeading::new(t("queue_now"))).into_any_element(),
            Some(Line::Next(count)) => {
                heading(SectionHeading::new(t("queue_next_up")).count(count)).into_any_element()
            }
            None => div().into_any_element(),
        }
    }

    fn history_heading(&self, count: usize, cx: &mut Context<Self>) -> AnyElement {
        let glyph = if self.history_open {
            icons::CHEVRON_DOWN
        } else {
            icons::CHEVRON_RIGHT
        };
        heading(
            SectionHeading::new(t("queue_history")).count(count).beside(
                icons::icon(glyph)
                    .size(components::lead::GLYPH)
                    .text_color(style::slate_500(cx)),
            ),
        )
        .id("queue-history-heading")
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            this.history_open = !this.history_open;
            this.rebuild(cx);
            cx.notify();
        }))
        .into_any_element()
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let model = QueueModel::global(cx);
        let queue = model.read(cx);
        let Some(track) = queue.items().get(index) else {
            return div().into_any_element();
        };
        let count = queue.len() as u32;
        let at = index as u32;
        let title = SharedString::from(track.title.clone());
        let zones = ReorderDropZones::new(("queue-drop", index), style::media_row(), index)
            .split(DROP_SPLIT)
            .on_drop::<TrackDrag>(move |drag, at, _, cx| {
                insert_at(at as u32, drag.keys.to_vec(), cx)
            })
            .on_drop::<QueueRowDrag>(move |drag, to, _, cx| {
                if let Some(to) = move_target(drag.from, to as u32, count) {
                    let from = drag.from;
                    QueueModel::global(cx)
                        .update(cx, move |queue, cx| queue.move_item(from, to, cx));
                }
            });

        QueueRow::new(
            index,
            track.uid.clone(),
            track.title.clone(),
            track.artist.clone(),
        )
        .cover(
            track.artwork.clone(),
            ConfigModel::global(cx).read(cx).row_images(),
        )
        .active(queue.position() == Some(at))
        .played(queue.position().is_some_and(|position| at < position))
        .last(at + 1 >= count)
        .menu(Target::QueueItem {
            index: at,
            key: SharedString::from(track.key.clone()),
        })
        .on_jump(move |_, cx| QueueModel::global(cx).update(cx, |queue, cx| queue.jump(at, cx)))
        .on_move_up(move |_, cx| {
            QueueModel::global(cx).update(cx, |queue, cx| {
                queue.move_item(at, at.saturating_sub(1), cx)
            });
        })
        .on_move_down(move |_, cx| {
            QueueModel::global(cx).update(cx, |queue, cx| queue.move_item(at, at + 1, cx));
        })
        .on_remove(move |_, cx| store::actions::queue_remove(at, cx))
        .drag(QueueRowDrag {
            from: at,
            label: title,
        })
        .drop_zones(zones)
        .into_any_element()
    }

    /// "3/57 • 3:41:02", then everything that can be done to the rest of the queue.
    fn toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let _ = window;
        let model = QueueModel::global(cx);
        let queue = model.read(cx);
        let radio = queue.current().is_some_and(|track| track.is_radio());
        let summary = queue.summary();
        let upcoming = queue.upcoming().len();
        let empty = queue.is_empty();
        let place = format!("{}/{}", summary.position.max(1), summary.count);
        let label = SharedString::from(match summary.total_ms {
            Some(total) => format!("{place} • {}", duration(total)),
            None => place,
        });

        h_flex()
            .w_full()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .pt_1()
            .pb_2()
            .child(
                Link::new("queue-summary")
                    .label(if radio { t("radio") } else { label })
                    .tone(Tone::Quiet)
                    .tooltip(Some(t("jump_to_current_song")))
                    .on_click(cx.listener(|this, _, _, cx| this.rearm(cx)))
                    .flex_1()
                    .min_w_0()
                    .text_size(style::text_11()),
            )
            .child(
                tool("queue-shuffle-upcoming", icons::SHUFFLE, cx)
                    .tooltip(t("shuffle_upcoming"))
                    .disabled(upcoming < 2)
                    .on_click(cx.listener(|this, _, _, cx| this.shuffle_upcoming(cx))),
            )
            .child(
                tool("queue-clear-upcoming", CLEAR, cx)
                    .tooltip(t("clear_upcoming"))
                    .disabled(upcoming == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.clear_upcoming(cx))),
            )
            .child(
                tool("queue-save", SAVE, cx)
                    .tooltip(t("save_queue_as_playlist"))
                    .disabled(empty)
                    .on_click(cx.listener(|this, _, window, cx| this.save_as_playlist(window, cx))),
            )
    }
}

impl Render for QueueView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        components::scroll::smooth_list("queue-scroll", &self.scroll, window, cx);
        let count = QueueModel::global(cx).read(cx).len();
        if count == 0 {
            return v_flex()
                .size_full()
                .child(EmptyState::new(t("no_more_songs")).icon(icons::LIST))
                .into_any_element();
        }
        let end = count as u32;
        let lines = self.lines.len();

        v_flex()
            .relative()
            .size_full()
            .child(self.toolbar(window, cx))
            .child(
                uniform_list(
                    "queue-list",
                    lines,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|slot| this.line(slot, cx)).collect()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0()
                .px_2()
                .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                    if this.auto_follow {
                        this.auto_follow = false;
                        cx.notify();
                    }
                }))
                .on_drop::<TrackDrag>(move |drag, _, cx| insert_at(end, drag.keys.to_vec(), cx)),
            )
            .when(!self.auto_follow, |panel| {
                panel.child(
                    components::FollowButton::new("queue-jump-to-current")
                        .on_click(cx.listener(|this, _, _, cx| this.rearm(cx))),
                )
            })
            .into_any_element()
    }
}

/// A section heading, drawn at row height so the whole list can stay one `uniform_list`.
fn heading(heading: SectionHeading) -> Stateful<Div> {
    h_flex()
        .id("queue-heading")
        .h(style::media_row())
        .w_full()
        .items_end()
        .px_2()
        .pb_2()
        .child(heading.heading(Heading::Caption))
}

fn tool(id: &'static str, glyph: &'static str, cx: &App) -> Button {
    Button::new(id)
        .custom(style::quiet_button(style::slate_500(cx), cx))
        .with_size(TOOL_GLYPH * (4. / 3.))
        .size(style::chrome().tool_button)
        .rounded(style::chrome().tool_button / 2.)
        .icon(icons::icon(glyph))
}

#[cfg(test)]
mod tests {
    use super::move_target;

    #[test]
    fn an_insertion_point_below_the_row_loses_the_row_itself() {
        assert_eq!(move_target(0, 3, 5), Some(2));
        assert_eq!(move_target(4, 1, 5), Some(1));
    }

    #[test]
    fn dropping_a_row_back_where_it_was_moves_nothing() {
        assert_eq!(move_target(2, 2, 5), None);
        assert_eq!(move_target(2, 3, 5), None);
        assert_eq!(move_target(5, 1, 5), None);
    }
}
