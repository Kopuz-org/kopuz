//! A track's song and music video cuts: the switch between them, and the video's picture under the daemon's sound.

#[cfg(target_os = "macos")]
#[path = "video_mac.rs"]
mod mac;

use api::{PlayerCommand, TrackVersion};
use components::format::t;
use components::{Segment, Segmented, style};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div,
};
use link::Link;
use store::{Chrome, PlayerModel, SourcesModel};

use crate::NowPlaying;

const CUTS: [TrackVersion; 2] = [TrackVersion::Song, TrackVersion::Video];

/// Whether this build can draw a video's picture; without it the switch stays hidden, as the Dioxus Android build hides it.
const DRAWS_PICTURES: bool = cfg!(target_os = "macos");

/// The cut playing, when the source paired it with another and the picture can be shown.
pub(crate) fn current_version(cx: &App) -> Option<TrackVersion> {
    if !DRAWS_PICTURES
        || !SourcesModel::global(cx)
            .read(cx)
            .capabilities()
            .music_videos
    {
        return None;
    }
    PlayerModel::global(cx)
        .read(cx)
        .track()
        .and_then(api::TrackInfo::version)
}

/// Whether the shell draws the sheet at all: open, not shrunk to compact, and attached to a daemon.
fn sheet_shown(sheet_open: bool, compact: bool, connected: bool) -> bool {
    sheet_open && !compact && connected
}

/// The first decoded frame arrived, or the frames stopped.
pub(crate) struct Framed;

/// The muted picture of the music video playing, following the engine's playhead.
pub(crate) struct MusicVideo {
    key: String,
    framed: bool,
    #[cfg(target_os = "macos")]
    picture: Option<mac::Picture>,
    _player: Option<Subscription>,
}

impl EventEmitter<Framed> for MusicVideo {}

impl MusicVideo {
    fn new(key: String, cx: &mut Context<Self>) -> Self {
        #[cfg(target_os = "macos")]
        let picture = {
            let (link, key) = (Link::global(cx).clone(), key.clone());
            mac::Picture::open(Box::new(move |start, length| {
                let key = key.clone();
                futures::executor::block_on(link.call(move |api| async move {
                    api.video(api::VideoRequest {
                        key,
                        start,
                        length: Some(length),
                    })
                    .await
                }))
            }))
        };
        // A pause or a seek reaches the picture even on a frame GPUI does not draw it.
        let player = cx.observe(&PlayerModel::global(cx), |this, _, cx| this.follow(cx));
        Self {
            key,
            framed: false,
            #[cfg(target_os = "macos")]
            picture,
            _player: Some(player),
        }
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        if let Some(picture) = self.picture.as_mut() {
            let player = PlayerModel::global(cx).read(cx);
            let latency = player.output_latency_ms().unwrap_or_default();
            let position = player.position_ms().saturating_sub(latency) as f64 / 1000.;
            picture.follow(position, player.is_playing());
        }
        #[cfg(not(target_os = "macos"))]
        let _ = cx;
    }

    /// Whether a frame has been decoded, so the cover standing in for it can go.
    pub(crate) fn framed(&self) -> bool {
        self.framed
    }
}

impl Render for MusicVideo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let surface = div().size_full().rounded_xl().bg(style::black(1., cx));
        #[cfg(target_os = "macos")]
        if self.picture.is_some() {
            self.follow(cx);
            let frame = self.picture.as_mut().and_then(mac::Picture::frame);
            if frame.is_some() != self.framed {
                self.framed = frame.is_some();
                cx.emit(Framed);
            }
            // Frames arrive on the decoder's clock, not GPUI's, so it is asked for one every frame it is shown.
            window.request_animation_frame();
            return surface.children(frame.map(|frame| gpui_kit::surface(frame).size_full()));
        }
        let _ = window;
        surface
    }
}

impl NowPlaying {
    /// The video entity for what is playing, kept while the same cut plays and the sheet is drawn, dropped otherwise.
    pub(crate) fn sync_video(&mut self, cx: &mut Context<Self>) {
        let chrome = Chrome::global(cx).read(cx);
        let shown = self.presentation == crate::Presentation::Sheet
            && sheet_shown(
                chrome.is_sheet_open(),
                chrome.is_compact(),
                Link::connection(cx).read(cx).is_connected(),
            );
        let key = self
            .track(cx)
            .filter(|_| shown && current_version(cx) == Some(TrackVersion::Video))
            .map(|track| track.key);
        match key {
            Some(key)
                if self
                    .video
                    .as_ref()
                    .is_some_and(|(video, _)| video.read(cx).key == key) => {}
            Some(key) => {
                let video = cx.new(|cx| MusicVideo::new(key, cx));
                let watch = cx.subscribe(&video, |_, _, _: &Framed, cx| cx.notify());
                self.video = Some((video, watch));
            }
            None => self.video = None,
        }
    }

    pub(crate) fn video(&self) -> Option<Entity<MusicVideo>> {
        self.video.as_ref().map(|(video, _)| video.clone())
    }

    /// Song or video: pressed at once, while the other cut loads, until the daemon's swap settles it.
    pub(crate) fn version_switch(
        &self,
        version: TrackVersion,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let shown = self
            .requested_version
            .as_ref()
            .map_or(version, |(_, cut)| *cut);
        let owner = cx.entity().downgrade();
        Segmented::new("track-version")
            .segments([
                Segment::new("song").label(t("track_version_song")),
                Segment::new("video").label(t("track_version_video")),
            ])
            .selected(CUTS.iter().position(|cut| *cut == shown))
            .on_select(move |index, _, cx| {
                let Some(choice) = CUTS.get(*index).copied() else {
                    return;
                };
                owner
                    .update(cx, |this, cx| this.choose_version(choice, version, cx))
                    .ok();
            })
    }

    fn choose_version(
        &mut self,
        choice: TrackVersion,
        playing: TrackVersion,
        cx: &mut Context<Self>,
    ) {
        let Some(track) = self.track(cx) else { return };
        if self
            .requested_version
            .as_ref()
            .map_or(playing, |(_, cut)| *cut)
            == choice
        {
            return;
        }
        self.requested_version = Some((track.key, choice));
        let call = Link::global(cx).call(move |api| async move {
            api.player_command(PlayerCommand::SetVersion { version: choice })
                .await
        });
        cx.spawn(async move |this, cx| {
            let Err(error) = call.await else { return };
            this.update(cx, |this, cx| {
                this.requested_version = None;
                cx.notify();
            })
            .ok();
            cx.update(|cx| store::Notices::failed(&error, cx));
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::sheet_shown;

    #[test]
    fn the_picture_goes_with_the_sheet_into_compact_or_a_disconnect() {
        assert!(sheet_shown(true, false, true));
        assert!(!sheet_shown(true, true, true));
        assert!(!sheet_shown(true, false, false));
        assert!(!sheet_shown(false, false, true));
    }
}
