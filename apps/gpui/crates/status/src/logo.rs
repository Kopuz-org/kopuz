//! Kopuz's mark as white outlines, drawn on frame by frame: GPUI cannot tween a stroke's dash.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use components::logo;
use gpui_kit::{App, Image, ImageFormat};

/// Frames after the empty one; the last is the whole outline.
pub const FRAMES: usize = 20;
/// The draw-on, once.
pub const DRAW: Duration = Duration::from_millis(800);
/// One breath, out and back.
pub const BREATHE: Duration = Duration::from_millis(1600);
/// The faintest the breath fades to.
const DIMMEST: f32 = 0.35;

/// Every frame of the draw-on, built once.
pub fn frames() -> &'static [Arc<Image>] {
    static FRAMES_HELD: OnceLock<Vec<Arc<Image>>> = OnceLock::new();
    FRAMES_HELD.get_or_init(|| {
        (0..=FRAMES)
            .map(|frame| {
                let share = frame as f32 / FRAMES as f32;
                let svg = logo::svg(share, logo::THIN);
                Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()))
            })
            .collect()
    })
}

/// Drop the rasterised frames once nothing draws them.
pub fn release(cx: &mut App) {
    for frame in frames() {
        frame.clone().remove_asset(cx);
    }
}

/// Which frame and how opaque, `elapsed` into the loader; reduced motion is the still outline.
pub fn pose(elapsed: Duration, reduced: bool) -> (usize, f32) {
    if reduced {
        return (FRAMES, 1.);
    }
    if elapsed < DRAW {
        let share = elapsed.as_secs_f32() / DRAW.as_secs_f32();
        return ((share * FRAMES as f32).round() as usize, 1.);
    }
    let into = (elapsed - DRAW).as_secs_f32() / BREATHE.as_secs_f32();
    // A cosine starting at full: down to the dimmest and back each breath.
    let wave = (1. + (into * std::f32::consts::TAU).cos()) / 2.;
    (FRAMES, DIMMEST + (1. - DIMMEST) * wave)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outline_draws_on_then_breathes_from_full() {
        assert_eq!(pose(Duration::ZERO, false), (0, 1.));
        assert_eq!(pose(DRAW / 2, false).0, FRAMES / 2);
        assert_eq!(pose(DRAW, false), (FRAMES, 1.));
        let (_, faint) = pose(DRAW + BREATHE / 2, false);
        assert!((faint - DIMMEST).abs() < 0.01);
        let (_, back) = pose(DRAW + BREATHE, false);
        assert!((back - 1.).abs() < 0.01);
    }

    #[test]
    fn reduced_motion_is_the_still_outline() {
        assert_eq!(pose(Duration::from_millis(100), true), (FRAMES, 1.));
    }
}
