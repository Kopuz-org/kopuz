//! The picture on macOS: a muted AVPlayer reading the daemon's byte ranges through a resource loader.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use api::{ApiError, VideoChunk};
use block2::RcBlock;
use core_foundation::base::TCFType as _;
use core_video::pixel_buffer::CVPixelBuffer as Frame;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AnyThread as _, DefinedClass as _, MainThreadMarker, Message as _, define_class, msg_send,
};
use objc2_av_foundation::{
    AVAssetResourceLoader, AVAssetResourceLoaderDelegate, AVAssetResourceLoadingRequest, AVPlayer,
    AVPlayerActionAtItemEnd, AVPlayerItem, AVPlayerItemStatus, AVPlayerItemVideoOutput, AVURLAsset,
};
use objc2_core_media::{CMTime, kCMTimeZero};
use objc2_core_video::{
    kCVPixelBufferMetalCompatibilityKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use objc2_foundation::{NSData, NSDictionary, NSError, NSNumber, NSString, NSURL};

// A scheme AVFoundation cannot load itself, so every range is asked of the delegate.
const URL: &str = "kopuz-video://picture/video.mp4";
// The daemon answers at most this much per call.
const CHUNK: u64 = 2 * 1024 * 1024;
// A drift found this soon after a seek is what that seek cost, so the next aims that much ahead.
const LEARN_WINDOW: Duration = Duration::from_secs(4);
const PLAYING_DRIFT: f64 = 0.15;
const PAUSED_DRIFT: f64 = 0.05;
const TIMESCALE: i32 = 600;

/// Where a range of the video's bytes comes from: start and length in, the chunk out, called off the main thread.
pub(crate) type Ranges = Box<dyn Fn(u64, u64) -> Result<VideoChunk, ApiError> + Send + Sync>;

struct Feed {
    ranges: Ranges,
    closed: AtomicBool,
}

enum Served {
    Done,
    Dropped,
}

/// A loading request handed to the thread that answers it.
struct Pending(Retained<AVAssetResourceLoadingRequest>);

// AVFoundation takes a loading request's data and its finish from any thread.
unsafe impl Send for Pending {}

impl Pending {
    fn into_inner(self) -> Retained<AVAssetResourceLoadingRequest> {
        self.0
    }
}

impl Feed {
    fn closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    fn serve(self: Arc<Self>, request: Pending) {
        let spawned = std::thread::Builder::new()
            .name("music-video-range".into())
            .spawn(move || {
                let request = request.into_inner();
                match self.answer(&request) {
                    Ok(Served::Done) => unsafe { request.finishLoading() },
                    Ok(Served::Dropped) => {}
                    Err(error) => {
                        tracing::debug!(%error, "music video range failed");
                        let domain = NSString::from_str("moe.kopuz.music-video");
                        let error = NSError::new(-1, &domain);
                        unsafe { request.finishLoadingWithError(Some(&error)) };
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start a music video range");
        }
    }

    fn answer(&self, request: &AVAssetResourceLoadingRequest) -> Result<Served, ApiError> {
        debug_assert!(
            MainThreadMarker::new().is_none(),
            "a range blocks on the daemon, never on the main thread"
        );
        let info = unsafe { request.contentInformationRequest() };
        let data = unsafe { request.dataRequest() };
        let (mut at, mut end) = match &data {
            Some(data) => unsafe {
                let start = data.requestedOffset().max(0) as u64;
                let end = if data.requestsAllDataToEndOfResource() {
                    u64::MAX
                } else {
                    start.saturating_add(data.requestedLength().max(0) as u64)
                };
                (data.currentOffset().max(0) as u64, end)
            },
            None => (0, 1),
        };
        let mut described = info.is_none();
        while at < end || !described {
            if self.closed() || unsafe { request.isCancelled() } {
                return Ok(Served::Dropped);
            }
            let chunk = (self.ranges)(at, end.saturating_sub(at).clamp(1, CHUNK))?;
            if let Some(info) = info.as_ref().filter(|_| !described) {
                unsafe {
                    info.setContentType(Some(&NSString::from_str(uti(&chunk.content_type))));
                    if let Some(total) = chunk.total {
                        info.setContentLength(total as i64);
                    }
                    info.setByteRangeAccessSupported(true);
                }
                described = true;
            }
            if let Some(total) = chunk.total {
                end = end.min(total);
            }
            let Some(data) = &data else { break };
            if chunk.bytes.is_empty() {
                break;
            }
            unsafe { data.respondWithData(&NSData::with_bytes(&chunk.bytes)) };
            at += chunk.bytes.len() as u64;
        }
        Ok(Served::Done)
    }
}

/// The type identifier AVFoundation wants for the stream's MIME type.
fn uti(mime: &str) -> &'static str {
    match mime.split(';').next().map(str::trim) {
        Some("video/quicktime") => "com.apple.quicktime-movie",
        Some("video/x-m4v") => "com.apple.m4v-video",
        _ => "public.mpeg-4",
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Loader` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "KopuzMusicVideoLoader"]
    #[ivars = Arc<Feed>]
    struct Loader;

    unsafe impl NSObjectProtocol for Loader {}

    unsafe impl AVAssetResourceLoaderDelegate for Loader {
        #[unsafe(method(resourceLoader:shouldWaitForLoadingOfRequestedResource:))]
        fn should_wait(
            &self,
            _: &AVAssetResourceLoader,
            request: &AVAssetResourceLoadingRequest,
        ) -> bool {
            self.ivars().clone().serve(Pending(request.retain()));
            true
        }
    }
);

impl Loader {
    fn new(feed: Arc<Feed>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(feed);
        unsafe { msg_send![super(this), init] }
    }
}

/// A queued music video playing muted, whose frames a GPUI surface draws.
pub(crate) struct Picture {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    output: Retained<AVPlayerItemVideoOutput>,
    asset: Retained<AVURLAsset>,
    feed: Arc<Feed>,
    _loader: Retained<Loader>,
    _queue: DispatchRetained<DispatchQueue>,
    frame: Option<Frame>,
    seeked: Option<Instant>,
    /// Set while a seek is in flight, so a slow one is not interrupted by the next correction.
    seeking: Arc<AtomicBool>,
    lead: f64,
}

/// Whether drift this large calls for a seek; never while one is still landing.
fn needs_seek(drift: f64, playing: bool, seeking: bool) -> bool {
    let limit = if playing { PLAYING_DRIFT } else { PAUSED_DRIFT };
    !seeking && drift.abs() > limit
}

impl Picture {
    /// A picture read through `ranges`; only the main thread can build one.
    pub(crate) fn open(ranges: Ranges) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let url = NSURL::URLWithString(&NSString::from_str(URL))?;
        let feed = Arc::new(Feed {
            ranges,
            closed: AtomicBool::new(false),
        });
        let loader = Loader::new(feed.clone());
        let queue = DispatchQueue::new("moe.kopuz.music-video", None);
        unsafe {
            let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
            asset
                .resourceLoader()
                .setDelegate_queue(Some(ProtocolObject::from_ref(&*loader)), Some(&queue));
            let item = AVPlayerItem::playerItemWithAsset(&asset, mtm);
            let output = video_output();
            item.addOutput(&output);
            let player = AVPlayer::playerWithPlayerItem(Some(&item), mtm);
            player.setMuted(true);
            player.setActionAtItemEnd(AVPlayerActionAtItemEnd::Pause);
            player.setPreventsDisplaySleepDuringVideoPlayback(false);
            Some(Self {
                player,
                item,
                output,
                asset,
                feed,
                _loader: loader,
                _queue: queue,
                frame: None,
                seeked: None,
                seeking: Arc::new(AtomicBool::new(false)),
                lead: 0.,
            })
        }
    }

    /// Hold the picture to `position` seconds and the engine's play state, seeking only when it has drifted.
    pub(crate) fn follow(&mut self, position: f64, playing: bool) {
        unsafe {
            let rate = self.player.rate();
            if playing && rate == 0. {
                self.player.play();
            } else if !playing && rate != 0. {
                self.player.pause();
            }
            if self.item.status() != AVPlayerItemStatus::ReadyToPlay {
                return;
            }
            let drift = self.item.currentTime().seconds() - position;
            if !needs_seek(drift, playing, self.seeking.load(Ordering::Relaxed)) {
                return;
            }
            let seeked = self.seeked.map(|at| at.elapsed());
            if playing && seeked.is_some_and(|since| since < LEARN_WINDOW) && drift.abs() < 5. {
                self.lead = (self.lead - drift).clamp(0., 1.);
            }
            let target = position + if playing { self.lead } else { 0. };
            self.seeking.store(true, Ordering::Relaxed);
            let seeking = self.seeking.clone();
            // Called once the seek lands or a later one supersedes it, which both end this one.
            let landed = RcBlock::new(move |_: Bool| seeking.store(false, Ordering::Relaxed));
            self.player
                .seekToTime_toleranceBefore_toleranceAfter_completionHandler(
                    CMTime::with_seconds(target.max(0.), TIMESCALE),
                    kCMTimeZero,
                    kCMTimeZero,
                    &landed,
                );
        }
        self.seeked = Some(Instant::now());
    }

    /// The newest decoded frame, or the last one while no newer has arrived.
    pub(crate) fn frame(&mut self) -> Option<Frame> {
        unsafe {
            let now = self.item.currentTime();
            if self.output.hasNewPixelBufferForItemTime(now)
                && let Some(buffer) = self
                    .output
                    .copyPixelBufferForItemTime_itemTimeForDisplay(now, std::ptr::null_mut())
            {
                let raw = Retained::into_raw(buffer);
                self.frame = Some(Frame::wrap_under_create_rule(raw.cast()));
            }
        }
        self.frame.clone()
    }
}

impl Drop for Picture {
    fn drop(&mut self) {
        self.feed.closed.store(true, Ordering::Relaxed);
        unsafe {
            self.player.pause();
            self.player.replaceCurrentItemWithPlayerItem(None);
            self.asset.resourceLoader().setDelegate_queue(None, None);
        }
    }
}

/// Frames in the biplanar 4:2:0 layout GPUI's Metal surface pipeline samples.
fn video_output() -> Retained<AVPlayerItemVideoOutput> {
    let format = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange);
    let metal = NSNumber::new_bool(true);
    let (format_key, metal_key): (&NSString, &NSString) = unsafe {
        (
            kCVPixelBufferPixelFormatTypeKey.as_ref(),
            kCVPixelBufferMetalCompatibilityKey.as_ref(),
        )
    };
    let attributes = NSDictionary::<NSString, AnyObject>::from_slices(
        &[format_key, metal_key],
        &[format.as_ref(), metal.as_ref()],
    );
    unsafe {
        AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
            AVPlayerItemVideoOutput::alloc(),
            Some(&attributes),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::needs_seek;

    #[test]
    fn a_seek_still_landing_holds_off_the_next_correction() {
        assert!(needs_seek(2.0, true, false));
        assert!(!needs_seek(2.0, true, true));
        assert!(!needs_seek(-9.0, false, true));
    }

    #[test]
    fn a_paused_picture_is_held_closer_than_a_playing_one() {
        assert!(!needs_seek(0.1, true, false));
        assert!(needs_seek(0.1, false, false));
        assert!(!needs_seek(0.04, false, false));
    }
}
