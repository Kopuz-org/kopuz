//! The "For you" row: the mix once it is in, pulsing tiles while it is asked for, and why it failed.

use std::rc::Rc;
use std::time::Duration;

use components::format::t;
use components::{Carousel, SectionHeading, icons, motion, skeleton, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, IntoElement, ParentElement as _, ScrollHandle,
    SharedString, Styled, Window, div, pulsating_between, px,
};
use store::{ForYouModel, ForYouState};

use api::ApiError;

use crate::cards::{GAP, TILE};
use crate::data::{Shelf, Tile};

/// Enough placeholders to fill a wide window; the daemon's mix is about this long anyway.
const PLACEHOLDERS: usize = 8;

/// Where the mix stands, as the row draws it.
#[derive(Clone, Default)]
pub(crate) struct Status {
    pub(crate) shown: bool,
    pub(crate) idle: bool,
    filled: bool,
    failed: Option<ApiError>,
}

impl Status {
    pub(crate) fn read(cx: &App) -> Self {
        let model = ForYouModel::global(cx).read(cx);
        Self {
            shown: model.is_shown(),
            idle: *model.state() == ForYouState::Idle,
            filled: model.shown_mix().is_some(),
            failed: match model.state() {
                ForYouState::Failed(error) => Some(error.clone()),
                _ => None,
            },
        }
    }
}

pub(crate) fn row(
    shelf: &Shelf,
    shared: &crate::Shared,
    last: &Rc<[Tile]>,
    handle: &ScrollHandle,
    fade: &mut motion::Fade,
    window: &mut Window,
    cx: &App,
) -> AnyElement {
    let title = t(crate::shelf::title(shelf.name));
    let (status, now) = (&shared.for_you, &shared.now);
    let strapline = shared.strapline.clone();
    let filled = status.filled;
    let shown = fade.sample(filled, motion::MEDIUM, window, cx);
    if filled {
        let content = crate::shelf::carousel(shelf, handle, now)
            .strapline(strapline.clone())
            .action(refresh(cx));
        // The pulsing row stays under the mix while it fades in, so the row never empties.
        let placeholder = (shown < 1.).then(|| placeholder(title.clone(), strapline, false, cx));
        return skeleton::reveal(content, placeholder, shown);
    }
    if shown > 0. && status.failed.is_none() && !last.is_empty() {
        // A refresh lets the mix go, so it fades out over the pulsing row rather than cutting to it.
        let leaving = Shelf {
            name: shelf.name,
            tiles: last.clone(),
            form: shelf.form,
        };
        let content = crate::shelf::carousel(&leaving, handle, now).strapline(strapline.clone());
        let placeholder = placeholder(title, strapline, true, cx);
        return skeleton::reveal(content, Some(placeholder), shown);
    }
    match &status.failed {
        Some(error) => v_flex()
            .w_full()
            .gap_4()
            .child(
                SectionHeading::new(title)
                    .strapline(strapline)
                    .trailing(refresh(cx)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(store::human(error)),
            )
            .into_any_element(),
        None => placeholder(title, strapline, true, cx),
    }
}

/// The pulsing row; the copy under a fading mix leaves the refresh button to the mix.
fn placeholder(
    title: SharedString,
    strapline: Option<SharedString>,
    with_refresh: bool,
    cx: &App,
) -> AnyElement {
    let row = Carousel::new(
        "for_you_loading",
        title,
        PLACEHOLDERS,
        TILE,
        |slot, _, cx| pulsing(slot, cx),
    )
    .gap(px(GAP))
    .strapline(strapline);
    match with_refresh {
        true => row.action(refresh(cx)).into_any_element(),
        false => row.into_any_element(),
    }
}

fn pulsing(slot: usize, cx: &App) -> AnyElement {
    div()
        .child(skeleton::shelf_tile(TILE, cx))
        .with_animation(
            SharedString::from(format!("for-you-pulse-{slot}")),
            Animation::new(Duration::from_millis(1100))
                .repeat()
                .with_easing(pulsating_between(0.4, 1.)),
            |tile, delta| tile.opacity(delta),
        )
        .into_any_element()
}

fn refresh(cx: &App) -> impl IntoElement {
    Button::new("for-you-refresh")
        .custom(style::quiet_button(
            style::white(style::TEXT_STRONG, cx),
            cx,
        ))
        .small()
        .rounded_full()
        .border_1()
        .border_color(style::white(style::BORDER, cx))
        .icon(icons::icon(icons::REFRESH))
        .label(t("refresh"))
        .on_click(|_, _, cx| {
            ForYouModel::global(cx).update(cx, |model, cx| model.refresh(cx));
        })
}
