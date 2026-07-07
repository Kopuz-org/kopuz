//! Smooth, clock-synced progress-bar animation.
//!
//! The playback position updates ~once a second (integer seconds), so animating
//! the bar with a CSS transition made it lag ~1s behind the real position and
//! kept drifting after pause. Instead a single `requestAnimationFrame` loop
//! interpolates from the engine's exact position against a real clock: the bar
//! is smooth while playing and freezes the instant `playing` goes false — no
//! CSS transition, no re-render per frame.
//!
//! The loop reads a global `__kopuzBar` state that Rust refreshes whenever the
//! position, play state or duration changes. It drives every `.kopuz-fill`
//! (width) and `.kopuz-thumb` (left) element, so all visible bars stay in sync.

use dioxus::{document::eval, prelude::*};
use hooks::use_player_controller::PlayerController;

const RAF_SETUP: &str = r#"
if (!window.__kopuzBarInit) {
    window.__kopuzBarInit = true;
    window.__kopuzBar = { sec: 0, dur: 0, playing: false, at: performance.now() };
    const tick = () => {
        const s = window.__kopuzBar;
        let cur = s.playing ? s.sec + (performance.now() - s.at) / 1000 : s.sec;
        if (s.dur > 0 && cur > s.dur) cur = s.dur;
        if (cur < 0) cur = 0;
        const w = (s.dur > 0 ? (cur / s.dur) * 100 : 0) + '%';
        const fills = document.getElementsByClassName('kopuz-fill');
        for (let i = 0; i < fills.length; i++) fills[i].style.width = w;
        const thumbs = document.getElementsByClassName('kopuz-thumb');
        for (let i = 0; i < thumbs.length; i++) thumbs[i].style.left = w;
        requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
}
"#;

/// Publish the current playback position/duration/state to the animation loop.
/// Call from a drag handler so the bar follows the drag (paused so it doesn't
/// interpolate forward); the sync effect resumes on drag end.
pub fn push_bar_state(secs: f64, duration_secs: f64, playing: bool) {
    let playing = if playing { "true" } else { "false" };
    let _ = eval(&format!(
        "if(window.__kopuzBar){{window.__kopuzBar={{sec:{secs},dur:{duration_secs},playing:{playing},at:performance.now()}};}}"
    ));
}

/// Drive the progress bars from the engine clock. Call once from any rendered
/// player bar (idempotent; the loop installs itself once).
pub fn use_progress_sync() {
    let ctrl = use_context::<PlayerController>();

    use_hook(|| {
        let _ = eval(RAF_SETUP);
    });

    use_effect(move || {
        let playing = *ctrl.is_playing.read();
        // Subscribe to the ~1Hz progress signal so this re-pushes as position
        // advances and immediately on seek/track-change; the deferred-crossfade
        // display is handled by `displayed_progress_secs_f64`.
        let _ = ctrl.current_song_progress.read();
        let secs = ctrl.displayed_progress_secs_f64();
        let duration = *ctrl.current_song_duration.read() as f64;
        push_bar_state(secs, duration, playing);
    });
}
