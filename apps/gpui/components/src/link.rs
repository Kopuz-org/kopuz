//! The one text link: a name that opens what it names, brightening or underlining under the pointer.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, Entity, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, ParentElement, RenderOnce, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Window, div,
};
use store::{Navigator, Route};

use crate::{motion, style};

/// What colour a link rests at, and what it lifts to under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tone {
    /// Keeps the colour around it; the underline is its only cue.
    #[default]
    Inherit,
    /// `text-slate-500 hover:text-slate-400`: a track row's artist and album.
    Dim,
    /// `text-slate-400 hover:text-white/70`: the bar's artist line.
    Muted,
    /// `text-white/50 hover:text-white/80`: a line under a title.
    Faint,
    /// `text-white/60 hover:text-white`: a heading's "show all", an album's artist.
    Quiet,
}

impl Tone {
    fn inks(self, cx: &App) -> Option<(Hsla, Hsla)> {
        match self {
            Tone::Inherit => None,
            Tone::Dim => Some((style::slate_500(cx), style::slate_400(cx))),
            Tone::Muted => Some((style::slate_400(cx), style::white(0.7, cx))),
            Tone::Faint => Some((style::white(style::TEXT_MUTED, cx), style::white(0.8, cx))),
            Tone::Quiet => Some((
                style::white(style::TEXT_SECONDARY, cx),
                cx.theme().foreground,
            )),
        }
    }
}

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type BeforeNavigate = Rc<dyn Fn(&mut Window, &mut App)>;

/// Text that opens a route or runs a click; with neither it is drawn as plain text in its tone.
#[derive(IntoElement)]
pub struct Link {
    id: ElementId,
    label: Option<SharedString>,
    children: Vec<AnyElement>,
    route: Option<Route>,
    click: Option<OnClick>,
    before: Option<BeforeNavigate>,
    tone: Tone,
    brighten: bool,
    underline: bool,
    stop: bool,
    tooltip: Option<SharedString>,
    hover: Option<Entity<bool>>,
    style: StyleRefinement,
}

impl Link {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            children: Vec::new(),
            route: None,
            click: None,
            before: None,
            tone: Tone::Inherit,
            brighten: true,
            underline: false,
            stop: false,
            tooltip: None,
            hover: None,
            style: StyleRefinement::default(),
        }
    }

    /// The text, truncated with an ellipsis when it does not fit.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Where a click goes; none leaves the text inert, as for a name nothing could key.
    pub fn route(mut self, route: Option<Route>) -> Self {
        self.route = route;
        self
    }

    /// What a click does, when it is more than a navigation.
    pub fn on_click(
        mut self,
        click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some(Rc::new(click));
        self
    }

    /// Run before the link acts, so a sheet can close before the page under it changes.
    pub fn on_before_navigate(mut self, before: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.before = Some(Rc::new(before));
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Whether the pointer lifts the tone's colour; off leaves only the underline.
    pub fn brighten(mut self, brighten: bool) -> Self {
        self.brighten = brighten;
        self
    }

    pub fn underline(mut self, underline: bool) -> Self {
        self.underline = underline;
        self
    }

    /// Keep the click from reaching what the link sits in, such as a row that would play.
    pub fn stop_propagation(mut self, stop: bool) -> Self {
        self.stop = stop;
        self
    }

    pub fn tooltip(mut self, tooltip: Option<impl Into<SharedString>>) -> Self {
        self.tooltip = tooltip.map(Into::into);
        self
    }

    /// Report hover into a flag the caller reads, for a link whose content reacts to the pointer.
    pub fn hover_state(mut self, state: Entity<bool>) -> Self {
        self.hover = Some(state);
        self
    }
}

impl Styled for Link {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Link {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Link {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = Arc::new(self.id.clone());
        let pointer = match self.hover {
            Some(state) => state,
            None => {
                motion::hovered(
                    ElementId::NamedChild(key.clone(), "hover".into()),
                    window,
                    cx,
                )
                .1
            }
        };
        let over = *pointer.read(cx);
        let live = self.route.is_some() || self.click.is_some();
        let lit = live && over;
        let ink = self.tone.inks(cx).map(|(rest, lifted)| {
            motion::color(
                ElementId::NamedChild(key, "ink".into()),
                match lit && self.brighten {
                    true => lifted,
                    false => rest,
                },
                window,
                cx,
            )
        });
        let act = match (self.route, self.click) {
            (_, Some(click)) => Some(click),
            (Some(route), None) => Some(Rc::new(
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    Navigator::navigate(route.clone(), cx)
                },
            ) as OnClick),
            (None, None) => None,
        };
        let before = self.before;
        let stop = self.stop;
        div()
            .id(self.id)
            .refine_style(&self.style)
            .when_some(ink, |link, ink| link.text_color(ink))
            .when(lit && self.underline, |link| link.underline())
            .on_hover(motion::track(pointer))
            .when_some(act, |link, act| {
                link.cursor_pointer().on_click(move |event, window, cx| {
                    if stop {
                        cx.stop_propagation();
                    }
                    if let Some(before) = &before {
                        before(window, cx);
                    }
                    act(event, window, cx);
                })
            })
            .when_some(self.tooltip, |link, tip| {
                link.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .when_some(self.label, |link, label| link.truncate().child(label))
            .children(self.children)
    }
}

/// The quiet link at the end of a heading, such as "show all".
pub fn heading_link(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Link {
    Link::new(id)
        .label(label)
        .tone(Tone::Quiet)
        .flex_shrink_0()
        .text_xs()
        .font_weight(FontWeight::BOLD)
}
