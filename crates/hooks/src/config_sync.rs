//! Keep the app's settings copy in step with the daemon's.

use dioxus::prelude::*;
use serde_json::{Map, Value};

/// Volume rides the player state, so the settings copy never takes it from here.
const SKIPPED: &[&str] = &["volume"];

/// The config this app last sent or adopted and the daemon revision it matches, which tells the echo of its own write from another writer's change.
#[derive(Clone, Copy)]
pub struct ConfigBaseline(Signal<Option<(config::AppConfig, u64)>>);

pub fn use_config_baseline_provider() -> ConfigBaseline {
    use_context_provider(|| ConfigBaseline(Signal::new(None)))
}

pub fn use_config_baseline() -> ConfigBaseline {
    use_context::<ConfigBaseline>()
}

impl ConfigBaseline {
    pub fn update(self, config: config::AppConfig) -> Option<api::ConfigUpdate> {
        self.0
            .read()
            .as_ref()
            .map(|(_, revision)| api::ConfigUpdate {
                config,
                expected_revision: *revision,
            })
    }

    /// Start from the view the app loaded.
    pub fn loaded(mut self, view: &api::ConfigView) {
        self.0.set(Some((view.config.clone(), view.revision)));
    }

    /// Acknowledge a save without rolling back a newer event or an edit made during the request.
    pub fn acknowledge(
        mut self,
        mut local: Signal<config::AppConfig>,
        sent: &config::AppConfig,
        view: &api::ConfigView,
    ) {
        let (daemon, revision) = self
            .0
            .peek()
            .clone()
            .filter(|(_, revision)| *revision > view.revision)
            .unwrap_or_else(|| (view.config.clone(), view.revision));
        let current = local.peek().clone();
        if let Some((next, _)) = moved(&current, sent, &daemon) {
            local.set(next);
        }
        self.0.set(Some((daemon, revision)));
    }

    /// Take into `local` every field a newer `view` moved since the baseline, keeping edits not yet sent.
    pub fn adopt(mut self, mut local: Signal<config::AppConfig>, view: &api::ConfigView) {
        let Some((baseline, revision)) = self.0.peek().clone() else {
            return;
        };

        if view.revision != 0 && view.revision <= revision {
            return;
        }
        let current = local.peek().clone();
        match moved(&current, &baseline, &view.config) {
            Some((next_local, next_baseline)) => {
                self.0.set(Some((next_baseline, view.revision)));
                local.set(next_local);
            }
            None => self.0.set(Some((baseline, view.revision))),
        }
    }
}

fn fields(config: &config::AppConfig) -> Option<Map<String, Value>> {
    match serde_json::to_value(config) {
        Ok(Value::Object(map)) => Some(map),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%error, "settings did not serialize");
            None
        }
    }
}

fn rebuild(map: Map<String, Value>) -> Option<config::AppConfig> {
    serde_json::from_value(Value::Object(map))
        .map_err(|error| tracing::warn!(%error, "settings did not deserialize"))
        .ok()
}

/// `daemon`'s fields that differ from `baseline`, laid over `local` and over `baseline`; `None` when none moved.
fn moved(
    local: &config::AppConfig,
    baseline: &config::AppConfig,
    daemon: &config::AppConfig,
) -> Option<(config::AppConfig, config::AppConfig)> {
    let (mut local, mut baseline) = (fields(local)?, fields(baseline)?);
    let mut changed = false;
    for (key, value) in fields(daemon)? {
        if SKIPPED.contains(&key.as_str()) || baseline.get(&key) == Some(&value) {
            continue;
        }
        if local.get(&key) == baseline.get(&key) {
            local.insert(key.clone(), value.clone());
        }
        baseline.insert(key, value);
        changed = true;
    }
    if !changed {
        return None;
    }
    Some((rebuild(local)?, rebuild(baseline)?))
}

#[cfg(test)]
mod tests {
    use super::moved;

    fn config(crossfade: u8, lyrics: bool) -> config::AppConfig {
        config::AppConfig {
            crossfade_seconds: crossfade,
            prefer_local_lyrics: lyrics,
            ..Default::default()
        }
    }

    #[test]
    fn a_conflict_keeps_the_unsent_edit_while_advancing_the_baseline() {
        let baseline = config(4, false);
        let local = config(5, false);
        let daemon = config(6, true);
        let (next, base) = moved(&local, &baseline, &daemon).expect("remote change");
        assert_eq!(next.crossfade_seconds, 5);
        assert!(next.prefer_local_lyrics);
        assert_eq!(base.crossfade_seconds, 6);
    }

    #[test]
    fn the_echo_of_a_write_does_not_undo_an_edit_made_since() {
        let sent = config(4, false);
        let local = config(5, false);
        assert!(moved(&local, &sent, &sent).is_none());
    }

    #[test]
    fn another_writers_change_lands_beside_an_unsent_edit() {
        let baseline = config(4, false);
        let local = config(5, false);
        let daemon = config(4, true);
        let (next, base) = moved(&local, &baseline, &daemon).expect("a field moved");
        assert_eq!(
            (next.crossfade_seconds, next.prefer_local_lyrics),
            (5, true)
        );
        assert_eq!(
            (base.crossfade_seconds, base.prefer_local_lyrics),
            (4, true)
        );
    }

    #[test]
    fn a_field_only_the_daemon_writes_reaches_the_copy() {
        let baseline = config(4, false);
        let mut daemon = baseline.clone();
        daemon.offline_tracks.insert("k".into(), "/cache/k".into());
        let (next, _) = moved(&baseline, &baseline, &daemon).expect("downloads moved");
        assert_eq!(
            next.offline_tracks.get("k").map(String::as_str),
            Some("/cache/k")
        );
    }

    #[test]
    fn volume_never_comes_from_the_settings_copy() {
        let baseline = config(4, false);
        let daemon = config::AppConfig {
            volume: 0.1,
            ..baseline.clone()
        };
        assert!(moved(&baseline, &baseline, &daemon).is_none());
    }
}
