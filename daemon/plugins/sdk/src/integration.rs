//! Build an integration, a plugin that adds something besides a source:
//! implement [`Integration`] and export it with
//! [`export_integration!`](crate::export_integration). The manifest's
//! `[integration]` table lists the fields the person fills in, and the host
//! opens it with their answers.

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../plugin-host/wit",
        world: "integration-plugin",
        pub_export_macro: true,
        export_macro_name: "__export_integration_plugin",
        default_bindings_module: "kopuz_plugin_sdk::integration::bindings",
        additional_derives: [PartialEq],
        with: {
            "kopuz:plugin/types@0.1.0": crate::bindings::kopuz::plugin::types,
            "kopuz:plugin/http@0.1.0": crate::bindings::kopuz::plugin::http,
            "kopuz:plugin/log@0.1.0": crate::bindings::kopuz::plugin::log,
            "kopuz:plugin/tcp@0.1.0": crate::bindings::kopuz::plugin::tcp,
        },
    });
}

pub use bindings::exports::kopuz::plugin::integration::{
    AudioFrame, NowPlaying, PlaybackEvent, Setting, Started,
};

use crate::Result;

/// The answers the integration was opened with, by field key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings(Vec<Setting>);

impl Settings {
    pub fn new(settings: Vec<Setting>) -> Self {
        Self(settings)
    }

    /// The answer to `key`, or `None` when the person left it empty.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|setting| setting.key == key)
            .map(|setting| setting.value.as_str())
            .filter(|value| !value.is_empty())
    }

    /// A toggle field's answer; unanswered is `false`.
    pub fn flag(&self, key: &str) -> bool {
        self.get(key) == Some("true")
    }
}

/// What an integration does. The host keeps one value of it while the
/// person has it set up, so it can hold a connection open.
pub trait Integration: Sized + 'static {
    fn open(settings: Settings) -> Self;

    /// Checked when the settings are saved; an error is shown and nothing is kept.
    fn validate(&self) -> Result<()> {
        Ok(())
    }

    /// What playback just did, for a manifest that lists `"playback"` in
    /// `listens`. Calls come one at a time, in order; a slow one holds up the next.
    fn on_playback(&self, event: PlaybackEvent) {
        let _ = event;
    }

    /// How the music sounds, about twenty times a second while it plays, for
    /// a manifest that lists `"audio"` in `listens`. A slow answer misses
    /// frames rather than hearing them late.
    fn on_audio(&self, frames: Vec<AudioFrame>) {
        let _ = frames;
    }

    /// Every `tick-ms`, for a manifest that lists `"tick"` in `listens`: the
    /// place to look for what the plugin waits on, such as a remote's button.
    fn on_tick(&self) {}
}

/// Steer what plays, for a manifest with `playback = "control"` under
/// `[permissions]`; any other plugin's calls are refused.
pub mod playback {
    pub use super::bindings::kopuz::plugin::playback::{Command, QueueMode};

    pub fn control(command: Command) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        return super::bindings::kopuz::plugin::playback::control(command);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = command;
            Err("there is no host outside wasm".into())
        }
    }

    /// Queue library track keys, as playback events name them.
    pub fn queue(keys: &[String], mode: QueueMode) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        return super::bindings::kopuz::plugin::playback::queue(keys, mode);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (keys, mode);
            Err("there is no host outside wasm".into())
        }
    }
}

#[macro_export]
macro_rules! export_integration {
    ($integration:ty) => {
        #[doc(hidden)]
        pub struct __KopuzIntegration;

        #[doc(hidden)]
        pub struct __KopuzInstance($integration);

        impl $crate::integration::bindings::exports::kopuz::plugin::integration::Guest
            for __KopuzIntegration
        {
            type Instance = __KopuzInstance;
        }

        impl $crate::integration::bindings::exports::kopuz::plugin::integration::GuestInstance
            for __KopuzInstance
        {
            fn new(settings: ::std::vec::Vec<$crate::integration::Setting>) -> Self {
                __KopuzInstance(<$integration as $crate::integration::Integration>::open(
                    $crate::integration::Settings::new(settings),
                ))
            }
            fn validate(&self) -> ::std::result::Result<(), $crate::SourceError> {
                $crate::integration::Integration::validate(&self.0)
            }
            fn on_playback(&self, event: $crate::integration::PlaybackEvent) {
                $crate::integration::Integration::on_playback(&self.0, event)
            }
            fn on_audio(&self, frames: ::std::vec::Vec<$crate::integration::AudioFrame>) {
                $crate::integration::Integration::on_audio(&self.0, frames)
            }
            fn on_tick(&self) {
                $crate::integration::Integration::on_tick(&self.0)
            }
        }

        $crate::integration::bindings::__export_integration_plugin!(
            __KopuzIntegration with_types_in $crate::integration::bindings
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(key: &str, value: &str) -> Setting {
        Setting {
            key: key.into(),
            value: value.into(),
        }
    }

    #[test]
    fn an_empty_answer_reads_as_none_and_a_toggle_as_a_flag() {
        let settings = Settings::new(vec![
            setting("url", "ws://127.0.0.1:12345"),
            setting("token", ""),
            setting("vibrate", "true"),
            setting("rotate", "false"),
        ]);
        assert_eq!(settings.get("url"), Some("ws://127.0.0.1:12345"));
        assert_eq!(settings.get("token"), None);
        assert_eq!(settings.get("missing"), None);
        assert!(settings.flag("vibrate"));
        assert!(!settings.flag("rotate"));
        assert!(!settings.flag("missing"));
    }
}
