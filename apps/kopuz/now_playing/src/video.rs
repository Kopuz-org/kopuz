//! A track's song and music video cuts: the switch between them, and the video's picture under the daemon's sound.

#[cfg(target_os = "macos")]
#[path = "video_mac.rs"]
mod mac;

use api::{PlayerCommand, TrackVersion};
use components::format::t;
use components::{Segment, Segmented, style};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div,
};
use link::Link;
use store::{PlayerModel, SourcesModel};

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

/// The muted picture of the music video playing, following the engine's playhead.
pub(crate) struct MusicVideo {
    key: String,
    framed: bool,
    #[cfg(target_os = "macos")]
    picture: Option<mac::Picture>,
}

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
        #[cfg(not(target_os = "macos"))]
        let _ = cx;
        Self {
            key,
            framed: false,
            #[cfg(target_os = "macos")]
            picture,
        }
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
        if let Some(picture) = self.picture.as_mut() {
            let player = PlayerModel::global(cx).read(cx);
            let latency = player.output_latency_ms().unwrap_or_default();
            let position = player.position_ms().saturating_sub(latency) as f64 / 1000.;
            picture.follow(position, player.is_playing());
            let frame = picture.frame();
            if frame.is_some() != self.framed {
                self.framed = frame.is_some();
                cx.notify();
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
    /// The video entity for what is playing, kept while the same cut plays and dropped once it is not a video.
    pub(crate) fn sync_video(&mut self, open: bool, cx: &mut Context<Self>) {
        let key = self
            .track(cx)
            .filter(|_| open && current_version(cx) == Some(TrackVersion::Video))
            .map(|track| track.key);
        match key {
            Some(key)
                if self
                    .video
                    .as_ref()
                    .is_some_and(|(video, _)| video.read(cx).key == key) => {}
            Some(key) => {
                let video = cx.new(|cx| MusicVideo::new(key, cx));
                let watch: Subscription = cx.observe(&video, |_, _, cx| cx.notify());
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
