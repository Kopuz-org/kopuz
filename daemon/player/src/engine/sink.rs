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
    /// Microseconds the callback's samples sit ahead of the speakers. Zero from
    /// a backend that reports no playback timestamp.
    fn output_latency_micros(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0))
    }
    fn play(&mut self) -> Result<(), String>;
    fn pause(&mut self);
    fn close(&mut self);
    /// The device the next `open` lands on, as a cpal device id; `None` is the system default.
    fn set_device(&mut self, _device: Option<String>) {}
    /// Whether the next `open` should hold its device alone.
    fn set_exclusive(&mut self, _exclusive: bool) {}
    /// The next `open` holds a hardware device alone, so it must run at the source's rate.
    fn exclusive(&self) -> bool {
        false
    }
}

/// Discards audio: opens successfully, never pulls samples. Lets the full
/// actor/decoder state machine run in headless integration tests and tools
/// without an output device.
#[derive(Default)]
pub struct NullSink {
    config: Option<SinkConfig>,
}

impl NullSink {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AudioSink for NullSink {
    fn probe_config(&mut self, desired_sample_rate: Option<u32>) -> Result<SinkConfig, String> {
        Ok(SinkConfig {
            channels: 2,
            sample_rate: desired_sample_rate.unwrap_or(44_100),
        })
    }

    fn open(
        &mut self,
        desired_sample_rate: Option<u32>,
        make_cb: DataCallbackFactory,
    ) -> Result<SinkConfig, String> {
        let config = SinkConfig {
            channels: 2,
            sample_rate: desired_sample_rate.unwrap_or(44_100),
        };
        drop(make_cb(config));
        self.config = Some(config);
        Ok(config)
    }

    fn config(&self) -> Option<SinkConfig> {
        self.config
    }

    fn play(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn pause(&mut self) {}

    fn close(&mut self) {
        self.config = None;
    }
}

/// Out-of-band notifications from the audio backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkEvent {
    /// The device is gone (unplugged, format lost) — recovery lands on
    /// whatever device is the default now.
    DeviceLost,
    /// The stream stalled on a still-present device (an xrun the backend
    /// couldn't recover in place). Rebuilding lands on the same device.
    StreamStalled,
    /// The OS default output device changed while our stream kept playing on
    /// the old one.
    DefaultDeviceChanged,
}

/// How often the watcher compares the OS default output device. Enumeration is
/// cheap on every backend, and a couple of seconds of migration latency is
/// imperceptible next to the device switch itself.
const DEVICE_WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Interleaved samples the conversion scratch holds before the first callback, so it rarely grows on the audio thread.
const SCRATCH_SAMPLES: usize = 16_384;

/// What a device says it can play, read once per open so a probe never reopens a device the stream holds.
#[derive(Clone)]
struct DeviceCaps {
    default: cpal::SupportedStreamConfig,
    ranges: Vec<cpal::SupportedStreamConfigRange>,
}

impl DeviceCaps {
    fn query(device: &cpal::Device) -> Result<Self, cpal::Error> {
        let default = device.default_output_config()?;
        let ranges = match device.supported_output_configs() {
            Ok(ranges) => ranges.collect(),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to query supported output configs; using default output config"
                );
                Vec::new()
            }
        };
        Ok(Self { default, ranges })
    }
}

pub struct CpalSink {
    device: cpal::Device,
    /// The setting's device id; `None` follows the system default.
    preferred: Option<String>,
    /// Hold the preferred device's raw hardware path alone instead of sharing it.
    exclusive: bool,
    caps: Option<DeviceCaps>,
    stream: Option<cpal::Stream>,
    config: Option<SinkConfig>,
    on_event: std::sync::Arc<dyn Fn(SinkEvent) + Send + Sync + 'static>,
    watcher_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Off while the chosen device is open, so a default change leaves it alone.
    follow_default: std::sync::Arc<std::sync::atomic::AtomicBool>,
    latency_micros: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl CpalSink {
    pub fn try_new(
        preferred: Option<String>,
        exclusive: bool,
        on_event: impl Fn(SinkEvent) + Send + Sync + 'static,
    ) -> Result<Self, PlayerInitError> {
        let (device, pick) = crate::output::pick_device(preferred.as_deref(), exclusive)
            .ok_or(PlayerInitError::NoOutputDevice)?;

        let on_event = std::sync::Arc::new(on_event);
        let watcher_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let follow_default =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(pick.follows_default()));
        spawn_default_device_watcher(
            on_event.clone(),
            watcher_stop.clone(),
            follow_default.clone(),
        );

        Ok(Self {
            device,
            preferred,
            exclusive,
            caps: None,
            stream: None,
            config: None,
            on_event,
            watcher_stop,
            follow_default,
            latency_micros: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    fn preferred_stream_config(
        supported_config: &cpal::SupportedStreamConfig,
    ) -> cpal::StreamConfig {
        let mut stream_config = supported_config.config();
        stream_config.buffer_size = match supported_config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
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

    fn caps(&mut self) -> Result<DeviceCaps, String> {
        if let Some(caps) = &self.caps {
            return Ok(caps.clone());
        }
        let caps = DeviceCaps::query(&self.device)
            .map_err(|e| PlayerInitError::OutputStream(e).to_string())?;
        self.caps = Some(caps.clone());
        Ok(caps)
    }

    /// Re-resolve the device and read its caps afresh, letting go of our own stream first if it holds the device.
    fn reacquire(&mut self) -> Result<DeviceCaps, String> {
        let (device, pick) = crate::output::pick_device(self.preferred.as_deref(), self.exclusive)
            .ok_or_else(|| PlayerInitError::NoOutputDevice.to_string())?;
        self.device = device;
        self.caps = None;
        self.follow_default
            .store(pick.follows_default(), std::sync::atomic::Ordering::Relaxed);
        let caps = match DeviceCaps::query(&self.device) {
            Ok(caps) => caps,
            // A raw hw: device admits one opener, and that may be the stream being replaced.
            Err(e) if e.kind() == cpal::ErrorKind::DeviceBusy && self.stream.is_some() => {
                self.stream = None;
                self.config = None;
                DeviceCaps::query(&self.device)
                    .map_err(|e| PlayerInitError::OutputStream(e).to_string())?
            }
            Err(e) => return Err(PlayerInitError::OutputStream(e).to_string()),
        };
        self.caps = Some(caps.clone());
        Ok(caps)
    }
}

/// Whether the data callback can write this format from the engine's f32.
fn convertible(format: cpal::SampleFormat) -> bool {
    use cpal::SampleFormat as F;
    matches!(
        format,
        F::I8
            | F::I16
            | F::I24
            | F::I32
            | F::I64
            | F::U8
            | F::U16
            | F::U24
            | F::U32
            | F::U64
            | F::F32
            | F::F64
    )
}

/// The config to open: the default's channels, f32 when the device takes it
/// (else its own format, converted), at the rate nearest `desired_sample_rate`.
fn choose_config(
    caps: &DeviceCaps,
    desired_sample_rate: Option<u32>,
) -> Result<cpal::SupportedStreamConfig, String> {
    let default = caps.default;
    let channels = default.channels();
    let format = if caps.ranges.iter().any(|range| {
        range.channels() == channels && range.sample_format() == cpal::SampleFormat::F32
    }) {
        cpal::SampleFormat::F32
    } else {
        default.sample_format()
    };
    if !convertible(format) {
        return Err(format!("output device only takes {format} samples"));
    }
    if desired_sample_rate.is_none() && format == default.sample_format() {
        return Ok(default);
    }
    let target = desired_sample_rate.unwrap_or(default.sample_rate());

    let mut best_config = None;
    let mut best_distance = u32::MAX;
    for range in &caps.ranges {
        if range.channels() != channels || range.sample_format() != format {
            continue;
        }
        let sample_rate = target.clamp(range.min_sample_rate(), range.max_sample_rate());
        let Some(config) = range.try_with_sample_rate(sample_rate) else {
            continue;
        };
        let distance = sample_rate.abs_diff(target);
        if distance < best_distance {
            best_distance = distance;
            best_config = Some(config);
        }
    }
    Ok(best_config.unwrap_or(default))
}

/// Write the engine's f32 samples into a device buffer of another format.
fn write_converted(data: &mut cpal::Data, samples: &[f32]) {
    fn fill<T: cpal::SizedSample + cpal::FromSample<f32>>(data: &mut cpal::Data, samples: &[f32]) {
        if let Some(out) = data.as_slice_mut::<T>() {
            for (out, sample) in out.iter_mut().zip(samples) {
                *out = T::from_sample(*sample);
            }
        }
    }
    use cpal::SampleFormat as F;
    match data.sample_format() {
        F::I8 => fill::<i8>(data, samples),
        F::I16 => fill::<i16>(data, samples),
        F::I24 => fill::<cpal::I24>(data, samples),
        F::I32 => fill::<i32>(data, samples),
        F::I64 => fill::<i64>(data, samples),
        F::U8 => fill::<u8>(data, samples),
        F::U16 => fill::<u16>(data, samples),
        F::U24 => fill::<cpal::U24>(data, samples),
        F::U32 => fill::<u32>(data, samples),
        F::U64 => fill::<u64>(data, samples),
        F::F32 => fill::<f32>(data, samples),
        F::F64 => fill::<f64>(data, samples),
        _ => {}
    }
}

impl AudioSink for CpalSink {
    fn output_latency_micros(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        self.latency_micros.clone()
    }

    fn probe_config(&mut self, desired_sample_rate: Option<u32>) -> Result<SinkConfig, String> {
        let caps = self.caps()?;
        let supported = choose_config(&caps, desired_sample_rate)?;
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
        let caps = self.reacquire()?;
        let supported = choose_config(&caps, desired_sample_rate)?;
        let sample_format = supported.sample_format();
        let stream_config = Self::preferred_stream_config(&supported);
        let config = SinkConfig {
            channels: stream_config.channels as usize,
            sample_rate: stream_config.sample_rate,
        };

        let mut data_cb = make_cb(config);
        let mut scratch = vec![0.0f32; SCRATCH_SAMPLES];
        let on_event = self.on_event.clone();
        let latency_micros = self.latency_micros.clone();

        self.latency_micros
            .store(0, std::sync::atomic::Ordering::Relaxed);
        let stream = self
            .device
            .build_output_stream_raw(
                stream_config,
                sample_format,
                move |data: &mut cpal::Data, info: &cpal::OutputCallbackInfo| {
                    let stamps = info.timestamp();
                    if let Some(ahead) = stamps.playback.checked_duration_since(stamps.callback) {
                        let sample = ahead.as_micros() as u64;
                        let prior = latency_micros.load(std::sync::atomic::Ordering::Relaxed);
                        let next = if prior == 0 {
                            sample
                        } else {
                            (prior * 7 + sample) / 8
                        };
                        latency_micros.store(next, std::sync::atomic::Ordering::Relaxed);
                    }
                    if let Some(out) = data.as_slice_mut::<f32>() {
                        data_cb(out);
                        return;
                    }
                    let len = data.len();
                    if scratch.len() < len {
                        scratch.resize(len, 0.0);
                    }
                    let samples = &mut scratch[..len];
                    data_cb(samples);
                    write_converted(data, samples);
                },
                move |err: cpal::Error| {
                    let event = match err.kind() {
                        cpal::ErrorKind::DeviceNotAvailable => SinkEvent::DeviceLost,

                        cpal::ErrorKind::Xrun => {
                            tracing::warn!("audio buffer underrun (recovered in place)");
                            return;
                        }

                        cpal::ErrorKind::DeviceChanged => {
                            tracing::info!("audio stream rerouted by the backend");
                            return;
                        }

                        _ => SinkEvent::StreamStalled,
                    };
                    tracing::error!(error = %err, "cpal stream error");
                    on_event(event);
                },
                None,
            )
            .map_err(|e| PlayerInitError::OutputStream(e).to_string())?;
        stream
            .play()
            .map_err(|e| PlayerInitError::OutputStream(e).to_string())?;

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

    fn set_device(&mut self, device: Option<String>) {
        self.preferred = device;
    }

    fn set_exclusive(&mut self, exclusive: bool) {
        self.exclusive = exclusive;
    }

    fn exclusive(&self) -> bool {
        self.exclusive
            && self
                .preferred
                .as_deref()
                .and_then(crate::output::exclusive_target)
                .is_some()
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
    follow_default: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
                    if last.as_ref().is_some_and(|previous| *previous != current)
                        && follow_default.load(std::sync::atomic::Ordering::Relaxed)
                    {
                        tracing::info!(device = ?current, "default output device changed");
                        on_event(SinkEvent::DefaultDeviceChanged);
                    }
                    last = Some(current);
                }
                std::thread::sleep(DEVICE_WATCH_INTERVAL);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(
        channels: u16,
        min: u32,
        max: u32,
        format: cpal::SampleFormat,
    ) -> cpal::SupportedStreamConfigRange {
        cpal::SupportedStreamConfigRange::new(
            channels,
            min,
            max,
            cpal::SupportedBufferSize::Unknown,
            format,
        )
    }

    fn caps(
        default_format: cpal::SampleFormat,
        ranges: Vec<cpal::SupportedStreamConfigRange>,
    ) -> DeviceCaps {
        DeviceCaps {
            default: cpal::SupportedStreamConfig::new(
                2,
                48_000,
                cpal::SupportedBufferSize::Unknown,
                default_format,
            ),
            ranges,
        }
    }

    #[test]
    fn a_float_device_keeps_its_default_unless_a_rate_is_asked() {
        let caps = caps(
            cpal::SampleFormat::F32,
            vec![range(2, 8_000, 192_000, cpal::SampleFormat::F32)],
        );
        assert_eq!(choose_config(&caps, None).unwrap(), caps.default);
        let at_source = choose_config(&caps, Some(44_100)).unwrap();
        assert_eq!(at_source.sample_rate(), 44_100);
        assert_eq!(at_source.sample_format(), cpal::SampleFormat::F32);
    }

    #[test]
    fn a_hardware_device_without_float_opens_in_its_own_format() {
        let caps = caps(
            cpal::SampleFormat::I32,
            vec![
                range(2, 44_100, 44_100, cpal::SampleFormat::I16),
                range(2, 48_000, 48_000, cpal::SampleFormat::I16),
                range(2, 44_100, 44_100, cpal::SampleFormat::I32),
                range(2, 48_000, 48_000, cpal::SampleFormat::I32),
                range(2, 96_000, 96_000, cpal::SampleFormat::I32),
            ],
        );
        let chosen = choose_config(&caps, Some(96_000)).unwrap();
        assert_eq!(chosen.sample_format(), cpal::SampleFormat::I32);
        assert_eq!(chosen.sample_rate(), 96_000);
        assert_eq!(
            choose_config(&caps, Some(88_200)).unwrap().sample_rate(),
            96_000
        );
    }

    #[test]
    fn a_dsd_only_device_is_refused() {
        let caps = caps(
            cpal::SampleFormat::DsdU32,
            vec![range(2, 44_100, 44_100, cpal::SampleFormat::DsdU32)],
        );
        assert!(choose_config(&caps, None).is_err());
    }
}
