//! Tells the integration plugins that listen to audio how the music sounds:
//! the player's meter, read about twenty times a second while one listens,
//! with onsets and likely beats worked out here so a plugin needs no signal
//! processing of its own.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use player::meter::{AudioMeter, Levels};
use plugin_host::manifest::Listen;
use plugin_host::wit;

use crate::plugin_integrations::PluginIntegrations;
use crate::plugin_listeners::{Queue, listening};

const TICK: Duration = Duration::from_millis(50);

/// How much audio each frame covers.
const WINDOW: Duration = Duration::from_millis((player::meter::WINDOW_SECONDS * 1000.0) as u64);

/// How often the listeners are looked up again, and how long to wait while none listen.
const RECHECK: Duration = Duration::from_secs(1);

/// Frames waiting for a plugin; a slow one misses the rest instead of hearing them late.
const QUEUE: usize = 2;

/// About a second of onsets, which a beat has to stand out from.
const HISTORY: usize = 20;

/// Reads in a row with nothing measured that count as the music stopping: a
/// pause, a stop or a gap between tracks, after which the beat finder starts
/// over instead of comparing the next song with the last.
const QUIET_READS: u32 = 4;

/// No beat follows another closer than this: 240 beats a minute.
const BEAT_GAP: Duration = Duration::from_millis(250);

/// An onset this far above the recent average is a beat, if it is loud enough at all.
const BEAT_RATIO: f32 = 1.6;
const BEAT_FLOOR: f32 = 0.02;

/// Works out onsets and beats from one window after another.
#[derive(Debug)]
pub(crate) struct Beats {
    previous: Option<Levels>,
    onsets: VecDeque<f32>,
    since_beat: Duration,
}

impl Default for Beats {
    /// As though the last beat were long past, so the first one counts.
    fn default() -> Self {
        Self {
            previous: None,
            onsets: VecDeque::with_capacity(HISTORY + 1),
            since_beat: BEAT_GAP,
        }
    }
}

impl Beats {
    /// The frame for `levels`, `elapsed` after the one before it.
    pub(crate) fn frame(&mut self, levels: Levels, elapsed: Duration) -> wit::AudioFrame {
        let before = self.previous.unwrap_or_default();
        // Kicks carry a beat more often than cymbals do.
        let onset = 1.5 * (levels.low - before.low).max(0.0)
            + (levels.mid - before.mid).max(0.0)
            + 0.5 * (levels.high - before.high).max(0.0);
        let recent = match self.onsets.is_empty() {
            true => 0.0,
            false => self.onsets.iter().sum::<f32>() / self.onsets.len() as f32,
        };
        self.since_beat = self.since_beat.saturating_add(elapsed);
        let beat =
            onset >= BEAT_FLOOR && onset > recent * BEAT_RATIO && self.since_beat >= BEAT_GAP;
        if beat {
            self.since_beat = Duration::ZERO;
        }
        self.onsets.push_back(onset);
        if self.onsets.len() > HISTORY {
            self.onsets.pop_front();
        }
        self.previous = Some(levels);
        wit::AudioFrame {
            level: levels.level,
            peak: levels.peak,
            low: levels.low,
            mid: levels.mid,
            high: levels.high,
            onset,
            beat,
        }
    }
}

fn audio_queue(integration: Arc<plugin_host::Integration>) -> Queue<Vec<wit::AudioFrame>> {
    Queue::open(
        integration,
        QUEUE,
        "audio",
        |integration, frames: Vec<wit::AudioFrame>| async move { integration.on_audio(&frames).await },
    )
}

/// Read the meter for as long as the daemon runs, measuring only while a plugin listens.
pub fn spawn(
    meter: Arc<AudioMeter>,
    plugins: Arc<PluginIntegrations>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut queues: HashMap<String, Queue<Vec<wit::AudioFrame>>> = HashMap::new();
        let mut beats = Beats::default();
        let mut seen = meter.windows();
        let mut since_recheck = RECHECK;
        let mut quiet = 0;
        loop {
            if since_recheck >= RECHECK {
                since_recheck = Duration::ZERO;
                for integration in listening(&plugins, &mut queues, Listen::Audio) {
                    let id = integration.plugin().id().to_string();
                    queues.entry(id).or_insert_with(|| audio_queue(integration));
                }
                meter.set_wanted(!queues.is_empty());
            }
            if queues.is_empty() {
                beats = Beats::default();
                // A plugin that starts listening hears the music from then, not what came before.
                seen = meter.windows();
                tokio::time::sleep(RECHECK).await;
                since_recheck = RECHECK;
                continue;
            }
            tokio::time::sleep(TICK).await;
            since_recheck += TICK;
            let (windows, measured) = meter.since(seen);
            seen = windows;
            // Paused or between tracks: nothing was measured, so nothing is said.
            if measured.is_empty() {
                quiet += 1;
                if quiet == QUIET_READS {
                    beats = Beats::default();
                }
                continue;
            }
            quiet = 0;
            let frames: Vec<wit::AudioFrame> = measured
                .into_iter()
                .map(|levels| beats.frame(levels, WINDOW))
                .collect();
            for queue in queues.values() {
                queue.send(frames.clone());
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_millis(50);

    fn levels(low: f32, mid: f32, high: f32) -> Levels {
        Levels {
            level: (low + mid + high) / 3.0,
            peak: low.max(mid).max(high),
            low,
            mid,
            high,
        }
    }

    #[test]
    fn a_held_note_has_no_onset_after_it_starts() {
        let mut beats = Beats::default();
        let first = beats.frame(levels(0.3, 0.3, 0.1), WINDOW);
        assert!(first.onset > 0.0);
        for _ in 0..10 {
            let held = beats.frame(levels(0.3, 0.3, 0.1), WINDOW);
            assert_eq!(held.onset, 0.0);
            assert!(!held.beat);
        }
    }

    #[test]
    fn a_kick_every_half_second_is_a_beat_each_time() {
        let mut beats = Beats::default();
        let mut at = Vec::new();
        for index in 0..60 {
            // A kick on every tenth window, quiet in between.
            let frame = match index % 10 {
                0 => levels(0.6, 0.2, 0.05),
                _ => levels(0.05, 0.15, 0.05),
            };
            if beats.frame(frame, WINDOW).beat {
                at.push(index);
            }
        }
        assert_eq!(at, [0, 10, 20, 30, 40, 50]);
    }

    #[test]
    fn beats_closer_than_a_quarter_second_count_once() {
        let mut beats = Beats::default();
        let mut found = 0;
        for index in 0..8 {
            let frame = match index % 2 {
                0 => levels(0.6, 0.2, 0.05),
                _ => levels(0.0, 0.0, 0.0),
            };
            found += usize::from(beats.frame(frame, WINDOW).beat);
        }
        // Eight hits 100 ms apart over 400 ms: one every 250 ms at most.
        assert_eq!(found, 2);
    }

    #[test]
    fn a_quiet_flicker_is_no_beat() {
        let mut beats = Beats::default();
        for index in 0..20 {
            let low = if index % 2 == 0 { 0.012 } else { 0.0 };
            assert!(!beats.frame(levels(low, 0.0, 0.0), WINDOW).beat);
        }
    }
}
