use std::rc::Rc;

use crate::cover::{Cover, EditableCover, Scrim};
use crate::menus::{self, Menu, Picture, WithContextMenu as _};
use crate::{icons, style};
use api::ArtworkRef;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, RenderOnce, SharedString, Styled, Window, div, px, rems,
};

/// How big a page's title is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TitleSize {
    /// `text-3xl font-semibold`, every page but one.
    #[default]
    Page,
    /// `text-4xl font-black`, the home page's greeting.
    Display,
}

/// The title block every page opens with, with its actions on the right.
#[derive(IntoElement)]
pub struct PageHeader {
    title: SharedString,
    subtitle: Option<SharedString>,
    count: Option<SharedString>,
    icon: Option<&'static str>,
    size: TitleSize,
    flush: bool,
    actions: Vec<AnyElement>,
}

impl PageHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            count: None,
            icon: None,
            size: TitleSize::Page,
            flush: false,
            actions: Vec::new(),
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// A count read on the title's baseline, such as how many tracks a library holds.
    pub fn count(mut self, count: impl Into<SharedString>) -> Self {
        self.count = Some(count.into());
        self
    }

    /// A glyph in the accent before the title.
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn size(mut self, size: TitleSize) -> Self {
        self.size = size;
        self
    }

    /// No space of its own under it, for a page that spaces its column itself.
    pub fn flush(mut self) -> Self {
        self.flush = true;
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for PageHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let quiet =
            |text: SharedString| div().text_sm().text_color(style::slate_400(cx)).child(text);
        let title = div()
            .text_color(cx.theme().foreground)
            .map(|title| match self.size {
                TitleSize::Page => title.text_3xl().font_weight(FontWeight::SEMIBOLD),
                TitleSize::Display => title.text_size(rems(2.25)).font_weight(FontWeight::BLACK),
            })
            .child(self.title);
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .flex_wrap()
            .gap_3()
            .when(!self.flush, |header| header.pb_6())
            .child(
                v_flex()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .when_some(self.icon, |line, icon| {
                                line.child(
                                    icons::icon(icon)
                                        .size(px(24.))
                                        .text_color(cx.theme().primary_hover),
                                )
                            })
                            .child(
                                h_flex()
                                    .items_baseline()
                                    .gap_3()
                                    .child(title)
                                    .children(self.count.map(quiet)),
                            ),
                    )
                    .children(self.subtitle.map(quiet)),
            )
            .child(h_flex().gap_2().items_center().children(self.actions))
    }
}

/// `w-64 h-64`: the cover a detail page opens with.
pub const HERO: Pixels = px(256.);

/// What a page does when its cover is clicked; only it knows what a new picture means.
type CoverAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// Facts joined with a dot, such as an album's song count, length and year.
#[derive(IntoElement)]
pub struct MetaLine {
    facts: Vec<AnyElement>,
    kind: Option<SharedString>,
    compact: bool,
    color: Option<Hsla>,
}

impl MetaLine {
    pub fn new(facts: impl IntoIterator<Item = impl IntoElement>) -> Self {
        Self {
            facts: facts
                .into_iter()
                .map(IntoElement::into_any_element)
                .collect(),
            kind: None,
            compact: false,
            color: None,
        }
    }

    /// What it is, set before the first fact in a smaller bold type.
    pub fn kind(mut self, kind: impl Into<SharedString>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// Small type, tight gaps and faint dots, for a column beside a list.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// The ink of the facts, `text-slate-400` unless set.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for MetaLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let compact = self.compact;
        let faint = style::white(0.3, cx);
        let dot = move || match compact {
            true => div().text_color(faint).child("•"),
            false => div().child("•"),
        };
        let kind = self.kind.map(|kind| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(style::white(0.4, cx))
                .child(kind)
        });
        let leading = kind.is_some();
        h_flex()
            .items_center()
            .flex_wrap()
            .map(|line| match compact {
                true => line.gap_x_2().text_sm(),
                false => line.gap_6(),
            })
            .text_color(self.color.unwrap_or_else(|| style::slate_400(cx)))
            .children(kind)
            .children(
                self.facts
                    .into_iter()
                    .enumerate()
                    .flat_map(move |(at, fact)| {
                        let dotted = at > 0 || leading;
                        dotted
                            .then(|| dot().into_any_element())
                            .into_iter()
                            .chain(std::iter::once(fact))
                    }),
            )
    }
}

/// How a detail header lays its cover and words out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HeaderLayout {
    /// The cover, then the words, then the actions, along one row.
    #[default]
    Row,
    /// A narrow column for a page that puts its list beside it.
    Column,
    /// The artwork behind the words, edge to edge.
    Banner,
}

/// `w-80`: the column layout's width, a cover and a little air.
const COLUMN: Pixels = px(320.);

/// The block every detail page opens with: the cover, what it is, and what you can do to it.
#[derive(IntoElement)]
pub struct DetailHeader {
    title: SharedString,
    artwork: Option<ArtworkRef>,
    placeholder: &'static str,
    round: bool,
    size: Pixels,
    layout: HeaderLayout,
    eyebrow: Option<AnyElement>,
    kind: Option<SharedString>,
    meta: Vec<SharedString>,
    description: Option<SharedString>,
    actions: Vec<AnyElement>,
    cover_id: Option<SharedString>,
    cover_tooltip: Option<SharedString>,
    on_cover: Option<CoverAction>,
    // A page whose cover carries its own overlay hands the whole thing over instead.
    custom_cover: Option<AnyElement>,
    cover_menu: Option<Menu>,
    picture: Option<Picture>,
}

impl DetailHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            artwork: None,
            placeholder: crate::icons::DISC,
            round: false,
            size: HERO,
            layout: HeaderLayout::Row,
            eyebrow: None,
            kind: None,
            meta: Vec::new(),
            description: None,
            actions: Vec::new(),
            cover_id: None,
            cover_tooltip: None,
            on_cover: None,
            custom_cover: None,
            cover_menu: None,
            picture: None,
        }
    }

    pub fn layout(mut self, layout: HeaderLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Draw this instead of the plain cover, for a page whose cover is also an upload target.
    pub fn cover(mut self, cover: impl IntoElement) -> Self {
        self.custom_cover = Some(cover.into_any_element());
        self
    }

    /// Right-click on the cover opens `menu`, ending in the picture's copy and save when there is one.
    pub fn cover_menu(mut self, menu: Menu, picture: Option<Picture>) -> Self {
        self.cover_menu = Some(menu);
        self.picture = picture;
        self
    }

    pub fn artwork(mut self, artwork: Option<&ArtworkRef>) -> Self {
        self.artwork = artwork.cloned();
        self
    }

    pub fn placeholder(mut self, placeholder: &'static str) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// An artist is a person, so their picture is a circle.
    pub fn round(mut self, round: bool) -> Self {
        self.round = round;
        self
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// The line above the title: an artist link, a kind, a source.
    pub fn eyebrow(mut self, eyebrow: impl IntoElement) -> Self {
        self.eyebrow = Some(eyebrow.into_any_element());
        self
    }

    /// What it is, set before the facts under the title.
    pub fn kind(mut self, kind: impl Into<SharedString>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// One more fact for the line under the title; they are joined with a dot.
    pub fn meta(mut self, fact: impl Into<SharedString>) -> Self {
        self.meta.push(fact.into());
        self
    }

    /// A few lines about it, under the facts.
    pub fn description(mut self, description: Option<impl Into<SharedString>>) -> Self {
        self.description = description.map(Into::into);
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// Make the cover a target for a new picture, named by `tooltip`; the page decides what that means.
    pub fn on_cover(
        mut self,
        id: impl Into<SharedString>,
        tooltip: Option<SharedString>,
        act: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.cover_id = Some(id.into());
        self.cover_tooltip = tooltip;
        self.on_cover = Some(Rc::new(act));
        self
    }

    fn cover_element(&mut self) -> AnyElement {
        let radius = match self.round {
            true => self.size / 2.,
            false => px(12.),
        };
        let spot = self.picture.as_ref().map(Picture::spot);
        let cover = match (
            self.custom_cover.take(),
            self.cover_id.take(),
            self.on_cover.take(),
        ) {
            (Some(custom), _, _) => custom,
            (None, Some(id), Some(act)) => EditableCover::new(id, self.artwork.as_ref())
                .size(self.size)
                .rounded(radius)
                .placeholder(self.placeholder)
                .spot(spot)
                .when_some(self.cover_tooltip.take(), |cover, tip| cover.tooltip(tip))
                .on_pick(move |window, cx| act(window, cx))
                .into_any_element(),
            (None, _, _) => Cover::new(self.artwork.as_ref())
                .size(self.size)
                .hq(true)
                .rounded(radius)
                .placeholder(self.placeholder)
                .spot(spot)
                .into_any_element(),
        };
        match menus::pictured(self.cover_menu.take(), self.picture.take()) {
            Some(menu) => div()
                .id("detail-cover")
                .child(cover)
                .with_context_menu(Some(menu)),
            None => cover,
        }
    }

    fn facts(&mut self, compact: bool, color: Option<Hsla>) -> Option<MetaLine> {
        if self.meta.is_empty() && self.kind.is_none() {
            return None;
        }
        let line = MetaLine::new(std::mem::take(&mut self.meta))
            .compact(compact)
            .when_some(color, |line, color| line.color(color));
        Some(match self.kind.take() {
            Some(kind) => line.kind(kind),
            None => line,
        })
    }

    fn row(mut self, cx: &App) -> AnyElement {
        let cover = self.cover_element();
        let facts = self.facts(false, None);
        h_flex()
            .w_full()
            .gap_8()
            .items_end()
            .child(div().flex_shrink_0().child(cover))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .children(self.eyebrow.map(|eyebrow| div().mb_2().child(eyebrow)))
                    .child(
                        div()
                            .mb_6()
                            .line_clamp(3)
                            .text_size(rems(4.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title),
                    )
                    .children(facts)
                    .children(self.description.map(|text| description(text, cx))),
            )
            // Upstream the actions are the row's third column, at its bottom-right, not under the facts.
            .when(!self.actions.is_empty(), |row| {
                row.child(
                    h_flex()
                        .flex_shrink_0()
                        .gap_4()
                        .items_center()
                        .children(self.actions),
                )
            })
            .into_any_element()
    }

    fn column(mut self, cx: &App) -> AnyElement {
        let cover = self.cover_element();
        let facts = self.facts(true, None);
        v_flex()
            .w(COLUMN)
            .flex_shrink_0()
            .items_start()
            .gap_5()
            .pt_2()
            .child(div().shadow_2xl().child(cover))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .children(self.eyebrow)
                    .child(
                        div()
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title),
                    )
                    .children(facts)
                    .children(self.description.map(|text| description(text, cx))),
            )
            .when(!self.actions.is_empty(), |column| {
                column.child(
                    h_flex()
                        .mt_1()
                        .gap_3()
                        .items_center()
                        .flex_wrap()
                        .children(self.actions),
                )
            })
            .into_any_element()
    }

    fn banner(mut self, cx: &App) -> AnyElement {
        let facts = self.facts(true, Some(style::white(0.7, cx)));
        div()
            .relative()
            .w_full()
            .h(rems(22.5))
            .overflow_hidden()
            .bg(cx.theme().secondary)
            .child(
                Cover::new(self.artwork.as_ref())
                    .fill()
                    .hq(true)
                    .scrim(Scrim {
                        foot: 0.95,
                        top: 0.2,
                        reach: 1.,
                    }),
            )
            .child(
                v_flex()
                    .absolute()
                    .size_full()
                    .justify_end()
                    .gap_4()
                    .px_10()
                    .pt_16()
                    .pb_10()
                    .children(self.eyebrow)
                    .child(
                        div()
                            .text_size(rems(3.75))
                            .font_weight(FontWeight::BLACK)
                            .child(self.title),
                    )
                    .children(facts)
                    .children(self.description.map(|text| description(text, cx)))
                    .when(!self.actions.is_empty(), |banner| {
                        banner.child(
                            h_flex()
                                .mt_2()
                                .gap_3()
                                .items_center()
                                .children(self.actions),
                        )
                    }),
            )
            .into_any_element()
    }
}

fn description(text: SharedString, cx: &App) -> impl IntoElement {
    div()
        .max_w(rems(48.))
        .text_sm()
        .line_clamp(3)
        .text_color(style::white(style::TEXT_SECONDARY, cx))
        .child(text)
}

impl RenderOnce for DetailHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        match self.layout {
            HeaderLayout::Row => self.row(cx),
            HeaderLayout::Column => self.column(cx),
            HeaderLayout::Banner => self.banner(cx),
        }
    }
}

/// The same shapes in the theme's placeholder wash, for a detail page that has not been answered.
pub fn detail_header_skeleton(size: Pixels, round: bool, cx: &App) -> impl IntoElement {
    let ink = cx.theme().skeleton;
    let bar = move |width: Pixels, height: Pixels| {
        div().w(width).h(height).rounded(style::radius(cx)).bg(ink)
    };
    h_flex()
        .w_full()
        .gap_8()
        .items_end()
        .child(
            div()
                .size(size)
                .flex_shrink_0()
                .bg(ink)
                .map(|block| match round {
                    true => block.rounded(size / 2.),
                    false => block.rounded(px(12.)),
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_4()
                .child(bar(px(120.), px(12.)))
                .child(bar(px(380.), px(48.)))
                .child(bar(px(220.), px(12.))),
        )
}
