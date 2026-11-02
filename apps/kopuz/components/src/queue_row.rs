//! One row of the play queue: its number, cover and title, with the reorder buttons a hover brings out.

use std::rc::Rc;

use api::ArtworkRef;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Div, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Render, RenderOnce, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};
use store::{Pictured, Target};

use crate::format::t;
use crate::menus::{Picture, WithContextMenu as _};
use crate::reorder::ReorderDropZones;
use crate::{icons, lead, menus, motion, style, track_row};

const WASH: u64 = 1;
const TITLE: u64 = 2;
const LEAD: u64 = 3;

const LEAD_WIDTH: Pixels = px(16.);
const REORDER_BUTTON: Pixels = px(14.);
const REORDER_GLYPH: Pixels = px(9.);
const ROW_SUBTITLE: f32 = 0.65;
const ROW_GROUP: &str = "queue-row";

type Press = Rc<dyn Fn(&mut Window, &mut App)>;
type Wire = Box<dyn FnOnce(Stateful<Div>) -> Stateful<Div>>;

/// A queue row: playing or played or still to come, jumpable, draggable, and reorderable by button.
#[derive(IntoElement)]
pub struct QueueRow {
    index: usize,
    uid: SharedString,
    title: SharedString,
    artist: SharedString,
    artwork: Option<ArtworkRef>,
    covers: bool,
    active: bool,
    played: bool,
    last: bool,
    target: Option<Target>,
    jump: Option<Press>,
    up: Option<Press>,
    down: Option<Press>,
    remove: Option<Press>,
    drag: Option<Wire>,
    zones: Option<ReorderDropZones>,
}

impl QueueRow {
    /// The row at play-order `index`, whose track has the unique `uid` that keys its colours.
    pub fn new(
        index: usize,
        uid: impl Into<SharedString>,
        title: impl Into<SharedString>,
        artist: impl Into<SharedString>,
    ) -> Self {
        Self {
            index,
            uid: uid.into(),
            title: title.into(),
            artist: artist.into(),
            artwork: None,
            covers: false,
            active: false,
            played: false,
            last: false,
            target: None,
            jump: None,
            up: None,
            down: None,
            remove: None,
            drag: None,
            zones: None,
        }
    }

    /// The cover, drawn when the preference asks for row images.
    pub fn cover(mut self, artwork: Option<ArtworkRef>, shown: bool) -> Self {
        self.artwork = artwork;
        self.covers = shown;
        self
    }

    /// The track playing now.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Already played, so dimmed.
    pub fn played(mut self, played: bool) -> Self {
        self.played = played;
        self
    }

    /// The last row, which cannot move down.
    pub fn last(mut self, last: bool) -> Self {
        self.last = last;
        self
    }

    /// What the row's right-click menu is built for.
    pub fn menu(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    pub fn on_jump(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.jump = Some(Rc::new(press));
        self
    }

    pub fn on_move_up(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.up = Some(Rc::new(press));
        self
    }

    pub fn on_move_down(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.down = Some(Rc::new(press));
        self
    }

    pub fn on_remove(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.remove = Some(Rc::new(press));
        self
    }

    /// Carry `payload` when the row is dragged, for the drop strips of another row to take.
    pub fn drag<D: Clone + Render + 'static>(mut self, payload: D) -> Self {
        self.drag = Some(Box::new(move |row| {
            row.on_drag(payload, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
        }));
        self
    }

    pub fn drop_zones(mut self, zones: ReorderDropZones) -> Self {
        self.zones = Some(zones);
        self
    }
}

fn reorder_button(id: (&'static str, usize), glyph: &'static str, cx: &App) -> Button {
    Button::new(id)
        .custom(style::quiet_button(style::slate_500(cx), cx))
        .with_size(REORDER_GLYPH * (4. / 3.))
        .size(REORDER_BUTTON)
        .icon(icons::icon(glyph))
}

fn stopped(
    press: Option<Press>,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        cx.stop_propagation();
        if let Some(press) = &press {
            press(window, cx);
        }
    }
}

impl RenderOnce for QueueRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let index = self.index;
        let active = self.active;
        let raised = style::white(style::RAISED, cx);
        let accent = motion::accent(theme::Surface::Sheet, cx.theme().primary, window, cx);
        let readable = theme::readable_on(accent, cx.theme().background);
        let wash_target = if active {
            accent.opacity(style::PLAYING)
        } else {
            cx.theme().transparent
        };
        let lit_target = if active {
            readable
        } else if self.played {
            style::white(style::TEXT_MUTED, cx)
        } else {
            cx.theme().foreground
        };
        let lead_target = if active {
            readable
        } else {
            style::white(style::TEXT_SECONDARY, cx)
        };
        let wash = motion::color(motion::channel(&self.uid, WASH), wash_target, window, cx);
        let lit = motion::color(motion::channel(&self.uid, TITLE), lit_target, window, cx);
        let lead = motion::color(motion::channel(&self.uid, LEAD), lead_target, window, cx);
        let jump = self.jump;

        let row = div()
            .id(("queue-row", index))
            .group(ROW_GROUP)
            .relative()
            .h(style::media_row())
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .rounded(style::radius_lg())
            .bg(wash)
            .hover(move |row| row.bg(raised))
            .on_click(move |_, window, cx| {
                if let Some(jump) = &jump {
                    jump(window, cx);
                }
            });
        let row = match self.drag {
            Some(drag) => drag(row),
            None => row,
        };

        let picture = Picture::new(
            self.artwork.as_ref().filter(|_| self.covers),
            Pictured::Track {
                artist: self.artist.clone(),
                title: self.title.clone(),
            },
        );
        let spot = picture.as_ref().map(Picture::spot);
        let body = row
            .child(
                div()
                    .relative()
                    .w(LEAD_WIDTH)
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .items_end()
                    .text_xs()
                    .text_color(lead)
                    .children(lead::lead(
                        ROW_GROUP,
                        active,
                        Some((index + 1).to_string().into()),
                        lead,
                    )),
            )
            .when(self.covers, |row| {
                row.child(track_row::thumb(
                    self.artwork.as_ref(),
                    spot,
                    None,
                    window,
                    cx,
                ))
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_center()
                    .gap_0p5()
                    .child(track_row::title(("queue-title", index), self.title, lit))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(style::white(ROW_SUBTITLE, cx))
                            .child(self.artist),
                    ),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .pr_1()
                    .invisible()
                    .group_hover(ROW_GROUP, |buttons| buttons.visible())
                    .child(
                        reorder_button(("queue-up", index), icons::CHEVRON_UP, cx)
                            .tooltip(t("move_up"))
                            .disabled(index == 0)
                            .on_click(stopped(self.up)),
                    )
                    .child(
                        reorder_button(("queue-down", index), icons::CHEVRON_DOWN, cx)
                            .tooltip(t("move_down"))
                            .disabled(self.last)
                            .on_click(stopped(self.down)),
                    )
                    .child(
                        reorder_button(("queue-remove", index), icons::CLOSE, cx)
                            .tooltip(t("remove"))
                            .on_click(stopped(self.remove)),
                    ),
            )
            .children(self.zones);

        body.with_context_menu(menus::pictured(self.target.map(menus::menu_for), picture))
    }
}
