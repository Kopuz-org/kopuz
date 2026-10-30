use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use api::ArtworkRef;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, Pixels, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled,
    StyledImage as _, Window, div, img, linear_color_stop, linear_gradient, px, relative,
};

use crate::icon_button::{IconButton, IconTone};
use crate::icons;
use crate::menus::Spot;
use crate::motion;
use crate::style;

/// A black wash rising from a cover's foot, so the text laid over it stays legible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrim {
    /// The wash's opacity at the foot.
    pub foot: f32,
    /// Its opacity from `reach` up.
    pub top: f32,
    /// How far up the cover the wash fades, as a fraction of its height.
    pub reach: f32,
}

/// A row, tile or hero cover. A row without artwork renders the placeholder and asks for nothing.
#[derive(IntoElement)]
pub struct Cover {
    artwork: Option<ArtworkRef>,
    size: Pixels,
    hq: bool,
    rounded: Option<Pixels>,
    placeholder: &'static str,
    zoom: Option<(ElementId, bool)>,
    zoom_speed: Duration,
    zoom_factor: f32,
    fill: bool,
    scrim: Option<Scrim>,
    spot: Option<Spot>,
}

impl Cover {
    pub fn new(artwork: Option<&ArtworkRef>) -> Self {
        Self {
            artwork: artwork.cloned(),
            size: px(40.),
            hq: false,
            rounded: None,
            placeholder: icons::MUSIC,
            zoom: None,
            zoom_speed: motion::SLOW,
            zoom_factor: motion::ZOOM_SM,
            fill: false,
            scrim: None,
            spot: None,
        }
    }

    /// Record where the picture is drawn, so a right-click on it can offer to copy or save it.
    pub fn spot(mut self, spot: Option<Spot>) -> Self {
        self.spot = spot;
        self
    }

    /// The spot, only while there is a picture to offer.
    fn shown_spot(&self) -> Option<Spot> {
        self.spot.clone().filter(|_| self.artwork.is_some())
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        // Past a thumbnail, ask for the full-size picture.
        self.hq = size > px(512.);
        self
    }

    /// Ask for the full-size picture regardless of the rendered size.
    pub fn hq(mut self, hq: bool) -> Self {
        self.hq = hq;
        self
    }

    pub fn rounded(mut self, radius: Pixels) -> Self {
        self.rounded = Some(radius);
        self
    }

    /// The glyph shown when there is no picture: an artist gets a mic, a station a radio.
    pub fn placeholder(mut self, icon: &'static str) -> Self {
        self.placeholder = icon;
        self
    }

    /// `group-hover:scale-105`: the picture grows inside its own clipped frame while `hovered`.
    pub fn zoom(mut self, id: impl Into<ElementId>, hovered: bool) -> Self {
        self.zoom = Some((id.into(), hovered));
        self
    }

    /// How far it zooms; `scale-105` unless the original says otherwise.
    pub fn zoom_factor(mut self, factor: f32) -> Self {
        self.zoom_factor = factor;
        self
    }

    /// How long that zoom takes; `duration-300` unless the original says `duration-500`.
    pub fn zoom_speed(mut self, duration: Duration) -> Self {
        self.zoom_speed = duration;
        self
    }

    /// Cover the positioned parent edge to edge, as a banner or a card's backdrop; no placeholder.
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }

    /// A wash over the picture; a filled cover without artwork still draws it.
    pub fn scrim(mut self, scrim: Scrim) -> Self {
        self.scrim = Some(scrim);
        self
    }

    fn filled(self, window: &mut Window, cx: &mut App) -> AnyElement {
        // A fraction of the frame per side, so the bleed is a true scale whatever the frame's shape.
        let bleed = match self.zoom.clone() {
            Some((id, hovered)) => {
                let grown = motion::amount(
                    id,
                    if hovered { 1. } else { 0. },
                    self.zoom_speed,
                    window,
                    cx,
                );
                -(self.zoom_factor - 1.) / 2. * grown
            }
            None => 0.,
        };
        let hq = self.hq;
        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .when_some(self.rounded, |frame, radius| frame.rounded(radius))
            .when_some(self.artwork, |frame, artwork| {
                let shown = arrived(&artwork, hq, window, cx);
                frame.child(
                    img(store::artwork_source(&artwork, hq))
                        .opacity(shown)
                        .absolute()
                        .top(relative(bleed))
                        .left(relative(bleed))
                        .right(relative(bleed))
                        .bottom(relative(bleed))
                        .object_fit(ObjectFit::Cover),
                )
            })
            .when_some(self.scrim, |frame, scrim| {
                frame.child(div().absolute().inset_0().bg(linear_gradient(
                    0.,
                    linear_color_stop(style::black(scrim.foot, cx), 0.),
                    linear_color_stop(style::black(scrim.top, cx), scrim.reach),
                )))
            })
            .into_any_element()
    }
}

/// How far `artwork` has faded in: nothing until its picture is first drawn, then up over [`motion::FAST`].
pub fn arrived(artwork: &ArtworkRef, hq: bool, window: &mut Window, cx: &App) -> f32 {
    let Some(duration) = motion::live(motion::FAST, cx) else {
        return 1.;
    };
    match store::Residents::arrival(artwork, hq, cx) {
        store::Arrival::Waiting => 0.,
        store::Arrival::Settled => 1.,
        store::Arrival::Since(at) => {
            let shown = motion::progress(at.elapsed(), duration);
            if shown < 1. {
                window.request_animation_frame();
            }
            shown
        }
    }
}

/// Whether `artwork` has anything drawable yet, for a caller holding the old cover up until it does.
pub fn is_ready(artwork: Option<&ArtworkRef>, hq: bool, cx: &App) -> bool {
    artwork
        .is_none_or(|artwork| store::Residents::arrival(artwork, hq, cx) != store::Arrival::Waiting)
}

impl RenderOnce for Cover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.fill {
            return self.filled(window, cx);
        }
        // `rounded` on the thumb; a caller that wants a card or an avatar says so.
        let radius = self
            .rounded
            .unwrap_or_else(|| style::to_px(style::radius(cx), window));
        let glyph = px((self.size.as_f32() * 0.22).max(12.));
        let placeholder = self.placeholder;
        // The frame keeps its size and clips; only the picture inside it grows.
        let drawn = match self.zoom.clone() {
            Some((id, hovered)) => motion::zoom_by(
                id,
                self.size,
                self.zoom_factor,
                hovered,
                self.zoom_speed,
                window,
                cx,
            ),
            None => self.size,
        };
        let spot = self.shown_spot();
        let fell_back = spot.clone();
        div()
            .size(self.size)
            .rounded(radius)
            .overflow_hidden()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .bg(style::white(style::RAISED, cx))
            .map(|frame| match &self.artwork {
                Some(artwork) => frame.child({
                    let faint = style::white(style::TEXT_FAINT, cx);
                    let shown = arrived(artwork, self.hq, window, cx);
                    img(store::artwork_source(artwork, self.hq))
                        .opacity(shown)
                        .size(drawn)
                        .flex_shrink_0()
                        // The frame's mask is a rect, so the picture rounds its own corners.
                        .rounded(radius)
                        .object_fit(ObjectFit::Cover)
                        // A cover that will not load is otherwise a blank square, which reads as slow.
                        .with_fallback(move || {
                            if let Some(spot) = &fell_back {
                                spot.broken();
                            }
                            Icon::default()
                                .path(placeholder)
                                .size(glyph)
                                .text_color(faint)
                                .into_any_element()
                        })
                }),
                None => frame.child(
                    Icon::default()
                        .path(self.placeholder)
                        .size(glyph)
                        .text_color(style::white(style::TEXT_FAINT, cx)),
                ),
            })
            .when_some(spot, |frame, spot| {
                frame
                    .relative()
                    .on_prepaint(move |bounds, _, _| spot.drawn(bounds))
            })
            .into_any_element()
    }
}

type Act = Rc<dyn Fn(&mut Window, &mut App)>;

/// `opacity-0 hover:opacity-100`: the scrim and pencil an editable cover shows under the pointer.
const EDIT_SCRIM: f32 = 0.5;
const EDIT_GLYPH: Pixels = px(30.);
const REMOVE_BUTTON: Pixels = px(28.);
const EDIT_FRAME: u64 = 0;
const EDIT_HOVER: u64 = 1;
const EDIT_VEIL: u64 = 2;

/// A hero cover that takes a new picture on a click, and can give its current one back.
#[derive(IntoElement)]
pub struct EditableCover {
    id: SharedString,
    artwork: Option<ArtworkRef>,
    pending: Option<PathBuf>,
    size: Pixels,
    radius: Pixels,
    placeholder: &'static str,
    tooltip: Option<SharedString>,
    pick: Option<Act>,
    remove: Option<(SharedString, Act)>,
    spot: Option<Spot>,
}

impl EditableCover {
    pub fn new(id: impl Into<SharedString>, artwork: Option<&ArtworkRef>) -> Self {
        Self {
            id: id.into(),
            artwork: artwork.cloned(),
            pending: None,
            size: px(256.),
            radius: px(12.),
            placeholder: icons::DISC,
            tooltip: None,
            pick: None,
            remove: None,
            spot: None,
        }
    }

    /// A picked local file not yet sent anywhere, shown in place of the artwork.
    pub fn pending(mut self, path: Option<PathBuf>) -> Self {
        self.pending = path;
        self
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn rounded(mut self, radius: Pixels) -> Self {
        self.radius = radius;
        self
    }

    pub fn placeholder(mut self, icon: &'static str) -> Self {
        self.placeholder = icon;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// What a click does; without one the cover is only a picture.
    pub fn on_pick(mut self, pick: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.pick = Some(Rc::new(pick));
        self
    }

    /// Where the picture is drawn, for a right-click that offers to copy or save it.
    pub fn spot(mut self, spot: Option<Spot>) -> Self {
        self.spot = spot;
        self
    }

    /// A trash button over the corner that takes the picture away, named by `tooltip`.
    pub fn on_remove(
        mut self,
        tooltip: impl Into<SharedString>,
        remove: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.remove = Some((tooltip.into(), Rc::new(remove)));
        self
    }
}

impl RenderOnce for EditableCover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let (over, pointer) = motion::hovered(motion::channel(&id, EDIT_HOVER), window, cx);
        let veil = motion::amount(
            motion::channel(&id, EDIT_VEIL),
            if over { 1. } else { 0. },
            motion::FAST,
            window,
            cx,
        );
        let radius = self.radius;
        let has_artwork = self.artwork.is_some() || self.pending.is_some();
        let picture = match self.pending {
            Some(path) => img(path)
                .size(self.size)
                .rounded(radius)
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => Cover::new(self.artwork.as_ref())
                .size(self.size)
                .hq(true)
                .rounded(radius)
                .placeholder(self.placeholder)
                .spot(self.spot)
                .into_any_element(),
        };
        div()
            .id(motion::channel(&id, EDIT_FRAME))
            .relative()
            .flex_shrink_0()
            .on_hover(motion::track(pointer))
            .child(picture)
            .when_some(self.pick, |frame, pick| {
                frame
                    .cursor_pointer()
                    .when_some(self.tooltip, |frame, tip| {
                        frame.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })
                    .on_click(move |_, window, cx| pick(window, cx))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(radius)
                            .bg(style::black(EDIT_SCRIM, cx))
                            .opacity(veil)
                            .child(
                                icons::icon(icons::EDIT)
                                    .size(EDIT_GLYPH)
                                    .text_color(cx.theme().foreground),
                            ),
                    )
            })
            .when_some(
                self.remove.filter(|_| has_artwork),
                |frame, (tip, remove)| {
                    frame.child(
                        div().absolute().bottom_1().right_1().child(
                            IconButton::new(
                                SharedString::from(format!("{id}-remove")),
                                icons::TRASH,
                            )
                            .size(REMOVE_BUTTON)
                            .tone(IconTone::Scrim)
                            .tooltip(tip)
                            .stop_propagation(true)
                            .on_click(move |_, window, cx| remove(window, cx)),
                        ),
                    )
                },
            )
    }
}
