use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub struct NowPlayingMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    pub artwork: Option<String>,
}

#[cfg(target_arch = "wasm32")]
fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

fn apply_channel_mode_to_frame(frame: &mut [f32], mode: ChannelMode) {
    if frame.len() < 2 {
        return;
    }

    let left = frame[0];
    let right = frame[1];

    match mode {
        ChannelMode::Stereo => {}
        ChannelMode::Mono => {
            let mixed = (left + right) * 0.5;
            frame[0] = mixed;
            frame[1] = mixed;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
        ChannelMode::LeftOnly => {
            frame[0] = left;
            frame[1] = 0.0;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
        ChannelMode::RightOnly => {
            frame[0] = 0.0;
            frame[1] = right;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
        ChannelMode::SwapLeftRight => {
            frame[0] = right;
            frame[1] = left;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
    }
}

fn apply_channel_mode_in_place(samples: &mut [f32], channels: usize, mode: ChannelMode) {
    if matches!(mode, ChannelMode::Stereo) || channels < 2 {
        return;
    }

    for frame in samples.chunks_exact_mut(channels.max(1)) {
        apply_channel_mode_to_frame(frame, mode);
    }
}

#[cfg(target_arch = "wasm32")]
const WEB_EQ_BAND_FREQUENCIES: [f32; 5] = [60.0, 250.0, 1_000.0, 4_000.0, 12_000.0];
#[cfg(target_arch = "wasm32")]
const WEB_EQ_BAND_Q: [f32; 5] = [0.9, 1.0, 1.0, 0.9, 0.8];

#[cfg(not(target_arch = "wasm32"))]
use crate::eq::Equalizer;
#[cfg(not(target_arch = "wasm32"))]
use crate::systemint;
use config::{ChannelMode, EqualizerSettings};
#[cfg(not(target_arch = "wasm32"))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(not(target_arch = "wasm32"))]
use rb::{RB, RbConsumer, RbInspector, RbProducer, SpscRb};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(not(target_arch = "wasm32"))]
use symphonia::core::audio::{AudioBufferRef, Signal};
#[cfg(not(target_arch = "wasm32"))]
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
#[cfg(not(target_arch = "wasm32"))]
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
#[cfg(not(target_arch = "wasm32"))]
use symphonia::core::io::MediaSourceStream;
#[cfg(not(target_arch = "wasm32"))]
use symphonia::core::meta::MetadataOptions;
#[cfg(not(target_arch = "wasm32"))]
use symphonia::core::probe::Hint;
#[cfg(not(target_arch = "wasm32"))]
use symphonia::core::units::Time;

#[cfg(not(target_arch = "wasm32"))]
struct PlaybackState {
    paused: bool,
    stopped: bool,
    volume: f32,
    seek_to: Option<Duration>,
    finished: bool,
}

#[cfg(not(target_arch = "wasm32"))]
struct CrossfadeState {
    total_frames: u64,
    progress_frames: u64,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Player {
    state: Arc<Mutex<PlaybackState>>,
    active_state_handle: Arc<Mutex<Arc<Mutex<PlaybackState>>>>,
    _device: cpal::Device,
    stream_config: cpal::StreamConfig,
    _stream: Option<cpal::Stream>,
    active_consumer: Arc<Mutex<Option<Arc<Mutex<rb::Consumer<f32>>>>>>,
    fading_consumer: Arc<Mutex<Option<Arc<Mutex<rb::Consumer<f32>>>>>>,
    crossfade_state: Arc<Mutex<Option<CrossfadeState>>>,
    ring_buf_consumer: Option<Arc<Mutex<rb::Consumer<f32>>>>,
    ring_buf: Option<SpscRb<f32>>,
    decoder_handle: Option<std::thread::JoinHandle<()>>,
    fading_session_state: Arc<Mutex<Option<Arc<Mutex<PlaybackState>>>>>,
    fading_ring_buf: Option<SpscRb<f32>>,
    fading_decoder_handle: Option<std::thread::JoinHandle<()>>,

    now_playing: Option<NowPlayingMeta>,
    position_micros: Arc<AtomicU64>,
    finish_callback: Option<Arc<dyn Fn() + Send + Sync + 'static>>,

    position_thread_handle: Option<std::thread::JoinHandle<()>>,
    position_thread_stop: Arc<AtomicBool>,
    equalizer: Arc<Mutex<Equalizer>>,
    channel_mode: Arc<Mutex<ChannelMode>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Player {
    fn preferred_stream_config(
        supported_config: &cpal::SupportedStreamConfig,
    ) -> cpal::StreamConfig {
        let mut stream_config = supported_config.config();
        stream_config.buffer_size = match supported_config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                let target = 512u32.clamp(*min, *max);
                cpal::BufferSize::Fixed(target)
            }
            cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
        };
        stream_config
    }

    fn play_output_stream(&self) {
        if let Some(stream) = &self._stream {
            let _ = stream.play();
        }
    }

    fn pause_output_stream(&self) {
        if let Some(stream) = &self._stream {
            let _ = stream.pause();
        }
    }

    pub fn new() -> Self {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("no output device available");

        let supported_config = device
            .default_output_config()
            .expect("no default output config");

        let stream_config = Self::preferred_stream_config(&supported_config);
        let state = Arc::new(Mutex::new(PlaybackState {
            paused: false,
            stopped: false,
            volume: 1.0,
            seek_to: None,
            finished: false,
        }));
        let active_state_handle = Arc::new(Mutex::new(state.clone()));
        let position_micros = Arc::new(AtomicU64::new(0));
        let equalizer = Arc::new(Mutex::new(Equalizer::new(
            stream_config.sample_rate,
            stream_config.channels as usize,
        )));
        let channel_mode = Arc::new(Mutex::new(ChannelMode::Stereo));
        let active_consumer = Arc::new(Mutex::new(None::<Arc<Mutex<rb::Consumer<f32>>>>));
        let fading_consumer = Arc::new(Mutex::new(None::<Arc<Mutex<rb::Consumer<f32>>>>));
        let crossfade_state = Arc::new(Mutex::new(None::<CrossfadeState>));
        let fading_session_state = Arc::new(Mutex::new(None::<Arc<Mutex<PlaybackState>>>));

        let channels = stream_config.channels as usize;
        let device_sample_rate = stream_config.sample_rate;
        let stream_active_state_handle = active_state_handle.clone();
        let stream_position = position_micros.clone();
        let stream_equalizer = equalizer.clone();
        let stream_channel_mode = channel_mode.clone();
        let stream_active_consumer = active_consumer.clone();
        let stream_fading_consumer = fading_consumer.clone();
        let stream_crossfade_state = crossfade_state.clone();
        let stream_fading_session_state = fading_session_state.clone();

        let stream = device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let active_state = stream_active_state_handle
                        .lock()
                        .map(|state| state.clone())
                        .unwrap_or_else(|e| e.into_inner().clone());
                    let st = active_state.lock().unwrap_or_else(|e| e.into_inner());
                    let volume = st.volume;
                    let paused = st.paused;
                    drop(st);

                    if paused {
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                        return;
                    }

                    let active_consumer = stream_active_consumer
                        .lock()
                        .ok()
                        .and_then(|consumer| consumer.clone());
                    let fading_consumer = stream_fading_consumer
                        .lock()
                        .ok()
                        .and_then(|consumer| consumer.clone());

                    let (active_read, read, fade_completed) = if fading_consumer.is_none() {
                        let active_read = if let Some(consumer) = active_consumer {
                            let cons = consumer.lock().unwrap_or_else(|e| e.into_inner());
                            cons.read(data).unwrap_or(0)
                        } else {
                            0
                        };
                        (active_read, active_read, false)
                    } else {
                        let mut active_samples = vec![0.0_f32; data.len()];
                        let mut fading_samples = vec![0.0_f32; data.len()];

                        let active_read = if let Some(consumer) = active_consumer {
                            let cons = consumer.lock().unwrap_or_else(|e| e.into_inner());
                            cons.read(&mut active_samples).unwrap_or(0)
                        } else {
                            0
                        };
                        let fading_read = if let Some(consumer) = fading_consumer {
                            let cons = consumer.lock().unwrap_or_else(|e| e.into_inner());
                            cons.read(&mut fading_samples).unwrap_or(0)
                        } else {
                            0
                        };

                        let read = active_read.max(fading_read);
                        let fade_completed = {
                            let mut fade = stream_crossfade_state
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            if let Some(fade) = fade.as_mut() {
                                let frames = read / channels.max(1);
                                if frames == 0 {
                                    false
                                } else {
                                    for frame_idx in 0..frames {
                                        let progress = ((fade.progress_frames + frame_idx as u64)
                                            .min(fade.total_frames))
                                            as f32
                                            / fade.total_frames.max(1) as f32;
                                        let fade_in_gain = progress.clamp(0.0, 1.0);
                                        let fade_out_gain = 1.0 - fade_in_gain;
                                        for ch in 0..channels {
                                            let index = frame_idx * channels + ch;
                                            let active =
                                                active_samples.get(index).copied().unwrap_or(0.0);
                                            let fading =
                                                fading_samples.get(index).copied().unwrap_or(0.0);
                                            data[index] =
                                                active * fade_in_gain + fading * fade_out_gain;
                                        }
                                    }
                                    fade.progress_frames =
                                        fade.progress_frames.saturating_add(frames as u64);
                                    if fade.progress_frames >= fade.total_frames {
                                        *fade = CrossfadeState {
                                            total_frames: fade.total_frames,
                                            progress_frames: fade.total_frames,
                                        };
                                        true
                                    } else {
                                        false
                                    }
                                }
                            } else {
                                if active_read > 0 {
                                    data[..active_read]
                                        .copy_from_slice(&active_samples[..active_read]);
                                }
                                false
                            }
                        };

                        (active_read, read, fade_completed)
                    };

                    if read > 0 {
                        if let Ok(mut eq) = stream_equalizer.lock() {
                            eq.process_in_place(&mut data[..read]);
                        }
                        let channel_mode = stream_channel_mode
                            .lock()
                            .map(|mode| *mode)
                            .unwrap_or(ChannelMode::Stereo);
                        apply_channel_mode_in_place(&mut data[..read], channels, channel_mode);
                    }

                    if fade_completed {
                        if let Ok(mut fading_consumer) = stream_fading_consumer.lock() {
                            *fading_consumer = None;
                        }
                        if let Ok(mut fade) = stream_crossfade_state.lock() {
                            *fade = None;
                        }
                        if let Ok(fading_state_guard) = stream_fading_session_state.lock() {
                            if let Some(fading_state) = fading_state_guard.as_ref() {
                                let mut st = fading_state.lock().unwrap_or_else(|e| e.into_inner());
                                st.stopped = true;
                                st.finished = true;
                            }
                        }
                    }

                    if channels > 0 && device_sample_rate > 0 {
                        stream_position.fetch_add(
                            (read as u64 * 1_000_000)
                                / (channels as u64 * device_sample_rate as u64),
                            Ordering::Relaxed,
                        );
                    }

                    for sample in data[..read].iter_mut() {
                        *sample *= volume;
                    }
                    for sample in data[read..].iter_mut() {
                        *sample = 0.0;
                    }
                },
                move |err| {
                    eprintln!("cpal stream error: {}", err);
                },
                None,
            )
            .unwrap_or_else(|e| panic!("failed to build output stream: {e}"));

        stream
            .play()
            .unwrap_or_else(|e| panic!("failed to start output stream: {e}"));

        Self {
            state,
            active_state_handle,
            _device: device,
            stream_config,
            _stream: Some(stream),
            active_consumer,
            fading_consumer,
            crossfade_state,
            ring_buf_consumer: None,
            ring_buf: None,
            decoder_handle: None,
            fading_session_state,
            fading_ring_buf: None,
            fading_decoder_handle: None,
            now_playing: None,
            position_micros,
            finish_callback: None,
            position_thread_handle: None,
            position_thread_stop: Arc::default(),
            equalizer,
            channel_mode,
        }
    }

    /// Register a callback that fires whenever a track finishes playing naturally
    /// (e.g. EOF or decode error) but NOT when playback is explicitly stopped.
    /// Use this to trigger auto-skip from a background thread without depending
    /// on the Dioxus event loop being active.
    pub fn set_finish_callback(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.finish_callback = Some(Arc::new(f));
    }

    pub fn play(
        &mut self,
        source: Box<dyn symphonia::core::io::MediaSource>,
        meta: NowPlayingMeta,
        hint: Hint,
    ) -> Result<(), String> {
        self.cleanup_finished_fading_session();
        self.stop_playback_session();

        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.paused = false;
            st.stopped = false;
            st.seek_to = None;
            st.finished = false;
        }
        if let Ok(mut active_state_handle) = self.active_state_handle.lock() {
            *active_state_handle = self.state.clone();
        }
        self.position_micros.store(0, Ordering::SeqCst);

        let channels = self.stream_config.channels as usize;
        let device_sample_rate = self.stream_config.sample_rate;

        let ring_buf_size = device_sample_rate as usize * channels * 2;
        let ring_buf = SpscRb::new(ring_buf_size);
        let (producer, consumer) = (ring_buf.producer(), ring_buf.consumer());
        let consumer = Arc::new(Mutex::new(consumer));
        self.ring_buf_consumer = Some(consumer.clone());
        self.ring_buf = Some(ring_buf);
        if let Ok(mut active_consumer) = self.active_consumer.lock() {
            *active_consumer = Some(consumer.clone());
        }

        self.start_position_thread();

        let decoder_state = self.state.clone();
        let decoder_channels = channels;
        let decoder_sample_rate = device_sample_rate;
        let finish_cb = self.finish_callback.clone();

        if let Ok(mut eq) = self.equalizer.lock() {
            eq.update_output_format(device_sample_rate, channels);
        }

        let handle = std::thread::spawn(move || {
            Self::decoder_thread(
                source,
                hint,
                producer,
                decoder_state,
                decoder_channels,
                decoder_sample_rate,
                finish_cb,
            );
        });
        self.decoder_handle = Some(handle);

        self.now_playing = Some(meta);
        self.play_output_stream();

        self.update_now_playing_system();

        Ok(())
    }

    pub fn crossfade_to(
        &mut self,
        source: Box<dyn symphonia::core::io::MediaSource>,
        meta: NowPlayingMeta,
        hint: Hint,
        duration: Duration,
    ) -> Result<(), String> {
        self.cleanup_finished_fading_session();

        if duration.is_zero() || self.ring_buf_consumer.is_none() || self.decoder_handle.is_none() {
            return self.play(source, meta, hint);
        }

        self.stop_fading_session();

        let previous_volume = { self.state.lock().unwrap_or_else(|e| e.into_inner()).volume };
        let old_state = self.state.clone();
        let old_consumer = self.ring_buf_consumer.take();
        let old_ring_buf = self.ring_buf.take();
        let old_decoder_handle = self.decoder_handle.take();

        if let Some(old_consumer) = old_consumer {
            if let Ok(mut fading_consumer) = self.fading_consumer.lock() {
                *fading_consumer = Some(old_consumer);
            }
        }
        if let Ok(mut fading_state) = self.fading_session_state.lock() {
            *fading_state = Some(old_state);
        }
        self.fading_ring_buf = old_ring_buf;
        self.fading_decoder_handle = old_decoder_handle;

        let new_state = Arc::new(Mutex::new(PlaybackState {
            paused: false,
            stopped: false,
            volume: previous_volume,
            seek_to: None,
            finished: false,
        }));
        self.state = new_state.clone();
        if let Ok(mut active_state_handle) = self.active_state_handle.lock() {
            *active_state_handle = new_state.clone();
        }
        self.position_micros.store(0, Ordering::SeqCst);

        let channels = self.stream_config.channels as usize;
        let device_sample_rate = self.stream_config.sample_rate;
        let ring_buf_size = device_sample_rate as usize * channels * 2;
        let ring_buf = SpscRb::new(ring_buf_size);
        let (producer, consumer) = (ring_buf.producer(), ring_buf.consumer());
        let consumer = Arc::new(Mutex::new(consumer));
        self.ring_buf_consumer = Some(consumer.clone());
        self.ring_buf = Some(ring_buf);
        if let Ok(mut active_consumer) = self.active_consumer.lock() {
            *active_consumer = Some(consumer);
        }
        if let Ok(mut fade) = self.crossfade_state.lock() {
            let total_frames = (duration.as_secs_f64() * device_sample_rate as f64).round() as u64;
            *fade = Some(CrossfadeState {
                total_frames: total_frames.max(1),
                progress_frames: 0,
            });
        }

        self.start_position_thread();

        let finish_cb = self.finish_callback.clone();
        if let Ok(mut eq) = self.equalizer.lock() {
            eq.update_output_format(device_sample_rate, channels);
        }

        let handle = std::thread::spawn(move || {
            Self::decoder_thread(
                source,
                hint,
                producer,
                new_state,
                channels,
                device_sample_rate,
                finish_cb,
            );
        });
        self.decoder_handle = Some(handle);
        self.now_playing = Some(meta);
        self.play_output_stream();
        self.update_now_playing_system();

        Ok(())
    }

    fn start_position_thread(&mut self) {
        #[cfg(target_os = "linux")]
        {
            self.position_thread_stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.position_thread_handle.take() {
                let _ = handle.join();
            }

            let stop = Arc::new(AtomicBool::new(false));
            self.position_thread_stop = stop.clone();
            let pos = self.position_micros.clone();
            let state = self.state.clone();

            let handle = std::thread::spawn(move || {
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let st = state.lock().unwrap_or_else(|e| e.into_inner());
                    if st.finished {
                        break;
                    }
                    let paused = st.paused;
                    drop(st);
                    if !paused {
                        let micros = pos.load(std::sync::atomic::Ordering::Relaxed);
                        systemint::update_position(micros as f64 / 1_000_000.0);
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            });
            self.position_thread_handle = Some(handle);
        }
    }

    fn cleanup_finished_fading_session(&mut self) {
        let should_cleanup = self
            .fading_decoder_handle
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished);

        let fade_active = self
            .crossfade_state
            .lock()
            .map(|fade| fade.is_some())
            .unwrap_or(false);

        if should_cleanup && !fade_active {
            if let Some(handle) = self.fading_decoder_handle.take() {
                let _ = handle.join();
            }
            self.fading_ring_buf = None;
            if let Ok(mut fading_state) = self.fading_session_state.lock() {
                *fading_state = None;
            }
        }
    }

    fn spawn_cleanup(
        decoder_handle: Option<std::thread::JoinHandle<()>>,
        ring_buf: Option<SpscRb<f32>>,
        position_handle: Option<std::thread::JoinHandle<()>>,
    ) {
        if decoder_handle.is_none() && ring_buf.is_none() && position_handle.is_none() {
            return;
        }

        std::thread::spawn(move || {
            if let Some(handle) = decoder_handle {
                let _ = handle.join();
            }
            drop(ring_buf);
            if let Some(handle) = position_handle {
                let _ = handle.join();
            }
        });
    }

    fn stop_fading_session(&mut self) {
        if let Ok(fading_state) = self.fading_session_state.lock() {
            if let Some(state) = fading_state.as_ref() {
                let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                st.stopped = true;
                st.finished = true;
            }
        }
        if let Ok(mut fade) = self.crossfade_state.lock() {
            *fade = None;
        }
        if let Ok(mut fading_consumer) = self.fading_consumer.lock() {
            *fading_consumer = None;
        }
        let fading_decoder_handle = self.fading_decoder_handle.take();
        let fading_ring_buf = self.fading_ring_buf.take();
        if let Ok(mut fading_state) = self.fading_session_state.lock() {
            *fading_state = None;
        }
        Self::spawn_cleanup(fading_decoder_handle, fading_ring_buf, None);
    }

    fn decoder_thread(
        source: Box<dyn symphonia::core::io::MediaSource>,
        hint: Hint,
        producer: rb::Producer<f32>,
        state: Arc<Mutex<PlaybackState>>,
        target_channels: usize,
        target_sample_rate: u32,
        finish_cb: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    ) {
        let mss = MediaSourceStream::new(source, Default::default());

        let finish_natural = |state: &Arc<Mutex<PlaybackState>>| {
            state.lock().unwrap_or_else(|e| e.into_inner()).finished = true;
            if let Some(cb) = &finish_cb {
                cb();
            }
        };

        let probed = match symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("symphonia probe error: {}", e);
                finish_natural(&state);
                return;
            }
        };

        let mut format = probed.format;

        let track = match format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        {
            Some(t) => t,
            None => {
                eprintln!("no supported audio tracks found");
                finish_natural(&state);
                return;
            }
        };

        let track_id = track.id;
        let source_sample_rate = track.codec_params.sample_rate.unwrap_or(target_sample_rate);
        let source_channels = track
            .codec_params
            .channels
            .map(|c| c.count())
            .unwrap_or(target_channels);

        let mut decoder: Box<dyn symphonia::core::codecs::Decoder> =
            match symphonia::default::get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())
            {
                Ok(d) => d,
                Err(_) => match symphonia_adapter_libopus::OpusDecoder::try_new(
                    &track.codec_params,
                    &DecoderOptions::default(),
                ) {
                    Ok(d) => Box::new(d),
                    Err(e) => {
                        eprintln!("symphonia codec error: {}", e);
                        finish_natural(&state);
                        return;
                    }
                },
            };

        loop {
            {
                let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                if st.stopped {
                    st.finished = true;
                    return;
                }

                if let Some(seek_time) = st.seek_to.take() {
                    let time = Time::new(seek_time.as_secs(), seek_time.as_secs_f64().fract());
                    let seek_to = SeekTo::Time {
                        time,
                        track_id: Some(track_id),
                    };
                    drop(st);
                    let seek_result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            format.seek(SeekMode::Coarse, seek_to)
                        }));
                    match seek_result {
                        Ok(Ok(_)) => decoder.reset(),
                        Ok(Err(e)) => eprintln!("seek error: {}", e),
                        Err(_) => {
                            eprintln!(
                                "seek panicked inside symphonia demuxer; continuing playback"
                            );
                            decoder.reset();
                        }
                    }
                    continue;
                }

                while st.paused && !st.stopped {
                    drop(st);
                    std::thread::sleep(Duration::from_millis(10));
                    st = state.lock().unwrap_or_else(|e| e.into_inner());
                }
                if st.stopped {
                    st.finished = true;
                    return;
                }
            }

            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    // Natural end of track — fire the finish callback.
                    finish_natural(&state);
                    return;
                }
                Err(symphonia::core::errors::Error::ResetRequired) => {
                    decoder.reset();
                    continue;
                }
                Err(e) => {
                    eprintln!("format error: {}", e);
                    finish_natural(&state);
                    return;
                }
            };

            if packet.track_id() != track_id {
                continue;
            }

            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(symphonia::core::errors::Error::DecodeError(e)) => {
                    eprintln!("decode error: {}", e);
                    continue;
                }
                Err(e) => {
                    eprintln!("fatal decode error: {}", e);
                    finish_natural(&state);
                    return;
                }
            };

            let samples = Self::audio_buf_to_f32_interleaved(
                &decoded,
                source_channels,
                target_channels,
                source_sample_rate,
                target_sample_rate,
            );

            let mut offset = 0;
            while offset < samples.len() {
                {
                    let st = state.lock().unwrap_or_else(|e| e.into_inner());
                    if st.stopped {
                        return;
                    }
                }
                match producer.write(&samples[offset..]) {
                    Ok(written) => offset += written,
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        }
    }

    fn audio_buf_to_f32_interleaved(
        buf: &AudioBufferRef,
        source_channels: usize,
        target_channels: usize,
        source_sample_rate: u32,
        target_sample_rate: u32,
    ) -> Vec<f32> {
        let frames = buf.frames();
        let src_chans = source_channels.max(1);

        let mut interleaved = Vec::with_capacity(frames * src_chans);

        match buf {
            AudioBufferRef::F32(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            interleaved.push(b.chan(ch)[frame]);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::S16(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            interleaved.push(b.chan(ch)[frame] as f32 / 32768.0);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::S32(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            interleaved.push(b.chan(ch)[frame] as f32 / 2147483648.0);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::U8(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            interleaved.push((b.chan(ch)[frame] as f32 - 128.0) / 128.0);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::F64(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            interleaved.push(b.chan(ch)[frame] as f32);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::S24(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            let val = b.chan(ch)[frame].0;
                            interleaved.push(val as f32 / 8388608.0);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::U16(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            interleaved.push((b.chan(ch)[frame] as f32 - 32768.0) / 32768.0);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::U24(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            let val: u32 = b.chan(ch)[frame].0.into();
                            interleaved.push((val as f32 - 8388608.0) / 8388608.0);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::U32(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            interleaved.push(
                                (b.chan(ch)[frame] as f64 - 2147483648.0) as f32 / 2147483648.0,
                            );
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
            AudioBufferRef::S8(b) => {
                for frame in 0..frames {
                    for ch in 0..src_chans {
                        if ch < b.spec().channels.count() {
                            interleaved.push(b.chan(ch)[frame] as f32 / 128.0);
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }
        }

        let interleaved = if src_chans != target_channels {
            Self::convert_channels(&interleaved, src_chans, target_channels)
        } else {
            interleaved
        };

        if source_sample_rate != target_sample_rate {
            Self::resample(
                &interleaved,
                target_channels,
                source_sample_rate,
                target_sample_rate,
            )
        } else {
            interleaved
        }
    }

    fn convert_channels(samples: &[f32], src_channels: usize, dst_channels: usize) -> Vec<f32> {
        let frames = samples.len() / src_channels;
        let mut out = Vec::with_capacity(frames * dst_channels);

        for frame in 0..frames {
            let src_offset = frame * src_channels;
            for ch in 0..dst_channels {
                if ch < src_channels {
                    out.push(samples[src_offset + ch]);
                } else if src_channels == 1 {
                    // Mono to multi: duplicate
                    out.push(samples[src_offset]);
                } else {
                    out.push(0.0);
                }
            }
        }
        out
    }

    fn resample(samples: &[f32], channels: usize, src_rate: u32, dst_rate: u32) -> Vec<f32> {
        if channels == 0 || src_rate == 0 || dst_rate == 0 {
            return samples.to_vec();
        }
        let src_frames = samples.len() / channels;
        let ratio = dst_rate as f64 / src_rate as f64;
        if ratio.is_nan() || ratio.is_infinite() {
            return samples.to_vec();
        }
        let dst_frames = (src_frames as f64 * ratio).ceil() as usize;
        let mut out = Vec::with_capacity(dst_frames * channels);

        for i in 0..dst_frames {
            let src_pos = i as f64 / ratio;
            let src_idx = src_pos.floor() as usize;
            let frac = src_pos - src_idx as f64;

            for ch in 0..channels {
                let s0 = if src_idx < src_frames {
                    samples[src_idx * channels + ch]
                } else {
                    0.0
                };
                let s1 = if src_idx + 1 < src_frames {
                    samples[(src_idx + 1) * channels + ch]
                } else {
                    s0
                };
                out.push(s0 + (s1 - s0) * frac as f32);
            }
        }
        out
    }

    pub fn pause(&mut self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !st.paused {
            st.paused = true;
            drop(st);
            self.pause_output_stream();

            self.update_now_playing_system();
        }
    }

    pub fn play_resume(&mut self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.paused {
            st.paused = false;
            drop(st);
            self.play_output_stream();

            self.update_now_playing_system();
        }
    }

    pub fn seek(&mut self, time: Duration) {
        const END_GUARD: Duration = Duration::from_millis(2000);
        let time = if let Some(meta) = &self.now_playing {
            if meta.duration > END_GUARD {
                let max = meta.duration - END_GUARD;
                if time > max { max } else { time }
            } else {
                Duration::ZERO
            }
        } else {
            time
        };

        self.stop_fading_session();
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.seek_to = Some(time);
            st.finished = false;
            self.position_micros
                .store(time.as_micros() as u64, Ordering::Relaxed);

            if let Some(cons) = &self.ring_buf_consumer {
                if let Ok(cons) = cons.lock() {
                    let mut dummy = [0.0f32; 2048];
                    while cons.read(&mut dummy).unwrap_or(0) > 0 {}
                }
            }
        }

        self.update_now_playing_system();
    }

    pub fn is_empty(&self) -> bool {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.finished
    }

    pub fn is_playback_complete(&self) -> bool {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !st.finished {
            return false;
        }
        if let Some(rb) = &self.ring_buf {
            return rb.is_empty();
        }
        true
    }

    pub fn is_paused(&self) -> bool {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.paused
    }

    pub fn can_resume(&self) -> bool {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        !st.stopped && !st.finished && self._stream.is_some()
    }

    pub fn stop(&mut self) {
        self.stop_internal();
        self.now_playing = None;
    }

    pub fn stop_for_transition(&mut self) {
        self.stop_playback_session();
        self.position_micros.store(0, Ordering::SeqCst);
    }

    fn stop_internal(&mut self) {
        self.pause_output_stream();
        self.stop_playback_session();
        self.position_micros.store(0, Ordering::SeqCst);
    }

    fn stop_playback_session(&mut self) {
        self.position_thread_stop.store(true, Ordering::Relaxed);
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.stopped = true;
            st.paused = false;
            st.seek_to = None;
            st.finished = true;
        }

        if let Ok(mut active_consumer) = self.active_consumer.lock() {
            *active_consumer = None;
        }
        self.ring_buf_consumer = None;
        let ring_buf = self.ring_buf.take();
        let decoder_handle = self.decoder_handle.take();
        let position_handle = self.position_thread_handle.take();

        self.stop_fading_session();
        Self::spawn_cleanup(decoder_handle, ring_buf, position_handle);
    }

    pub fn set_volume(&mut self, volume: f32) {
        let gain = volume.clamp(0.0, 1.0).powi(3);
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.volume = gain;
    }

    pub fn set_channel_mode(&mut self, mode: ChannelMode) {
        if let Ok(mut channel_mode) = self.channel_mode.lock() {
            *channel_mode = mode;
        }
    }

    pub fn set_equalizer(&mut self, settings: EqualizerSettings) {
        if let Ok(mut eq) = self.equalizer.lock() {
            eq.set_settings(settings);
        }
    }

    pub fn update_metadata(&mut self, meta: NowPlayingMeta) {
        self.now_playing = Some(meta);
        self.update_now_playing_system();
    }

    fn update_now_playing_system(&self) {
        #[cfg(target_os = "macos")]
        if let Some(meta) = &self.now_playing {
            systemint::update_now_playing(
                &meta.title,
                &meta.artist,
                &meta.album,
                meta.duration.as_secs_f64(),
                self.get_position().as_secs_f64(),
                !self.is_paused(),
                meta.artwork.as_deref(),
            );
        }

        #[cfg(target_os = "linux")]
        if let Some(meta) = &self.now_playing {
            systemint::update_now_playing(
                &meta.title,
                &meta.artist,
                &meta.album,
                meta.duration.as_secs_f64(),
                self.get_position().as_secs_f64(),
                !self.is_paused(),
                meta.artwork.as_deref(),
            );
        }

        #[cfg(target_os = "windows")]
        if let Some(meta) = &self.now_playing {
            systemint::update_now_playing(
                &meta.title,
                &meta.artist,
                &meta.album,
                meta.duration.as_secs_f64(),
                self.get_position().as_secs_f64(),
                !self.is_paused(),
                meta.artwork.as_deref(),
            );
        }
    }

    pub fn get_position(&self) -> Duration {
        let raw = Duration::from_micros(self.position_micros.load(Ordering::Relaxed));

        if let Some(meta) = &self.now_playing {
            if meta.duration > Duration::ZERO && raw > meta.duration {
                return meta.duration;
            }
        }
        raw
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Player {
    fn drop(&mut self) {
        self.stop_playback_session();
        self._stream = None;
    }
}

// ─────────────────────────────────────────────
// Web (WASM) implementation — uses HtmlAudioElement
// ─────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
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
