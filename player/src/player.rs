use crate::systemint;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rb::{RB, RbConsumer, RbProducer, SpscRb};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

pub struct NowPlayingMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    pub artwork: Option<String>,
}

struct PlaybackState {
    paused: bool,
    stopped: bool,
    volume: f32,
    seek_to: Option<Duration>,
    finished: bool,
}

pub struct Player {
    state: Arc<Mutex<PlaybackState>>,
    _device: cpal::Device,
    stream_config: cpal::StreamConfig,
    _stream: Option<cpal::Stream>,
    ring_buf_consumer: Option<Arc<Mutex<rb::Consumer<f32>>>>,
    decoder_handle: Option<std::thread::JoinHandle<()>>,

    start_time: Option<Instant>,
    elapsed: Duration,

    now_playing: Option<NowPlayingMeta>,
}

impl Player {
    pub fn new() -> Self {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("no output device available");

        let supported_config = device
            .default_output_config()
            .expect("no default output config");

        let stream_config: cpal::StreamConfig = supported_config.into();

        Self {
            state: Arc::new(Mutex::new(PlaybackState {
                paused: false,
                stopped: false,
                volume: 1.0,
                seek_to: None,
                finished: false,
            })),
            _device: device,
            stream_config,
            _stream: None,
            ring_buf_consumer: None,
            decoder_handle: None,
            start_time: None,
            elapsed: Duration::from_secs(0),
            now_playing: None,
        }
    }

    pub fn play(
        &mut self,
        source: Box<dyn symphonia::core::io::MediaSource>,
        meta: NowPlayingMeta,
        hint: Hint,
    ) {
        self.stop_internal();

        let state = Arc::new(Mutex::new(PlaybackState {
            paused: false,
            stopped: false,
            volume: {
                let old = self.state.lock().unwrap();
                old.volume
            },
            seek_to: None,
            finished: false,
        }));
        self.state = state.clone();

        let channels = self.stream_config.channels as usize;
        let device_sample_rate = self.stream_config.sample_rate;

        let ring_buf_size = device_sample_rate as usize * channels * 2;
        let ring_buf = SpscRb::new(ring_buf_size);
        let (producer, consumer) = (ring_buf.producer(), ring_buf.consumer());
        let consumer = Arc::new(Mutex::new(consumer));
        self.ring_buf_consumer = Some(consumer.clone());

        let stream_state = state.clone();
        let stream_consumer = consumer.clone();

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("no output device available");

        let stream = device
            .build_output_stream(
                &self.stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let st = stream_state.lock().unwrap();
                    let volume = st.volume;
                    let paused = st.paused;
                    drop(st);

                    if paused {
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                        return;
                    }

                    let cons = stream_consumer.lock().unwrap();
                    let read = cons.read(data).unwrap_or(0);
                    drop(cons);

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
            .expect("failed to build output stream");

        stream.play().expect("failed to start output stream");
        self._stream = Some(stream);

        let decoder_state = state.clone();
        let decoder_channels = channels;
        let decoder_sample_rate = device_sample_rate;

        let handle = std::thread::spawn(move || {
            Self::decoder_thread(
                source,
                hint,
                producer,
                decoder_state,
                decoder_channels,
                decoder_sample_rate,
            );
        });
        self.decoder_handle = Some(handle);

        self.start_time = Some(Instant::now());
        self.elapsed = Duration::from_secs(0);
        self.now_playing = Some(meta);

        self.update_now_playing_system();
    }

    fn decoder_thread(
        source: Box<dyn symphonia::core::io::MediaSource>,
        hint: Hint,
        producer: rb::Producer<f32>,
        state: Arc<Mutex<PlaybackState>>,
        target_channels: usize,
        target_sample_rate: u32,
    ) {
        let mss = MediaSourceStream::new(source, Default::default());

        let probed = match symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("symphonia probe error: {}", e);
                let mut st = state.lock().unwrap();
                st.finished = true;
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
                let mut st = state.lock().unwrap();
                st.finished = true;
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

        let mut decoder = match symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
        {
            Ok(d) => d,
            Err(e) => {
                eprintln!("symphonia codec error: {}", e);
                let mut st = state.lock().unwrap();
                st.finished = true;
                return;
            }
        };

        loop {
            {
                let mut st = state.lock().unwrap();
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
                    if let Err(e) = format.seek(SeekMode::Coarse, seek_to) {
                        eprintln!("seek error: {}", e);
                    } else {
                        decoder.reset();
                    }
                }

                while st.paused && !st.stopped {
                    drop(st);
                    std::thread::sleep(Duration::from_millis(10));
                    st = state.lock().unwrap();
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
                    let mut st = state.lock().unwrap();
                    st.finished = true;
                    return;
                }
                Err(symphonia::core::errors::Error::ResetRequired) => {
                    decoder.reset();
                    continue;
                }
                Err(e) => {
                    eprintln!("format error: {}", e);
                    let mut st = state.lock().unwrap();
                    st.finished = true;
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
                    let mut st = state.lock().unwrap();
                    st.finished = true;
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
                    let st = state.lock().unwrap();
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
        let src_frames = samples.len() / channels;
        let ratio = dst_rate as f64 / src_rate as f64;
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
        let mut st = self.state.lock().unwrap();
        if !st.paused {
            st.paused = true;
            drop(st);

            if let Some(start) = self.start_time {
                self.elapsed += start.elapsed();
                self.start_time = None;
            }

            self.update_now_playing_system();
        }
    }

    pub fn play_resume(&mut self) {
        let mut st = self.state.lock().unwrap();
        if st.paused {
            st.paused = false;
            drop(st);

            self.start_time = Some(Instant::now());
            self.update_now_playing_system();
        }
    }

    pub fn seek(&mut self, time: Duration) {
        {
            let mut st = self.state.lock().unwrap();
            st.seek_to = Some(time);
        }

        self.elapsed = time;
        if !self.is_paused() {
            self.start_time = Some(Instant::now());
        }

        self.update_now_playing_system();
    }

    pub fn is_empty(&self) -> bool {
        let st = self.state.lock().unwrap();
        st.finished
    }

    pub fn is_paused(&self) -> bool {
        let st = self.state.lock().unwrap();
        st.paused
    }

    pub fn stop(&mut self) {
        self.stop_internal();
        self.start_time = None;
        self.elapsed = Duration::from_secs(0);
        self.now_playing = None;
    }

    fn stop_internal(&mut self) {
        {
            let mut st = self.state.lock().unwrap();
            st.stopped = true;
            st.paused = false;
        }

        self._stream = None;
        self.ring_buf_consumer = None;

        if let Some(handle) = self.decoder_handle.take() {
            let _ = handle.join();
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        let mut st = self.state.lock().unwrap();
        st.volume = volume;
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
        let raw = if let Some(start) = self.start_time {
            self.elapsed + start.elapsed()
        } else {
            self.elapsed
        };

        if let Some(meta) = &self.now_playing {
            if meta.duration > Duration::ZERO && raw > meta.duration {
                return meta.duration;
            }
        }
        raw
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}
