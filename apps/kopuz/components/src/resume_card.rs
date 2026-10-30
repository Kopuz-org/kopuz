//! The card a page opens with to pick the last queue back up: a cover, the track, and one button.

use std::rc::Rc;

use gpui_kit::component::h_flex;
use gpui_kit::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, px,
};
use store::{Navigator, Pictured, Route, Target};

use crate::format::{t, t_with};
use crate::menus::{Picture, WithContextMenu as _};
use crate::track_meta::{MetaLayout, NowPlayingMeta};
use crate::transport::PlayPauseButton;
use crate::{Cover, menus, motion, style};

const HOVER: u64 = 0;
const WASH: u64 = 1;
const COVER: u64 = 2;

/// The cover on the stage, against the 88px it used to be.
const STAGE_COVER: f32 = 132.;

type Press = Rc<dyn Fn(&mut Window, &mut App)>;

/// The queue the last session was interrupted in, and the one button that picks it up.
#[derive(IntoElement)]
pub struct ResumeCard {
    id: SharedString,
    track: api::TrackInfo,
    left: usize,
    playing: bool,
    toggle: Option<Press>,
}

impl ResumeCard {
    pub fn new(id: impl Into<SharedString>, track: &api::TrackInfo) -> Self {
        Self {
            id: id.into(),
            track: track.clone(),
            left: 0,
            playing: false,
            toggle: None,
        }
    }

    /// How many tracks are still queued after this one.
    pub fn left(mut self, left: usize) -> Self {
        self.left = left;
        self
    }

    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    pub fn on_toggle(mut self, toggle: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.toggle = Some(Rc::new(toggle));
        self
    }
}

impl RenderOnce for ResumeCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = self.id;
        let track = self.track;
        let route = match track.album_id.is_empty() {
            false => Some(Route::Album(track.album_id.clone().into())),
            true => Route::artist_of(&track),
        };
        let target = Target::track(track.key.clone(), track.title.clone());
        let (over, pointer) = motion::hovered(motion::channel(&key, HOVER), window, cx);
        // The window already carries the cover's colour behind this, so the stage is glass, not a second blur.
        let wash = motion::color(
            motion::channel(&key, WASH),
            style::white(
                match over {
                    true => style::HOVER,
                    false => style::RAISED,
                },
                cx,
            ),
            window,
            cx,
        );
        let toggle = self.toggle;
        let picture = Picture::new(
            track.artwork.as_ref(),
            Pictured::Track {
                artist: track.artist.clone().into(),
                title: track.title.clone().into(),
            },
        );

        h_flex()
            .id(key.clone())
            .relative()
            .w_full()
            // `overflow_hidden` zeroes the automatic minimum height, so the scroller would crush it.
            .flex_none()
            .gap_6()
            .p_6()
            .items_center()
            .overflow_hidden()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .bg(wash)
            .cursor_pointer()
            .on_hover(motion::track(pointer))
            .on_click(move |_, _, cx| {
                if let Some(route) = route.clone() {
                    Navigator::navigate(route, cx)
                }
            })
            .child(
                Cover::new(track.artwork.as_ref())
                    .size(px(STAGE_COVER))
                    .rounded(style::to_px(style::radius_lg(), window))
                    .zoom(motion::channel(&key, COVER), over)
                    .spot(picture.as_ref().map(Picture::spot)),
            )
            .child(
                NowPlayingMeta::new(SharedString::from(format!("{key}-meta")), &track)
                    .layout(MetaLayout::Card)
                    .eyebrow(t("home_continue"))
                    .note(
                        (self.left > 0)
                            .then(|| t_with("home_queue_left", &[("count", self.left.into())])),
                    ),
            )
            .child(
                PlayPauseButton::new(SharedString::from(format!("{key}-play")))
                    .pill(true)
                    .playing(self.playing)
                    .stop_propagation(true)
                    .on_click(move |_, window, cx| {
                        if let Some(toggle) = &toggle {
                            toggle(window, cx);
                        }
                    }),
            )
            .with_context_menu(menus::pictured(Some(menus::menu_for(target)), picture))
    }
}
