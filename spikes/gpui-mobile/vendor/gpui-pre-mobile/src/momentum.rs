//! Momentum scrolling (inertia / fling) for touch-based platforms.
//!
//! When a user drags their finger across the screen and lifts it, native
//! platforms continue scrolling with a decelerating velocity — this is
//! "momentum scrolling" or "fling". Without it, scrolling feels sluggish
//! and stops dead the moment the finger lifts.
//!
//! This module provides two components:
//!
//! 1. **`VelocityTracker`** — records recent touch positions and computes
//!    the release velocity when the finger lifts.
//!
//! 2. **`MomentumScroller`** — takes the release velocity and produces
//!    a stream of decelerating scroll deltas that should be emitted as
//!    `ScrollWheel` events on each frame tick.
//!
//! # Usage (platform integration)
//!
//! ```ignore
//! let mut tracker = VelocityTracker::new();
//! let mut scroller = MomentumScroller::new();
//!
//! // On each touch move:
//! tracker.record(logical_x, logical_y);
//!
//! // On finger lift:
//! let (vx, vy) = tracker.velocity();
//! scroller.fling(vx, vy, last_position);
//! tracker.reset();
//!
//! // On each frame tick (CADisplayLink / Choreographer):
//! if let Some(delta) = scroller.step() {
//!     emit(ScrollWheelEvent { delta, phase: Moved, .. });
//! }
//! // When step() returns None the fling is finished.
//! ```

use std::time::Instant;

// ── Configuration ────────────────────────────────────────────────────────────

/// Deceleration rate per millisecond.  iOS `UIScrollView` uses
/// `decelerationRate = .normal` which is 0.998 per ms.  This gives a
/// moderate fling (~2000 px/s) about 2–3 seconds of travel before
/// stopping — matching the native feel users expect.
///
/// At 60fps (dt=16.6ms): decay = 0.998^16.6 = 0.967 → 3.3% loss/frame.
/// A 2000px/s fling runs for ~130 frames (~2.2 seconds).
const DECELERATION_RATE: f32 = 0.998;

/// Below this velocity (px/s) we consider the fling finished.
const MIN_VELOCITY: f32 = 30.0;

/// Maximum velocity (px/s) we allow.  A fast swipe on modern phones
/// can easily reach 12000+ px/s.
const MAX_VELOCITY: f32 = 16000.0;

/// Maximum number of recent touch samples kept for velocity estimation.
const MAX_SAMPLES: usize = 20;

/// Only samples within the most recent N milliseconds are used for
/// velocity estimation.  A shorter window (100ms) captures just the
/// fast release portion of the gesture, preventing a slow start from
/// diluting the fling velocity.
const VELOCITY_WINDOW_SECS: f64 = 0.10; // 100 ms

/// Minimum number of samples needed to compute a meaningful velocity.
const MIN_SAMPLES_FOR_VELOCITY: usize = 2;

// ── VelocityTracker ──────────────────────────────────────────────────────────

/// A sample recorded during a touch drag.
#[derive(Clone, Copy, Debug)]
struct Sample {
    x: f32,
    y: f32,
    time: Instant,
}

/// Tracks recent touch positions and computes the release velocity.
///
/// Call [`record`](Self::record) on every `ACTION_MOVE` / `UITouchPhase::Moved`
/// event, then call [`velocity`](Self::velocity) when the finger lifts to get
/// the fling velocity in logical pixels per second.
pub struct VelocityTracker {
    /// Ring buffer of recent samples.
    samples: [Option<Sample>; MAX_SAMPLES],
    /// Next write index into `samples`.
    index: usize,
    /// Total number of samples recorded (may exceed MAX_SAMPLES).
    count: usize,
}

impl Default for VelocityTracker {
    fn default() -> Self {
        Self {
            samples: [None; MAX_SAMPLES],
            index: 0,
            count: 0,
        }
    }
}

impl VelocityTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a touch position.  Call on every move event.
    pub fn record(&mut self, x: f32, y: f32) {
        self.samples[self.index] = Some(Sample {
            x,
            y,
            time: Instant::now(),
        });
        self.index = (self.index + 1) % MAX_SAMPLES;
        self.count += 1;
    }

    /// Compute the release velocity in logical px/s.
    ///
    /// Uses a weighted least-squares fit over recent samples within
    /// [`VELOCITY_WINDOW_SECS`] to produce a smooth estimate that
    /// emphasizes the most recent (fastest) portion of the gesture.
    /// Returns `(vx, vy)`.
    pub fn velocity(&self) -> (f32, f32) {
        let now = Instant::now();

        // Collect recent samples within the velocity window.
        let mut recent: Vec<&Sample> = Vec::with_capacity(MAX_SAMPLES);
        for s in self.samples.iter().flatten() {
            let age = now.duration_since(s.time).as_secs_f64();
            if age <= VELOCITY_WINDOW_SECS {
                recent.push(s);
            }
        }

        if recent.len() < MIN_SAMPLES_FOR_VELOCITY {
            return (0.0, 0.0);
        }

        // Sort by time ascending.
        recent.sort_by(|a, b| a.time.cmp(&b.time));

        // Use weighted regression for 3+ samples, simple difference for 2.
        if recent.len() >= 3 {
            let (wvx, wvy) = weighted_velocity(&recent);
            return (clamp_velocity(wvx), clamp_velocity(wvy));
        }

        let first = recent[0];
        let last = recent[recent.len() - 1];
        let dt = last.time.duration_since(first.time).as_secs_f64();

        if dt < 1e-6 {
            return (0.0, 0.0);
        }

        let vx = ((last.x - first.x) as f64 / dt) as f32;
        let vy = ((last.y - first.y) as f64 / dt) as f32;

        (clamp_velocity(vx), clamp_velocity(vy))
    }

    /// Reset the tracker for a new gesture.
    pub fn reset(&mut self) {
        self.samples = [None; MAX_SAMPLES];
        self.index = 0;
        self.count = 0;
    }
}

/// Weighted least-squares velocity: exponentially increasing weight toward
/// recent samples so the release velocity reflects the *end* of the gesture,
/// not the slow start.
fn weighted_velocity(samples: &[&Sample]) -> (f32, f32) {
    if samples.len() < 2 {
        return (0.0, 0.0);
    }

    let t0 = samples[0].time;
    let n = samples.len();

    // Exponential weight: w = e^(2 * i / n).  The most recent sample gets
    // ~7x the weight of the oldest, strongly biasing toward the fast release.
    let mut sum_w = 0.0_f64;
    let mut sum_wt = 0.0_f64;
    let mut sum_wt2 = 0.0_f64;
    let mut sum_wx = 0.0_f64;
    let mut sum_wy = 0.0_f64;
    let mut sum_wtx = 0.0_f64;
    let mut sum_wty = 0.0_f64;

    for (i, s) in samples.iter().enumerate() {
        let t = s.time.duration_since(t0).as_secs_f64();
        let w = (2.0 * i as f64 / n as f64).exp();

        sum_w += w;
        sum_wt += w * t;
        sum_wt2 += w * t * t;
        sum_wx += w * s.x as f64;
        sum_wy += w * s.y as f64;
        sum_wtx += w * t * s.x as f64;
        sum_wty += w * t * s.y as f64;
    }

    let denom = sum_w * sum_wt2 - sum_wt * sum_wt;
    if denom.abs() < 1e-12 {
        let first = samples[0];
        let last = samples[n - 1];
        let dt = last.time.duration_since(first.time).as_secs_f64();
        if dt < 1e-6 {
            return (0.0, 0.0);
        }
        return (
            ((last.x - first.x) as f64 / dt) as f32,
            ((last.y - first.y) as f64 / dt) as f32,
        );
    }

    let vx = (sum_w * sum_wtx - sum_wt * sum_wx) / denom;
    let vy = (sum_w * sum_wty - sum_wt * sum_wy) / denom;

    (vx as f32, vy as f32)
}

fn clamp_velocity(v: f32) -> f32 {
    v.clamp(-MAX_VELOCITY, MAX_VELOCITY)
}

// ── MomentumScroller ─────────────────────────────────────────────────────────

/// A momentum/inertia scroller that produces decelerating scroll deltas.
///
/// After calling [`fling`](Self::fling) with the release velocity, call
/// [`step`](Self::step) on every frame tick.  It returns `Some((dx, dy))`
/// as long as the scroller is animating, and `None` when finished.
pub struct MomentumScroller {
    /// Current velocity in logical px/s.
    vx: f32,
    vy: f32,
    /// Last position of the finger / last emitted position (logical px).
    last_x: f32,
    last_y: f32,
    /// Whether a fling is currently active.
    active: bool,
    /// Last step timestamp.
    last_time: Instant,
}

impl Default for MomentumScroller {
    fn default() -> Self {
        Self {
            vx: 0.0,
            vy: 0.0,
            last_x: 0.0,
            last_y: 0.0,
            active: false,
            last_time: Instant::now(),
        }
    }
}

impl MomentumScroller {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a fling animation from the given velocity (px/s) and
    /// last finger position.
    pub fn fling(&mut self, vx: f32, vy: f32, last_x: f32, last_y: f32) {
        let speed = (vx * vx + vy * vy).sqrt();
        if speed < MIN_VELOCITY {
            self.active = false;
            return;
        }
        self.vx = vx;
        self.vy = vy;
        self.last_x = last_x;
        self.last_y = last_y;
        self.active = true;
        self.last_time = Instant::now();
    }

    /// Advance the momentum animation by one frame.
    ///
    /// Returns `Some(MomentumDelta)` — the scroll delta in logical pixels
    /// and the current position — or `None` if the fling has finished
    /// (velocity dropped below threshold).
    ///
    /// The caller should emit a `ScrollWheel` event with the returned delta.
    pub fn step(&mut self) -> Option<MomentumDelta> {
        if !self.active {
            return None;
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last_time).as_secs_f64() as f32;
        self.last_time = now;

        // Guard against huge dt (e.g. app was suspended or a GC pause).
        // Cap at 33ms (~30 fps) — enough for variable frame rates but
        // prevents teleporting content after long stalls.
        let dt = dt.min(0.033);

        if dt < 1e-6 {
            return None;
        }

        // Apply exponential deceleration: v *= DECELERATION_RATE^(dt_ms)
        // where dt_ms = dt * 1000.
        let dt_ms = dt * 1000.0;
        let decay = DECELERATION_RATE.powf(dt_ms);

        // Compute displacement using the analytical integral of
        // v * r^t over [0, dt_ms]:
        //   ∫₀^{dt_ms} v·r^t dt = v · (r^dt_ms − 1) / ln(r)
        //
        // This is more accurate than the simple v*dt approximation,
        // especially during the high-velocity early phase where v*dt
        // overshoot can make scrolling feel jumpy.
        let ln_r = DECELERATION_RATE.ln();
        let displacement_factor = if ln_r.abs() > 1e-9 {
            (decay - 1.0) / (ln_r * 1000.0) // divide by 1000 to convert ms→s
        } else {
            dt // degenerate: r≈1, integral ≈ v*dt
        };

        let dx = self.vx * displacement_factor;
        let dy = self.vy * displacement_factor;

        // Now apply decay to velocity for the next frame.
        self.vx *= decay;
        self.vy *= decay;

        let speed = (self.vx * self.vx + self.vy * self.vy).sqrt();
        if speed < MIN_VELOCITY {
            self.active = false;
            // Include the final displacement so the last frame isn't lost.
            if dx.abs() < 0.1 && dy.abs() < 0.1 {
                return None;
            }
        }

        Some(MomentumDelta {
            dx,
            dy,
            // Keep position fixed at the finger-lift point.  GPUI uses
            // this for hit-testing (`should_handle_scroll`), so a drifting
            // position that goes off-screen would cause momentum events to
            // be silently dropped.
            position_x: self.last_x,
            position_y: self.last_y,
        })
    }

    /// Cancel any active fling (e.g. when a new touch begins).
    pub fn cancel(&mut self) {
        self.active = false;
        self.vx = 0.0;
        self.vy = 0.0;
    }

    /// Whether a fling animation is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The X position (finger-lift point), for use in ScrollWheel events.
    pub fn position_x(&self) -> f32 {
        self.last_x
    }

    /// The Y position (finger-lift point), for use in ScrollWheel events.
    pub fn position_y(&self) -> f32 {
        self.last_y
    }
}

/// The output of a single momentum step.
#[derive(Clone, Copy, Debug)]
pub struct MomentumDelta {
    /// Scroll delta X in logical pixels (positive = scroll right).
    pub dx: f32,
    /// Scroll delta Y in logical pixels (positive = scroll down).
    pub dy: f32,
    /// Current logical position X (for the ScrollWheel event's `position`).
    pub position_x: f32,
    /// Current logical position Y.
    pub position_y: f32,
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn velocity_tracker_no_samples() {
        let tracker = VelocityTracker::new();
        let (vx, vy) = tracker.velocity();
        assert_eq!(vx, 0.0);
        assert_eq!(vy, 0.0);
    }

    #[test]
    fn velocity_tracker_single_sample() {
        let mut tracker = VelocityTracker::new();
        tracker.record(100.0, 200.0);
        let (vx, vy) = tracker.velocity();
        // Only one sample → cannot compute velocity.
        assert_eq!(vx, 0.0);
        assert_eq!(vy, 0.0);
    }

    #[test]
    fn velocity_tracker_multiple_samples() {
        let mut tracker = VelocityTracker::new();
        // Simulate a ~1000 px/s vertical scroll by recording samples
        // spaced ~5 ms apart.
        for i in 0..10 {
            tracker.record(100.0, 100.0 + i as f32 * 5.0);
            thread::sleep(Duration::from_millis(5));
        }
        let (_vx, vy) = tracker.velocity();
        // We expect vy roughly around 1000 px/s (5 px / 5 ms = 1000 px/s).
        // Allow very generous tolerance because thread::sleep is extremely
        // imprecise on CI runners (5 ms sleeps can actually take 30-50 ms).
        assert!(vy.abs() > 50.0, "vy={vy} should be > 50 px/s");
        assert!(vy > 0.0, "vy should be positive (scrolling down)");
    }

    #[test]
    fn velocity_tracker_reset() {
        let mut tracker = VelocityTracker::new();
        tracker.record(0.0, 0.0);
        thread::sleep(Duration::from_millis(5));
        tracker.record(50.0, 50.0);
        tracker.reset();
        let (vx, vy) = tracker.velocity();
        assert_eq!(vx, 0.0);
        assert_eq!(vy, 0.0);
    }

    #[test]
    fn velocity_clamped() {
        let mut tracker = VelocityTracker::new();
        // Two samples extremely close in time but far apart in space →
        // extremely high velocity that should be clamped.
        tracker.record(0.0, 0.0);
        // Record immediately (no sleep) — dt ≈ 0 → huge velocity.
        tracker.record(10000.0, 10000.0);
        thread::sleep(Duration::from_micros(100));
        tracker.record(20000.0, 20000.0);
        let (vx, vy) = tracker.velocity();
        assert!(vx.abs() <= MAX_VELOCITY, "vx should be clamped");
        assert!(vy.abs() <= MAX_VELOCITY, "vy should be clamped");
    }

    #[test]
    fn momentum_scroller_no_fling() {
        let mut scroller = MomentumScroller::new();
        assert!(!scroller.is_active());
        assert!(scroller.step().is_none());
    }

    #[test]
    fn momentum_scroller_below_threshold() {
        let mut scroller = MomentumScroller::new();
        // Velocity below MIN_VELOCITY → should not start.
        scroller.fling(5.0, 5.0, 100.0, 100.0);
        assert!(!scroller.is_active());
        assert!(scroller.step().is_none());
    }

    #[test]
    fn momentum_scroller_decelerates() {
        let mut scroller = MomentumScroller::new();
        scroller.fling(0.0, 2000.0, 100.0, 100.0);
        assert!(scroller.is_active());

        // Run for several "frames" (sleeping ~16ms each to simulate 60fps).
        // With DECELERATION_RATE=0.998, a 2000px/s fling should run for
        // ~130 frames (~2.2 seconds) and travel ~1000px total.
        let mut deltas = Vec::new();
        let mut total_dy = 0.0_f32;
        let mut frame_count = 0;

        loop {
            thread::sleep(Duration::from_millis(16));
            match scroller.step() {
                Some(delta) => {
                    assert!(delta.dy >= 0.0, "dy should be >= 0, got {}", delta.dy);
                    deltas.push(delta.dy);
                    total_dy += delta.dy;
                    frame_count += 1;
                }
                None => break,
            }
            // Safety: don't run forever.
            if frame_count > 1000 {
                break;
            }
        }

        assert!(!scroller.is_active());
        assert!(total_dy > 200.0, "total_dy={total_dy} should be > 200 px");
        assert!(
            frame_count > 20,
            "should have run for many frames, got {frame_count}"
        );

        // Verify overall deceleration: the average delta in the first
        // quarter of frames should be larger than the last quarter.
        let q = deltas.len() / 4;
        if q > 0 {
            let first_avg: f32 = deltas[..q].iter().sum::<f32>() / q as f32;
            let last_avg: f32 = deltas[deltas.len() - q..].iter().sum::<f32>() / q as f32;
            assert!(
                first_avg > last_avg,
                "first quarter avg ({first_avg}) should be > last quarter avg ({last_avg})"
            );
        }
    }

    #[test]
    fn momentum_scroller_cancel() {
        let mut scroller = MomentumScroller::new();
        scroller.fling(0.0, 3000.0, 100.0, 100.0);
        assert!(scroller.is_active());

        thread::sleep(Duration::from_millis(16));
        assert!(scroller.step().is_some());

        scroller.cancel();
        assert!(!scroller.is_active());
        assert!(scroller.step().is_none());
    }

    #[test]
    fn momentum_scroller_fling_replaces_previous() {
        let mut scroller = MomentumScroller::new();
        scroller.fling(0.0, 2000.0, 100.0, 100.0);
        assert!(scroller.is_active());

        // Starting a new fling should replace the old one.
        scroller.fling(1000.0, 0.0, 50.0, 50.0);
        assert!(scroller.is_active());

        thread::sleep(Duration::from_millis(16));
        if let Some(delta) = scroller.step() {
            // Should be moving mostly in X now, not Y.
            assert!(
                delta.dx.abs() > delta.dy.abs(),
                "dx={} should dominate dy={}",
                delta.dx,
                delta.dy
            );
        }
    }

    #[test]
    fn ring_buffer_wraps() {
        let mut tracker = VelocityTracker::new();
        // Record more than MAX_SAMPLES to ensure wrapping works.
        for i in 0..(MAX_SAMPLES * 2) {
            tracker.record(i as f32, i as f32);
            thread::sleep(Duration::from_millis(1));
        }
        // Should still produce a reasonable velocity.
        let (vx, vy) = tracker.velocity();
        assert!(vx.abs() > 0.0 || vy.abs() > 0.0);
    }
}
