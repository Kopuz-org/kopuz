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
    ActionTarget, AudioFrame, Block, FoundLyrics, NowPlaying, PanelAction, PanelTarget,
    PlaybackEvent, Setting, Song, Started,
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

    /// One of the manifest's `[[integration.action]]` entries, chosen by the
    /// person; `input` is their answer when it has an `ask`. `Ok(Some(text))`
    /// is shown to them as a notice, `Err` as a failure.
    fn run_action(
        &self,
        id: &str,
        target: ActionTarget,
        input: Option<String>,
    ) -> std::result::Result<Option<String>, String> {
        let _ = (target, input);
        Err(format!("this plugin has no action {id}"))
    }

    /// One of the manifest's `[[integration.panel]]` entries, for the album or
    /// artist page it is drawn on. An empty list draws nothing.
    fn render_panel(
        &self,
        id: &str,
        target: PanelTarget,
    ) -> std::result::Result<Vec<Block>, String> {
        let _ = target;
        Err(format!("this plugin has no panel {id}"))
    }

    /// Words for `track`, asked only when the manifest's `[integration]`
    /// provides `lyrics` and kopuz's own sources found none. `Ok(None)` is
    /// "nothing found".
    fn lyrics(&self, track: Song) -> std::result::Result<Option<FoundLyrics>, String> {
        let _ = track;
        Ok(None)
    }

    /// Library track keys to play after `seed` when the queue runs out,
    /// asked only when the manifest's `[integration]` provides `radio` and
    /// autoplay is on. `recent` is the queue so far, newest last. An empty
    /// list lets kopuz's own radio go on.
    fn radio(&self, seed: Song, recent: Vec<String>) -> std::result::Result<Vec<String>, String> {
        let _ = (seed, recent);
        Ok(Vec::new())
    }
}

/// Read the library, for a manifest with `library = "read"` under
/// `[permissions]`; any other plugin's calls are refused. Tracks are named by
/// the keys playback events carry and [`playback::queue`] takes.
pub mod library {
    pub use super::bindings::kopuz::plugin::library::{Playlist, Track, TrackPage};

    #[cfg(target_arch = "wasm32")]
    use super::bindings::kopuz::plugin::library as host;

    #[cfg(not(target_arch = "wasm32"))]
    fn no_host<T>() -> Result<T, String> {
        Err("there is no host outside wasm".into())
    }

    /// The active source's library, as its search box searches it; at most 500.
    pub fn search(query: &str, limit: u32) -> Result<Vec<Track>, String> {
        #[cfg(target_arch = "wasm32")]
        return host::search(query, limit);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (query, limit);
            no_host()
        }
    }

    /// The tracks with these keys, in that order; at most 500 keys.
    pub fn tracks(keys: &[String]) -> Result<Vec<Track>, String> {
        #[cfg(target_arch = "wasm32")]
        return host::tracks(keys);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = keys;
            no_host()
        }
    }

    pub fn album_tracks(album_id: &str) -> Result<Vec<Track>, String> {
        #[cfg(target_arch = "wasm32")]
        return host::album_tracks(album_id);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = album_id;
            no_host()
        }
    }

    /// The tracks of the artist with this library key, as an artist page and its actions name it.
    pub fn artist_tracks(artist_key: &str) -> Result<Vec<Track>, String> {
        #[cfg(target_arch = "wasm32")]
        return host::artist_tracks(artist_key);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = artist_key;
            no_host()
        }
    }

    /// Every track, a page at a time; at most 500 a page.
    pub fn tracks_page(offset: u32, limit: u32) -> Result<TrackPage, String> {
        #[cfg(target_arch = "wasm32")]
        return host::tracks_page(offset, limit);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (offset, limit);
            no_host()
        }
    }

    pub fn playlists() -> Result<Vec<Playlist>, String> {
        #[cfg(target_arch = "wasm32")]
        return host::playlists();
        #[cfg(not(target_arch = "wasm32"))]
        no_host()
    }

    /// The keys of the tracks marked favorite.
    pub fn favorites() -> Result<Vec<String>, String> {
        #[cfg(target_arch = "wasm32")]
        return host::favorites();
        #[cfg(not(target_arch = "wasm32"))]
        no_host()
    }
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
            fn run_action(
                &self,
                id: ::std::string::String,
                target: $crate::integration::ActionTarget,
                input: ::std::option::Option<::std::string::String>,
            ) -> ::std::result::Result<
                ::std::option::Option<::std::string::String>,
                ::std::string::String,
            > {
                $crate::integration::Integration::run_action(&self.0, &id, target, input)
            }
            fn render_panel(
                &self,
                id: ::std::string::String,
                target: $crate::integration::PanelTarget,
            ) -> ::std::result::Result<
                ::std::vec::Vec<$crate::integration::Block>,
                ::std::string::String,
            > {
                $crate::integration::Integration::render_panel(&self.0, &id, target)
            }
            fn lyrics(
                &self,
                track: $crate::integration::Song,
            ) -> ::std::result::Result<
                ::std::option::Option<$crate::integration::FoundLyrics>,
                ::std::string::String,
            > {
                $crate::integration::Integration::lyrics(&self.0, track)
            }
            fn radio(
                &self,
                seed: $crate::integration::Song,
                recent: ::std::vec::Vec<::std::string::String>,
            ) -> ::std::result::Result<
                ::std::vec::Vec<::std::string::String>,
                ::std::string::String,
            > {
                $crate::integration::Integration::radio(&self.0, seed, recent)
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
