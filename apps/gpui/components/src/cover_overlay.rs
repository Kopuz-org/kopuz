//! What rides on a cover: the hover wash, the playing badge, the play button and a corner control.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::transport::PlayPauseButton;
use crate::{icons, motion};

const CORNER: u64 = 10;

/// What a cover's play button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayState {
    #[default]
    Idle,
    /// What it names is the thing playing, so a press pauses.
    Playing,
    /// The press is on its way; the queue is not built yet.
    Loading,
}

/// `bg-primary` circle on a playing cover's corner.
pub fn playing_badge(cx: &App) -> AnyElement {
    div()
        .absolute()
        .bottom_2()
        .left_2()
        .size_6()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(cx.theme().primary)
        .child(
            icons::icon(icons::VOLUME)
                .size(px(12.))
                .text_color(cx.theme().primary_foreground),
        )
        .into_any_element()
}

/// A cover with the things that sit over it; the owner decides which are showing.
#[derive(IntoElement)]
pub struct CoverOverlay {
    id: SharedString,
    cover: AnyElement,
    radius: Pixels,
    wash: Option<Hsla>,
    playing: bool,
    play: Option<PlayPauseButton>,
    play_shown: f32,
    corner: Option<AnyElement>,
}

impl CoverOverlay {
    pub fn new(id: impl Into<SharedString>, cover: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            cover: cover.into_any_element(),
            radius: px(0.),
            wash: None,
            playing: false,
            play: None,
            play_shown: 1.,
            corner: None,
        }
    }

    /// The cover's corner radius, which the wash follows.
    pub fn radius(mut self, radius: Pixels) -> Self {
        self.radius = radius;
        self
    }

    /// A tint over the whole cover, such as the dimming under the pointer.
    pub fn wash(mut self, wash: Option<Hsla>) -> Self {
        self.wash = wash;
        self
    }

    /// The badge that says what the cover names is playing.
    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    pub fn play(mut self, play: Option<PlayPauseButton>) -> Self {
        self.play = play;
        self
    }

    /// How far the play button has faded in.
    pub fn play_shown(mut self, shown: f32) -> Self {
        self.play_shown = shown;
        self
    }

    /// A control in the top corner, whose press is never a click on the card under it.
    pub fn corner(mut self, corner: Option<AnyElement>) -> Self {
        self.corner = corner;
        self
    }
}

impl RenderOnce for CoverOverlay {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let radius = self.radius;
        div()
            .relative()
            .child(self.cover)
            .when_some(self.wash, |cover, wash| {
                cover.child(div().absolute().inset_0().rounded(radius).bg(wash))
            })
            .when(self.playing, |cover| cover.child(playing_badge(cx)))
            .when_some(self.play, |cover, play| {
                cover.child(
                    div()
                        .absolute()
                        .right_2()
                        .bottom_2()
                        .opacity(self.play_shown)
                        .child(play),
                )
            })
            .when_some(self.corner, |cover, corner| {
                cover.child(
                    div()
                        .id(motion::channel(&id, CORNER))
                        .absolute()
                        .top_2()
                        .right_2()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(corner),
                )
            })
    }
}
