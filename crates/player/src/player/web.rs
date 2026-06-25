use super::NowPlayingMeta;
use config::{ChannelMode, EqualizerSettings};
use std::time::Duration;
use wasm_bindgen::JsCast;

fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

const WEB_EQ_BAND_FREQUENCIES: [f32; 5] = [60.0, 250.0, 1_000.0, 4_000.0, 12_000.0];
const WEB_EQ_BAND_Q: [f32; 5] = [0.9, 1.0, 1.0, 0.9, 0.8];

pub struct Player {
    audio: web_sys::HtmlAudioElement,
    audio_context: web_sys::AudioContext,
    _source_node: web_sys::MediaElementAudioSourceNode,
    preamp_node: web_sys::GainNode,
    eq_filters: [web_sys::BiquadFilterNode; 5],
    _channel_splitter: web_sys::ChannelSplitterNode,
    _channel_merger: web_sys::ChannelMergerNode,
    routing_gains: [[web_sys::GainNode; 2]; 2],
    channel_mode: ChannelMode,
    volume: f32,
    /// True once play_url has been called and not yet stopped
    has_source: bool,
}

#[cfg(target_arch = "wasm32")]
impl Player {
    pub fn new() -> Self {
        let audio = web_sys::HtmlAudioElement::new().expect("HtmlAudioElement creation failed");
        audio.set_preload("auto");

        let audio_context = web_sys::AudioContext::new().expect("AudioContext creation failed");
        let preamp_node = audio_context
            .create_gain()
            .expect("GainNode creation failed");
        let eq_filters = std::array::from_fn(|index| {
            let filter = audio_context
                .create_biquad_filter()
                .expect("BiquadFilterNode creation failed");
            filter.set_type(web_sys::BiquadFilterType::Peaking);
            filter.frequency().set_value(WEB_EQ_BAND_FREQUENCIES[index]);
            filter.q().set_value(WEB_EQ_BAND_Q[index]);
            filter.gain().set_value(0.0);
            filter
        });
        let channel_splitter = audio_context
            .create_channel_splitter_with_number_of_outputs(2)
            .expect("ChannelSplitterNode creation failed");
        let channel_merger = audio_context
            .create_channel_merger_with_number_of_inputs(2)
            .expect("ChannelMergerNode creation failed");

        let media_element: web_sys::HtmlMediaElement = audio.clone().unchecked_into();
        let source_node = audio_context
            .create_media_element_source(&media_element)
            .expect("MediaElementAudioSourceNode creation failed");

        source_node
            .connect_with_audio_node(&preamp_node)
            .expect("source -> preamp connection failed");

        let mut previous: web_sys::AudioNode = preamp_node.clone().unchecked_into();
        for filter in &eq_filters {
            previous
                .connect_with_audio_node(filter.as_ref())
                .expect("filter connection failed");
            previous = filter.clone().unchecked_into();
        }
        previous
            .connect_with_audio_node(&channel_splitter)
            .expect("EQ -> channel splitter connection failed");

        let routing_gains = std::array::from_fn(|src| {
            std::array::from_fn(|dst| {
                let gain = audio_context
                    .create_gain()
                    .expect("channel routing GainNode creation failed");
                gain.gain().set_value(0.0);
                channel_splitter
                    .connect_with_audio_node_and_output(&gain, src as u32)
                    .expect("channel splitter -> routing gain connection failed");
                gain.connect_with_audio_node_and_output_and_input(&channel_merger, 0, dst as u32)
                    .expect("routing gain -> channel merger connection failed");
                gain
            })
        });

        channel_merger
            .connect_with_audio_node(&audio_context.destination())
            .expect("destination connection failed");

        let mut player = Self {
            audio,
            audio_context,
            _source_node: source_node,
            preamp_node,
            eq_filters,
            _channel_splitter: channel_splitter,
            _channel_merger: channel_merger,
            routing_gains,
            channel_mode: ChannelMode::Stereo,
            volume: 1.0,
            has_source: false,
        };
        player.set_channel_mode(ChannelMode::Stereo);
        player.set_equalizer(EqualizerSettings::default());
        player
    }

    /// No-op on web; auto-advance is handled by the 250ms polling loop
    /// (which calls `is_empty()` → `audio.ended()`).
    pub fn set_finish_callback(&mut self, _f: impl Fn() + Send + Sync + 'static) {}

    /// Primary play method for web — sets the `<audio>` src and starts playback.
    pub fn play_url(&mut self, url: String, _meta: NowPlayingMeta) {
        self.audio.set_src(&url);
        self.audio.set_volume(self.volume as f64);
        if let Err(error) = self.audio_context.resume() {
            web_sys::console::error_1(&error);
        }
        match self.audio.play() {
            Ok(_) => self.has_source = true,
            Err(_) => self.has_source = false,
        }
    }

    pub fn crossfade_to(&mut self, url: String, meta: NowPlayingMeta, _duration: Duration) {
        self.play_url(url, meta);
    }

    pub fn pause(&mut self) {
        let _ = self.audio.pause();
    }

    pub fn play_resume(&mut self) {
        if let Err(error) = self.audio_context.resume() {
            web_sys::console::error_1(&error);
        }
        let _ = self.audio.play();
    }

    pub fn seek(&mut self, time: Duration) {
        self.audio.set_current_time(time.as_secs_f64());
    }

    pub fn stop(&mut self) {
        let _ = self.audio.pause();
        self.audio.set_src("");
        self.has_source = false;
    }

    pub fn stop_for_transition(&mut self) {
        let _ = self.audio.pause();
        self.has_source = false;
    }

    pub fn set_volume(&mut self, volume: f32) {
        let gain = volume.clamp(0.0, 1.0).powi(3);
        self.volume = gain;
        self.audio.set_volume(gain as f64);
    }

    pub fn set_channel_mode(&mut self, mode: ChannelMode) {
        self.channel_mode = mode;
        self.update_channel_routing();
    }

    pub fn set_equalizer(&mut self, settings: EqualizerSettings) {
        let resolved_bands = if settings.enabled {
            settings.resolved_bands()
        } else {
            [0.0; 5]
        };
        let max_boost = resolved_bands
            .iter()
            .copied()
            .fold(0.0_f32, f32::max)
            .max(0.0);
        let preamp = if settings.enabled {
            db_to_linear(settings.preamp_db - max_boost)
        } else {
            1.0
        };

        self.preamp_node.gain().set_value(preamp);

        for (index, filter) in self.eq_filters.iter().enumerate() {
            filter.gain().set_value(resolved_bands[index]);
        }
    }

    fn update_channel_routing(&self) {
        let gains = match self.channel_mode {
            ChannelMode::Stereo => [[1.0, 0.0], [0.0, 1.0]],
            ChannelMode::Mono => [[0.5, 0.5], [0.5, 0.5]],
            ChannelMode::LeftOnly => [[1.0, 0.0], [0.0, 0.0]],
            ChannelMode::RightOnly => [[0.0, 0.0], [0.0, 1.0]],
            ChannelMode::SwapLeftRight => [[0.0, 1.0], [1.0, 0.0]],
        };

        for (src, outputs) in self.routing_gains.iter().enumerate() {
            for (dst, gain) in outputs.iter().enumerate() {
                gain.gain().set_value(gains[src][dst]);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.has_source || self.audio.ended() || self.audio.error().is_some()
    }

    pub fn is_playback_complete(&self) -> bool {
        self.is_empty()
    }

    pub fn is_paused(&self) -> bool {
        self.audio.paused()
    }

    pub fn can_resume(&self) -> bool {
        self.has_source && !self.audio.ended() && self.audio.error().is_none()
    }

    pub fn get_position(&self) -> Duration {
        Duration::from_secs_f64(self.audio.current_time())
    }

    pub fn update_metadata(&mut self, _meta: NowPlayingMeta) {
        // No system media integration on web
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}
