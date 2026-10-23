//! The one track row every table draws, and the lead, thumb, title and link cells it is built from.

use std::rc::Rc;

use api::{ArtworkRef, TrackInfo};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Div, ElementId, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, Length, ParentElement as _, RenderOnce, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use store::{Pictured, Route};

use crate::columns::{self, Layout};
use crate::cover::Cover;
use crate::format::known_duration;
use crate::icons;
use crate::lead;
use crate::link::{Link, Tone};
use crate::menu_button::MenuButton;
use crate::menus::{self, Picture, Spot, WithContextMenu as _};
use crate::motion;
use crate::style;
use crate::track_list::TrackDrag;

// The row is the hover group, so the number can step aside for the play glyph.
const ROW_GROUP: &str = "track-row";

/// What a press on a row does.
pub type OnRowClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
/// What ticking a row's box does.
pub type OnPick = Rc<dyn Fn(&bool, &mut Window, &mut App)>;
pub use crate::menus::Menu as RowMenu;

/// A track on its table's columns; whoever holds the rows says what is picked, playing and pressed.
#[derive(IntoElement)]
pub struct TrackRow {
    slot: usize,
    title: SharedString,
    artist: SharedString,
    album: SharedString,
    artist_route: Option<Route>,
    album_route: Option<Route>,
    artwork: Option<ArtworkRef>,
    length: SharedString,
    format: SharedString,
    plays: u64,
    columns: Layout,
    height: Length,
    lead: Option<SharedString>,
    cover: bool,
    stored: bool,
    cached: bool,
    playing: bool,
    selected: bool,
    picking: bool,
    cursor: bool,
    drag: Option<TrackDrag>,
    click: Option<OnRowClick>,
    pick: Option<OnPick>,
    menu: Option<RowMenu>,
}

impl TrackRow {
    /// `slot` is where the row sits in its list, which keys its tweens and its menu's open flag.
    pub fn new(track: &TrackInfo, slot: usize) -> Self {
        Self {
            slot,
            title: track.title.clone().into(),
            artist: track.artist.clone().into(),
            album: track.album.clone().into(),
            artist_route: Route::artist_of(track),
            album_route: (!track.album_id.is_empty())
                .then(|| Route::Album(track.album_id.clone().into())),
            artwork: track.artwork.clone(),
            length: known_duration(track.duration_secs().map(|secs| secs * 1000)),
            format: track.format.clone().unwrap_or_default().into(),
            plays: 0,
            columns: columns::standard(true),
            height: style::row_height().into(),
            lead: None,
            cover: true,
            stored: false,
            cached: track.cached,
            playing: false,
            selected: false,
            picking: false,
            cursor: false,
            drag: None,
            click: None,
            pick: None,
            menu: None,
        }
    }

    pub fn columns(mut self, columns: Layout) -> Self {
        self.columns = columns;
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// The number in the lead cell; none leaves it blank until the row plays.
    pub fn lead(mut self, lead: Option<SharedString>) -> Self {
        self.lead = lead;
        self
    }

    /// Whether an offline copy exists, shown as a corner mark on the cover.
    pub fn stored(mut self, stored: bool) -> Self {
        self.stored = stored;
        self
    }

    pub fn cover(mut self, cover: bool) -> Self {
        self.cover = cover;
        self
    }

    /// What the plays column reads, when it is shown.
    pub fn plays(mut self, plays: u64) -> Self {
        self.plays = plays;
        self
    }

    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// A selection is being built, so the lead is a box to tick.
    pub fn picking(mut self, picking: bool) -> Self {
        self.picking = picking;
        self
    }

    /// The keyboard is on this row.
    pub fn cursor(mut self, cursor: bool) -> Self {
        self.cursor = cursor;
        self
    }

    /// What dragging the row carries, such as the selection it is part of.
    pub fn drag(mut self, drag: TrackDrag) -> Self {
        self.drag = Some(drag);
        self
    }

    pub fn on_click(
        mut self,
        click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some(Rc::new(click));
        self
    }

    pub fn on_pick(mut self, pick: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.pick = Some(Rc::new(pick));
        self
    }

    pub fn menu(mut self, menu: RowMenu) -> Self {
        self.menu = Some(menu);
        self
    }
}

impl RenderOnce for TrackRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        profile::elements::built();
        let slot = self.slot;
        let accent = cx.theme().primary;
        // `transition-colors` on the row: the playing tint and the selection wash tween, hover does not.
        let wash = motion::color(
            motion::row_id("track-row-wash", slot),
            match (self.playing, self.selected) {
                (true, _) => style::accent(style::PLAYING, cx),
                (false, true) => style::white(style::SELECTED, cx),
                (false, false) => style::white(0., cx),
            },
            window,
            cx,
        );
        let title_ink = motion::color(
            motion::row_id("track-row-title", slot),
            match self.playing {
                true => accent,
                false => style::white(style::TEXT_STRONG, cx),
            },
            window,
            cx,
        );
        let mark = CopyMark::of(self.stored, self.cached);
        let opacity = motion::amount(
            motion::row_id("track-row-badge", slot),
            if mark.is_some() && !self.playing {
                1.
            } else {
                0.
            },
            motion::FAST,
            window,
            cx,
        );
        let badge = mark.map(|mark| Badge {
            id: motion::row_id("track-row-mark", slot),
            mark,
            opacity,
        });
        let picture = Picture::new(
            self.artwork.as_ref().filter(|_| self.cover),
            Pictured::Track {
                artist: self.artist.clone(),
                title: self.title.clone(),
            },
        );
        let spot = picture.as_ref().map(Picture::spot);
        let mut cover = self
            .cover
            .then(|| thumb(self.artwork.as_ref(), spot, badge, window, cx).mr_4());
        let lead_ink = match self.playing {
            true => accent,
            false => style::white(style::TEXT_SECONDARY, cx),
        };

        let mut row = columns::frame()
            .id(("track-row", slot))
            .group(ROW_GROUP)
            .h(self.height)
            .rounded(style::radius_lg())
            .bg(wash)
            .when_some(self.drag, |row, drag| {
                row.on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            })
            // Upstream the wash is an inline style, which outranks `hover:bg-white/5`; a refinement does not.
            .when(!self.playing && !self.selected, |row| {
                row.hover(|row| row.bg(style::white(style::RAISED, cx)))
            })
            .when(self.cursor, |row| {
                row.border_1().border_color(cx.theme().ring)
            })
            .when_some(self.click, |row, click| {
                row.on_click(move |event, window, cx| click(event, window, cx))
            })
            .child(
                columns::fixed(columns::INDEX)
                    .relative()
                    .justify_center()
                    .map(|cell| match (self.picking, self.pick) {
                        (true, Some(pick)) => cell.child(
                            Checkbox::new(("track-pick", slot))
                                .checked(self.selected)
                                .on_click(move |checked, window, cx| pick(checked, window, cx)),
                        ),
                        _ => {
                            cell.children(lead::lead(ROW_GROUP, self.playing, self.lead, lead_ink))
                        }
                    }),
            );

        for shown in self.columns.iter() {
            let cell = columns::sized(shown).when(shown.spec.align_end, |cell| cell.justify_end());
            row = row.child(match shown.spec.id {
                "title" => cell
                    .children(cover.take())
                    .child(title(("track-title", slot), self.title.clone(), title_ink).flex_1())
                    .into_any_element(),
                "artist" => link(
                    cell,
                    ("track-artist", slot),
                    self.artist.clone(),
                    self.artist_route.clone(),
                ),
                "album" => link(
                    cell,
                    ("track-album", slot),
                    self.album.clone(),
                    self.album_route.clone(),
                ),
                "plays" => dim(cell, cx)
                    .child(self.plays.to_string())
                    .into_any_element(),
                "format" => dim(cell, cx).child(self.format.clone()).into_any_element(),
                // Tabular digits, so the lengths line up rather than wander by glyph width.
                _ => dim(cell, cx)
                    .font_family(SharedString::new_static("monospace"))
                    .child(self.length.clone())
                    .into_any_element(),
            });
        }

        let row = row.child(
            columns::fixed(columns::MENU)
                .id(("track-row-menu", slot))
                .justify_end()
                // The popover stops the press; this stops the rest of the cell playing the row.
                .on_click(|_, _, cx| cx.stop_propagation())
                .when_some(self.menu.clone(), |cell, menu| {
                    cell.child(
                        MenuButton::new(format!("track-row-dots-{slot}"), menu)
                            .reveal_in_group(ROW_GROUP),
                    )
                }),
        );
        row.with_context_menu(menus::pictured(self.menu, picture))
    }
}

/// Which copy a row's corner mark says the track has; a download outranks a cached copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMark {
    Downloaded,
    Cached,
}

impl CopyMark {
    /// The mark a track wears, if any: a download wins over the cache.
    pub fn of(stored: bool, cached: bool) -> Option<Self> {
        match (stored, cached) {
            (true, _) => Some(Self::Downloaded),
            (false, true) => Some(Self::Cached),
            (false, false) => None,
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Downloaded => icons::CHECK,
            Self::Cached => icons::HARD_DRIVE,
        }
    }

    fn label(self) -> SharedString {
        crate::format::t(match self {
            Self::Downloaded => "offline_mark",
            Self::Cached => "cached_mark",
        })
    }
}

/// A corner mark as drawn: which, how faded in, and the id its tooltip hangs off.
pub struct Badge {
    pub id: ElementId,
    pub mark: CopyMark,
    pub opacity: f32,
}

/// `bg-black/60` behind the cached mark: quieter than a download's accent, legible on any cover.
const CACHED_FILL: f32 = 0.60;

/// A row's cover, with the corner mark saying it has a downloaded or cached copy.
pub fn thumb(
    artwork: Option<&ArtworkRef>,
    spot: Option<Spot>,
    badge: Option<Badge>,
    window: &Window,
    cx: &App,
) -> Div {
    div()
        .relative()
        .flex_shrink_0()
        .child(
            Cover::new(artwork)
                .size(style::to_px(style::row_cover(), window))
                .rounded(style::thumb_radius())
                .spot(spot),
        )
        .when_some(badge.filter(|badge| badge.opacity > 0.), |thumb, badge| {
            let fill = match badge.mark {
                CopyMark::Downloaded => cx.theme().primary,
                CopyMark::Cached => style::black(CACHED_FILL, cx),
            };
            let label = badge.mark.label();
            thumb.child(
                div()
                    .id(badge.id)
                    .absolute()
                    .bottom_0()
                    .right_0()
                    .size_3()
                    .opacity(badge.opacity)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_tl(style::radius(cx))
                    .bg(fill)
                    .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                    .child(
                        icons::icon(badge.mark.glyph())
                            .size(px(6.))
                            .text_color(cx.theme().foreground),
                    ),
            )
        })
}

/// A row's title line, whose tooltip recovers what truncation cuts off.
pub fn title(id: impl Into<ElementId>, title: SharedString, ink: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .min_w_0()
        .truncate()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink)
        .tooltip({
            let title = title.clone();
            move |window, cx| Tooltip::new(title.clone()).build(window, cx)
        })
        .child(title)
}

/// An artist or album cell that opens what it names, and brightens to say so.
fn link(
    cell: Div,
    id: impl Into<ElementId>,
    value: SharedString,
    route: Option<Route>,
) -> AnyElement {
    cell.child(
        Link::new(id)
            .label(value.clone())
            .route(route)
            .tone(Tone::Dim)
            // Without this the row's own click would play the track too.
            .stop_propagation(true)
            .tooltip(Some(value))
            .flex_1()
            .min_w_0()
            .text_sm(),
    )
    .into_any_element()
}

fn dim(cell: Div, cx: &App) -> Div {
    cell.text_xs().text_color(style::white(style::TEXT_DIM, cx))
}

#[cfg(test)]
mod tests {
    use super::CopyMark;

    #[test]
    fn a_download_outranks_a_cached_copy() {
        assert_eq!(CopyMark::of(true, true), Some(CopyMark::Downloaded));
        assert_eq!(CopyMark::of(true, false), Some(CopyMark::Downloaded));
        assert_eq!(CopyMark::of(false, true), Some(CopyMark::Cached));
        assert_eq!(CopyMark::of(false, false), None);
    }
}
