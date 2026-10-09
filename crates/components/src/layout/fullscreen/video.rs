use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

/// The cut a song/video switch offers, when the source has music videos and
/// paired one with what is playing.
pub(crate) fn current_version(ctrl: &PlayerController) -> Option<api::TrackVersion> {
    let caps = consume_context::<Signal<api::SourceCapabilities>>();
    if !caps.read().music_videos {
        return None;
    }
    ctrl.current_track_snapshot
        .read()
        .as_ref()
        .and_then(api::TrackInfo::version)
}

#[component]
pub(crate) fn VersionSwitch(version: api::TrackVersion) -> Element {
    let ctrl = use_context::<PlayerController>();
    // Pressed at once, while the other cut loads; the daemon's swap settles it.
    let mut requested = use_signal(|| None::<api::TrackVersion>);
    use_effect(use_reactive!(|version| {
        let _ = version;
        requested.set(None);
    }));
    let shown = requested().unwrap_or(version);
    let segment = move |choice: api::TrackVersion| {
        if shown == choice {
            "px-4 py-1.5 text-xs font-medium rounded-md bg-white/20 text-white transition-colors"
        } else {
            "px-4 py-1.5 text-xs font-medium rounded-md text-white/50 hover:text-white/80 transition-colors"
        }
    };
    let mut choose = move |choice: api::TrackVersion| {
        if shown != choice {
            requested.set(Some(choice));
            ctrl.set_version(choice);
        }
    };

    rsx! {
        div {
            class: "app-segmented flex items-center gap-1 p-1 rounded-lg bg-white/10",
            role: "group",
            "aria-label": i18n::t("track_version").to_string(),
            button {
                class: segment(api::TrackVersion::Song),
                aria_pressed: shown == api::TrackVersion::Song,
                onclick: move |_| choose(api::TrackVersion::Song),
                "{i18n::t(\"track_version_song\")}"
            }
            button {
                class: segment(api::TrackVersion::Video),
                aria_pressed: shown == api::TrackVersion::Video,
                onclick: move |_| choose(api::TrackVersion::Video),
                "{i18n::t(\"track_version_video\")}"
            }
        }
    }
}

/// Keeps the element on the engine's clock by seeking, never by changing
/// its rate: WebKit stalls a video whose playbackRate keeps changing. A seek
/// lands as far behind as it took to finish, so the next one aims that much
/// ahead. It reports its drift every few seconds while playing.
const SYNC_SCRIPT: &str = r#"
const id = await dioxus.recv();
let video = null;
while (!(video = document.getElementById(id))) {
    await new Promise((resolve) => setTimeout(resolve, 50));
}
video.muted = true;
// WebKit builds no picture layer for a video that starts loading in a page
// that is hidden (the window covered or minimised), and decodes on into
// nothing once it is shown. Loading again once visible gives it one.
let unseen = document.hidden;
document.addEventListener("visibilitychange", () => {
    dioxus.send(document.visibilityState);
    if (document.hidden) {
        unseen = unseen || video.readyState < 2;
    } else if (unseen && video.isConnected) {
        unseen = false;
        video.load();
    }
});
let anchor = null;
let reported = 0;
let lead = 0;
let settling = false;
const tick = () => {
    if (!video.isConnected) return;
    if (anchor && video.readyState >= 1 && !video.seeking) {
        const now = performance.now();
        const target = anchor.playing ? anchor.position + (now - anchor.at) / 1000 : anchor.position;
        const drift = video.currentTime - target;
        if (anchor.playing && video.paused) video.play().catch(() => {});
        if (!anchor.playing && !video.paused) video.pause();
        if (settling && anchor.playing) {
            lead = Math.min(1, Math.max(0, lead - drift));
        }
        settling = false;
        if (anchor.playing && now - reported > 5000) {
            reported = now;
            dioxus.send(Math.round(drift * 1000));
        }
        if (Math.abs(drift) > (anchor.playing ? 0.15 : 0.05)) {
            video.currentTime = Math.max(0, target + (anchor.playing ? lead : 0));
            settling = Math.abs(drift) < 5;
        }
    }
    setTimeout(tick, 250);
};
tick();
while (true) {
    const [position, playing] = await dioxus.recv();
    anchor = { position, playing, at: performance.now() };
}
"#;

const VIDEO_ID: &str = "fullscreen-music-video";

/// The picture of the playing music video, muted: its sound is the engine's,
/// and the element only follows the engine's position.
#[component]
pub(crate) fn MusicVideo(track_key: String, poster: String) -> Element {
    let ctrl = use_context::<PlayerController>();
    let sync = use_hook(|| {
        let sync = document::eval(SYNC_SCRIPT);
        let _ = sync.send(VIDEO_ID);
        sync
    });
    use_effect(move || {
        if let Some((position, playing)) = ctrl.picture_anchor() {
            let _ = sync.send((position, playing));
        }
    });
    use_future(move || async move {
        let mut reports = sync;
        while let Ok(report) = reports.recv::<serde_json::Value>().await {
            match report.as_i64() {
                Some(drift_ms) => tracing::debug!(drift_ms, "music video sync"),
                None => tracing::debug!(%report, "music video visibility"),
            }
        }
    });
    let src = utils::format_video_url(&track_key);

    rsx! {
        // Without a stacking context of its own, WebKit composited the video
        // beneath the fullscreen backdrop and showed nothing.
        video {
            id: VIDEO_ID,
            class: "rounded-xl bg-black",
            style: "position: relative; z-index: 1; max-width: 100%; max-height: 100%; width: 100%; aspect-ratio: 16/9; object-fit: contain; box-shadow: 0 25px 60px -15px rgba(0,0,0,0.55);",
            src: "{src}",
            poster: "{poster}",
            muted: true,
            playsinline: true,
            preload: "auto",
            "disablepictureinpicture": "true",
            "aria-label": i18n::t("music_video").to_string(),
        }
    }
}
