//! What the player says about the track in hand: its title, who made it, where the queue came from, its marks.

use std::rc::Rc;

use gpui_kit::base::ElementExt as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString, Styled as _, Window, div,
    px, rems,
};
use store::{Route, Target};

use crate::format::t;
use crate::icon_button::{IconButton, IconTone};
use crate::link::{Link, Tone};
use crate::menu_button::{MenuButton, MenuVariant};
use crate::{icons, menus, motion, style};

const ROOM: u64 = 1;
const HOVER: u64 = 2;

const MARQUEE_PERIOD: std::time::Duration = std::time::Duration::from_millis(6_000);

/// What `text_sm` is in rems, which is the size the title is drawn at and so the size it is measured at.
const TITLE_REMS: f32 = 0.875;

type Press = Rc<dyn Fn(&mut Window, &mut App)>;

/// What the title measures as drawn, so overflow is a fact rather than a guess at a character count.
fn title_width(title: &SharedString, scale: f32, window: &Window, cx: &App) -> Pixels {
    let size = rems(TITLE_REMS * scale).to_pixels(window.rem_size());
    let run = gpui_kit::TextRun {
        len: title.len(),
        font: gpui_kit::Font {
            weight: FontWeight::BOLD,
            ..gpui_kit::font(cx.theme().font_family.clone())
        },
        color: cx.theme().foreground,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(title.clone(), size, &[run], None)
        .width
}

/// A title too long for its column walks past the pointer, and carries the whole of itself in a tooltip.
#[derive(IntoElement)]
pub struct MarqueeText {
    id: SharedString,
    text: SharedString,
    route: Option<Route>,
    scale: f32,
}

impl MarqueeText {
    pub fn new(id: impl Into<SharedString>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            route: None,
            scale: 1.,
        }
    }

    /// The title drawn and measured at `scale` times its size.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Where a click on the title goes; none leaves it inert.
    pub fn route(mut self, route: Option<Route>) -> Self {
        self.route = route;
        self
    }
}

impl RenderOnce for MarqueeText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let text = self.text;
        let room = window.use_keyed_state(motion::channel(&id, ROOM), cx, |_, _| Pixels::ZERO);
        let natural = title_width(&text, self.scale, window, cx);
        // How far past its column the title runs, which is exactly how far it has to walk.
        let overflow = (natural - *room.read(cx)).max(px(0.));
        let long = overflow > px(0.);
        let (over, pointer) = motion::hovered(motion::channel(&id, HOVER), window, cx);
        let running = long && over;
        let shown = text.clone();
        let walk = SharedString::from(format!("{id}-marquee"));

        div()
            .id(SharedString::from(format!("{id}-frame")))
            .min_w_0()
            .overflow_hidden()
            // Prepaint runs every frame, so a width that has not moved must not dirty the window.
            .on_prepaint(move |bounds, _, cx| {
                room.update(cx, |room, cx| {
                    if *room != bounds.size.width {
                        *room = bounds.size.width;
                        cx.notify();
                    }
                })
            })
            .child(
                Link::new(id.clone())
                    .route(self.route)
                    .underline(true)
                    .hover_state(pointer)
                    .tooltip(long.then_some(text))
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_size(rems(TITLE_REMS * self.scale))
                            .font_weight(FontWeight::BOLD)
                            .text_color(style::white(style::TEXT_STRONG, cx))
                            .map(|label| {
                                if running {
                                    label
                                        .flex_shrink_0()
                                        .whitespace_nowrap()
                                        .child(shown)
                                        .with_animation(
                                            walk,
                                            Animation::new(MARQUEE_PERIOD).repeat(),
                                            move |label, progress| {
                                                // There and back over exactly what does not fit.
                                                let travel = 1. - (1. - 2. * progress).abs();
                                                label.ml(-overflow * travel)
                                            },
                                        )
                                        .into_any_element()
                                } else {
                                    label.truncate().child(shown).into_any_element()
                                }
                            }),
                    ),
            )
    }
}

/// "Playing from <album>": the only line that says how you got to the track.
#[derive(IntoElement)]
pub struct PlayingFromLink {
    id: SharedString,
    label: SharedString,
    route: Route,
    hero: bool,
    scale: f32,
    before: Option<Press>,
}

impl PlayingFromLink {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, route: Route) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            route,
            hero: false,
            scale: 1.,
            before: None,
        }
    }

    /// Drawn for the sheet, at `text-xs` and underlined, rather than the bar's 10px line.
    pub fn hero(mut self, hero: bool) -> Self {
        self.hero = hero;
        self
    }

    /// The bar line drawn at `scale` times its size.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub fn on_before_navigate(mut self, before: Option<Press>) -> Self {
        self.before = before;
        self
    }
}

impl RenderOnce for PlayingFromLink {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let hero = self.hero;
        let scale = self.scale;
        let before = self.before;
        Link::new(self.id)
            .route(Some(self.route))
            .tone(Tone::Faint)
            .brighten(!hero)
            .underline(hero)
            .when_some(before, |link, before| {
                link.on_before_navigate(move |window, cx| before(window, cx))
            })
            .flex()
            .items_center()
            .gap_1()
            .map(|link| match hero {
                true => link.max_w_full().text_xs(),
                false => link.min_w_0().text_size(style::text_10() * scale),
            })
            .child(div().flex_shrink_0().child(t("playing_from")))
            .child(div().truncate().child(self.label))
    }
}

/// How one surface sets the track's text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MetaLayout {
    /// The bar's block: a marquee title over small links.
    Bar,
    /// The sheet's: a large centred title over the artist and album.
    Hero,
    /// The mini window's: a truncated bold title over a dim artist.
    Mini,
    /// A card's: a large title with the artist and a note beside it.
    Card,
}

/// The track's title, artist and where the queue came from, laid out for one surface.
#[derive(IntoElement)]
pub struct NowPlayingMeta {
    id: SharedString,
    track: api::TrackInfo,
    layout: MetaLayout,
    origin: Option<(SharedString, Route)>,
    eyebrow: Option<SharedString>,
    note: Option<SharedString>,
    footer: Option<AnyElement>,
    before: Option<Press>,
    scale: f32,
}

impl NowPlayingMeta {
    pub fn new(id: impl Into<SharedString>, track: &api::TrackInfo) -> Self {
        Self {
            id: id.into(),
            track: track.clone(),
            layout: MetaLayout::Bar,
            origin: None,
            eyebrow: None,
            note: None,
            footer: None,
            before: None,
            scale: 1.,
        }
    }

    pub fn layout(mut self, layout: MetaLayout) -> Self {
        self.layout = layout;
        self
    }

    /// The bar's text drawn at `scale` times its size; the other layouts keep theirs.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// Where the queue came from, as the label to show and the route it opens.
    pub fn origin(mut self, origin: Option<(SharedString, Route)>) -> Self {
        self.origin = origin;
        self
    }

    /// A small line over a card's title.
    pub fn eyebrow(mut self, eyebrow: impl Into<SharedString>) -> Self {
        self.eyebrow = Some(eyebrow.into());
        self
    }

    /// A dim line after a card's artist, such as how much of the queue is left.
    pub fn note(mut self, note: Option<SharedString>) -> Self {
        self.note = note;
        self
    }

    /// Something under the text, such as the mini window's clock.
    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    /// Run before a link opens its page, so a sheet can fold away first.
    pub fn on_before_navigate(mut self, before: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.before = Some(Rc::new(before));
        self
    }
}

fn leave(link: Link, before: &Option<Press>) -> Link {
    match before.clone() {
        Some(before) => link.on_before_navigate(move |window, cx| before(window, cx)),
        None => link,
    }
}

impl RenderOnce for NowPlayingMeta {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let track = self.track;
        let title = SharedString::from(track.title.clone());
        let artist = SharedString::from(track.artist.clone());
        let album = SharedString::from(track.album.clone());
        let album_route =
            (!track.album_id.is_empty()).then(|| Route::Album(track.album_id.clone().into()));
        let artist_route = Route::artist_of(&track);
        let id = self.id;
        let key = |name: &str| SharedString::from(format!("{id}-{name}"));
        let before = self.before;
        let scale = match self.layout {
            MetaLayout::Bar => self.scale,
            _ => 1.,
        };
        let origin = self.origin.map(|(label, route)| {
            PlayingFromLink::new(key("playing-from"), label, route)
                .hero(self.layout == MetaLayout::Hero)
                .scale(scale)
                .on_before_navigate(before.clone())
        });

        match self.layout {
            MetaLayout::Bar => v_flex()
                // Sized by its text, not by the room going spare: the marks belong beside it.
                .min_w_0()
                .justify_center()
                // `gap-0.5` and `text-xs`, at the bar's scale.
                .gap(rems(0.125 * scale))
                .child(
                    MarqueeText::new(key("title"), title)
                        .route(album_route)
                        .scale(scale),
                )
                .child(
                    Link::new(key("artist"))
                        .label(artist)
                        .route(artist_route)
                        .tone(Tone::Muted)
                        .underline(true)
                        .text_size(rems(0.75 * scale)),
                )
                .children(origin)
                .into_any_element(),
            MetaLayout::Hero => v_flex()
                .w_full()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .w_full()
                        .text_center()
                        .text_size(rems(1.75))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_clamp(2)
                        .child(title),
                )
                .child(
                    h_flex()
                        .max_w_full()
                        .gap_2()
                        .items_center()
                        .justify_center()
                        .text_lg()
                        .text_color(style::white(0.7, cx))
                        .child(leave(
                            Link::new(key("artist"))
                                .label(artist)
                                .route(artist_route)
                                .underline(true),
                            &before,
                        ))
                        .when(!album.is_empty(), |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(style::white(style::TEXT_DIM, cx))
                                    .child("•"),
                            )
                            .child(leave(
                                Link::new(key("album"))
                                    .label(album)
                                    .route(album_route)
                                    .tone(Tone::Faint)
                                    .underline(true),
                                &before,
                            ))
                        }),
                )
                .children(origin)
                .into_any_element(),
            MetaLayout::Mini => v_flex()
                .flex_1()
                .min_w_0()
                .justify_center()
                .gap_0p5()
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(style::white(0.95, cx))
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(style::text_11())
                        .text_color(style::white(0.55, cx))
                        .child(artist),
                )
                .children(self.footer)
                .into_any_element(),
            MetaLayout::Card => v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .children(self.eyebrow.map(|eyebrow| {
                    div()
                        .text_size(style::text_10())
                        .font_weight(FontWeight::BOLD)
                        .text_color(cx.theme().primary_hover)
                        .child(eyebrow)
                }))
                .child(
                    div()
                        .truncate()
                        .text_2xl()
                        .line_height(gpui_kit::relative(1.2))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(style::white(style::TEXT_SECONDARY, cx))
                                .child(artist),
                        )
                        .children(self.note.map(|note| {
                            div()
                                .text_sm()
                                .text_color(style::white(style::TEXT_DIM, cx))
                                .child(note)
                        })),
                )
                .into_any_element(),
        }
    }
}

/// Favourite, don't-recommend, share and the track's menu: whichever of them a surface keeps.
#[derive(IntoElement)]
pub struct TrackMarks {
    id: SharedString,
    size: Pixels,
    glyph: Pixels,
    gap: Pixels,
    tone: IconTone,
    favorite: Option<bool>,
    dont_recommend: bool,
    share: bool,
    menu: Option<Target>,
    disabled: bool,
    on_favorite: Option<Press>,
    on_dont_recommend: Option<Press>,
    on_share: Option<Press>,
}

impl TrackMarks {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            size: px(32.),
            glyph: px(16.),
            gap: px(8.),
            tone: IconTone::Quiet,
            favorite: None,
            dont_recommend: false,
            share: false,
            menu: None,
            disabled: false,
            on_favorite: None,
            on_dont_recommend: None,
            on_share: None,
        }
    }

    pub fn size(mut self, size: Pixels, glyph: Pixels) -> Self {
        self.size = size;
        self.glyph = glyph;
        self
    }

    pub fn gap(mut self, gap: Pixels) -> Self {
        self.gap = gap;
        self
    }

    pub fn tone(mut self, tone: IconTone) -> Self {
        self.tone = tone;
        self
    }

    /// The heart, with whether the track is a favourite; `None` leaves it out.
    pub fn favorite(mut self, favorite: Option<bool>) -> Self {
        self.favorite = favorite;
        self
    }

    /// The thumb-down, for a source whose recommendations can be told.
    pub fn dont_recommend(mut self, shown: bool) -> Self {
        self.dont_recommend = shown;
        self
    }

    pub fn share(mut self, shown: bool) -> Self {
        self.share = shown;
        self
    }

    /// The dots, opening what the track's own menu offers.
    pub fn menu(mut self, target: Option<Target>) -> Self {
        self.menu = target;
        self
    }

    /// No track to act on: every mark is drawn and none answers.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_favorite(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_favorite = Some(Rc::new(press));
        self
    }

    pub fn on_dont_recommend(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dont_recommend = Some(Rc::new(press));
        self
    }

    pub fn on_share(mut self, press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_share = Some(Rc::new(press));
        self
    }
}

fn run(press: Option<Press>) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        if let Some(press) = &press {
            press(window, cx);
        }
    }
}

impl RenderOnce for TrackMarks {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let key = |name: &str| SharedString::from(format!("{id}-{name}"));
        let favorite = self.favorite.unwrap_or(false);
        let mark = |name: &str, glyph: &'static str| {
            IconButton::new(key(name), glyph)
                .size(self.size)
                .glyph(self.glyph)
                .tone(self.tone)
        };

        h_flex()
            .gap(self.gap)
            .items_center()
            .children(self.favorite.map(|_| {
                mark("favorite", icons::HEART)
                    .lit(favorite)
                    .accent(cx.theme().danger)
                    .tooltip(t(if favorite {
                        "remove_from_favorites"
                    } else {
                        "add_to_favorites"
                    }))
                    .disabled(self.disabled)
                    .on_click(run(self.on_favorite))
            }))
            .when(self.dont_recommend, |row| {
                // Like and dislike are one setting on the remote, so both at once would undo the heart.
                row.child(
                    mark("dont-recommend", icons::THUMBS_DOWN)
                        .tooltip(t("dont_recommend"))
                        .disabled(self.disabled || favorite)
                        .on_click(run(self.on_dont_recommend)),
                )
            })
            .when(self.share, |row| {
                // It copies whatever page the source itself publishes, which is rarely MusicBrainz.
                row.child(
                    mark("share", icons::SHARE)
                        .tooltip(t("share"))
                        .disabled(self.disabled)
                        .on_click(run(self.on_share)),
                )
            })
            .children(self.menu.map(|target| {
                MenuButton::new(key("menu"), menus::menu_for(target)).variant(MenuVariant::Icon {
                    size: self.size,
                    glyph: self.glyph,
                    tone: self.tone,
                })
            }))
    }
}
