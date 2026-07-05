use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::player::PlayerInitError;

/// Channel count and sample rate of an opened output stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SinkConfig {
    pub channels: usize,
    pub sample_rate: u32,
}

pub type DataCallback = Box<dyn FnMut(&mut [f32]) + Send + 'static>;

/// Builds the data callback once the final stream config is known — the device
/// may not honor the desired rate, and the callback's state (scratch buffers,
/// EQ) is sized from what was actually opened.
pub type DataCallbackFactory = Box<dyn FnOnce(SinkConfig) -> DataCallback>;

/// Minimal seam over the audio backend so the actor's state machine can run
/// headless in tests. The real implementation is [`CpalSink`].
pub trait AudioSink: Send {
    /// Config `open` would produce for this source rate, without rebuilding.
    fn probe_config(&mut self, desired_sample_rate: Option<u32>) -> Result<SinkConfig, String>;
    /// (Re)open the output stream, replacing any existing one. The previous
    /// stream and its data callback are dropped.
    fn open(
        &mut self,
        desired_sample_rate: Option<u32>,
        make_cb: DataCallbackFactory,
    ) -> Result<SinkConfig, String>;
    /// Config of the currently open stream, if any.
    fn config(&self) -> Option<SinkConfig>;
    fn play(&mut self) -> Result<(), String>;
    fn pause(&mut self);
    fn close(&mut self);
}

/// Out-of-band notifications from the audio backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkEvent {
    /// The running stream errored (device unplugged, format lost).
    StreamError,
    /// The OS default output device changed while our stream kept playing on
    /// the old one.
    DefaultDeviceChanged,
}

/// How often the watcher compares the OS default output device. Enumeration is
/// cheap on every backend, and a couple of seconds of migration latency is
/// imperceptible next to the device switch itself.
const DEVICE_WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

pub struct CpalSink {
    device: cpal::Device,
    stream: Option<cpal::Stream>,
    config: Option<SinkConfig>,
    on_event: std::sync::Arc<dyn Fn(SinkEvent) + Send + Sync + 'static>,
    watcher_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CpalSink {
    pub fn try_new(
        on_event: impl Fn(SinkEvent) + Send + Sync + 'static,
    ) -> Result<Self, PlayerInitError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(PlayerInitError::NoOutputDevice)?;

        let on_event = std::sync::Arc::new(on_event);
        let watcher_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        spawn_default_device_watcher(on_event.clone(), watcher_stop.clone());

        Ok(Self {
            device,
            stream: None,
            config: None,
            on_event,
            watcher_stop,
        })
    }

    fn preferred_stream_config(
        supported_config: &cpal::SupportedStreamConfig,
    ) -> cpal::StreamConfig {
        let mut stream_config = supported_config.config();
        stream_config.buffer_size = match supported_config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                // Android: larger buffer for stability under thermal throttling and when the
                // UI thread is busy (scroll, layout, image decode). ~46ms at 44.1kHz is the
                // sweet spot — low enough latency for media controls, big enough that the OS
                // scheduler doesn't drop frames.
                #[cfg(target_os = "android")]
                let target = 2048u32.clamp(*min, *max);
                #[cfg(not(target_os = "android"))]
                let target = 512u32.clamp(*min, *max);
                cpal::BufferSize::Fixed(target)
            }
            cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
        };
        stream_config
    }

    fn output_config_for_sample_rate(
        &self,
        desired_sample_rate: Option<u32>,
    ) -> Result<cpal::SupportedStreamConfig, String> {
        let default_config = self
            .device
            .default_output_config()
            .map_err(|e| PlayerInitError::DefaultOutputConfig(e).to_string())?;

        let Some(desired_sample_rate) = desired_sample_rate else {
            return Ok(default_config);
        };

        let default_channels = default_config.channels();
        let default_sample_format = default_config.sample_format();
        let supported_configs = match self.device.supported_output_configs() {
            Ok(configs) => configs,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to query supported output configs; using default output config"
                );
                return Ok(default_config);
            }
        };

        let mut best_config = None;
        let mut best_distance = u32::MAX;
        for supported_config in supported_configs {
            if supported_config.channels() != default_channels
                || supported_config.sample_format() != default_sample_format
            {
                continue;
            }

            let sample_rate = desired_sample_rate.clamp(
                supported_config.min_sample_rate(),
                supported_config.max_sample_rate(),
            );
            let Some(config) = supported_config.try_with_sample_rate(sample_rate) else {
                continue;
            };
            let distance = sample_rate.abs_diff(desired_sample_rate);
            if distance < best_distance {
                best_distance = distance;
                best_config = Some(config);
            }
        }

        Ok(best_config.unwrap_or(default_config))
    }
}

impl AudioSink for CpalSink {
    fn probe_config(&mut self, desired_sample_rate: Option<u32>) -> Result<SinkConfig, String> {
        let supported = self.output_config_for_sample_rate(desired_sample_rate)?;
        let stream_config = Self::preferred_stream_config(&supported);
        Ok(SinkConfig {
            channels: stream_config.channels as usize,
            sample_rate: stream_config.sample_rate,
        })
    }

    fn open(
        &mut self,
        desired_sample_rate: Option<u32>,
        make_cb: DataCallbackFactory,
    ) -> Result<SinkConfig, String> {
        // Re-acquire the default device: after a disconnect the cached handle is
        // dead, and opens are rare enough that a fresh lookup is free insurance.
        if let Some(device) = cpal::default_host().default_output_device() {
            self.device = device;
        }

        let supported = self.output_config_for_sample_rate(desired_sample_rate)?;
        let stream_config = Self::preferred_stream_config(&supported);
        let config = SinkConfig {
            channels: stream_config.channels as usize,
            sample_rate: stream_config.sample_rate,
        };

        let mut data_cb = make_cb(config);
        let on_event = self.on_event.clone();
        let stream = self
            .device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| data_cb(data),
                move |err| {
                    tracing::error!(error = %err, "cpal stream error");
                    on_event(SinkEvent::StreamError);
                },
                None,
            )
            .map_err(|e| PlayerInitError::BuildOutputStream(e).to_string())?;
        stream
            .play()
            .map_err(|e| PlayerInitError::StartOutputStream(e).to_string())?;

        self.stream = Some(stream);
        self.config = Some(config);
        Ok(config)
    }

    fn config(&self) -> Option<SinkConfig> {
        self.config
    }

    fn play(&mut self) -> Result<(), String> {
        if let Some(stream) = &self.stream {
            stream.play().map_err(|e| e.to_string())
        } else {
            Ok(())
        }
    }

    fn pause(&mut self) {
        if let Some(stream) = &self.stream {
            let _ = stream.pause();
        }
    }

    fn close(&mut self) {
        self.stream = None;
        self.config = None;
    }
}

impl Drop for CpalSink {
    fn drop(&mut self) {
        self.watcher_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Poll the OS default output device and report transitions. On backends where
/// the default is an alias that never renames (ALSA "default" under PipeWire,
/// which moves streams itself) this never fires; on WASAPI/CoreAudio the name
/// tracks the real device, which is exactly where Kopuz previously kept
/// playing on the old output until relaunch (issue #451).
fn spawn_default_device_watcher(
    on_event: std::sync::Arc<dyn Fn(SinkEvent) + Send + Sync + 'static>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let _ = std::thread::Builder::new()
        .name("kopuz-device-watch".to_string())
        .spawn(move || {
            let mut last: Option<cpal::DeviceId> = None;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let current = cpal::default_host()
                    .default_output_device()
                    .and_then(|device| device.id().ok());
                if let Some(current) = current {
                    if last.as_ref().is_some_and(|previous| *previous != current) {
                        tracing::info!(device = ?current, "default output device changed");
                        on_event(SinkEvent::DefaultDeviceChanged);
                    }
                    last = Some(current);
                }
                std::thread::sleep(DEVICE_WATCH_INTERVAL);
            }
        });
}
