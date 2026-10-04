use std::ptr::NonNull;
use std::sync::Mutex as StdMutex;
use std::sync::{Arc, OnceLock};

use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::NSImage;
use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryPlayback};
use objc2_foundation::{
    NSCopying, NSDictionary, NSMutableDictionary, NSNumber, NSProcessInfo, NSString,
};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle,
    MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
    MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingInfoPropertyPlaybackRate, MPRemoteCommandCenter, MPRemoteCommandEvent,
    MPRemoteCommandHandlerStatus,
};

unsafe extern "C" {
    fn CFRunLoopGetMain() -> *mut std::ffi::c_void;
    fn CFRunLoopWakeUp(rl: *mut std::ffi::c_void);
    fn CFRunLoopRun();
    fn CFRunLoopAddTimer(
        rl: *mut std::ffi::c_void,
        timer: *mut std::ffi::c_void,
        mode: *const std::ffi::c_void,
    );
    fn CFRunLoopTimerCreate(
        allocator: *const std::ffi::c_void,
        fire_date: f64,
        interval: f64,
        flags: u64,
        order: i64,
        callout: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void),
        context: *const std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn CFAbsoluteTimeGetCurrent() -> f64;
    static kCFRunLoopCommonModes: *const std::ffi::c_void;
}

type IOPMAssertionID = u32;
#[link(name = "IOKit", kind = "framework")]

unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: *const std::ffi::c_void,
        assertion_level: u32,
        reason: *const std::ffi::c_void,
        assertion_id: *mut IOPMAssertionID,
    ) -> i32;
}

/// Park the calling thread in the main CFRunLoop forever. A headless daemon
/// must call this from its main thread (with async work on other threads) or
/// the Now Playing heartbeat timer and MPRemoteCommandCenter callbacks
/// installed by [`init`] never fire.
pub fn park_main_loop() {
    unsafe { CFRunLoopRun() }
}

pub fn wake_run_loop() {
    unsafe { CFRunLoopWakeUp(CFRunLoopGetMain()) }
}

#[derive(Debug)]
pub enum SystemEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Seek(f64),
}

type BgHandler = Arc<StdMutex<Option<Box<dyn Fn(SystemEvent) + Send + Sync>>>>;
type TokioWaker = Arc<StdMutex<Option<Box<dyn Fn() + Send + Sync>>>>;

static BACKGROUND_HANDLER: OnceLock<BgHandler> = OnceLock::new();

static TOKIO_WAKER: OnceLock<TokioWaker> = OnceLock::new();

fn get_bg_handler() -> BgHandler {
    BACKGROUND_HANDLER
        .get_or_init(|| Arc::new(StdMutex::new(None)))
        .clone()
}

fn get_tokio_waker() -> TokioWaker {
    TOKIO_WAKER
        .get_or_init(|| Arc::new(StdMutex::new(None)))
        .clone()
}

pub fn set_tokio_waker(waker: impl Fn() + Send + Sync + 'static) {
    let arc = get_tokio_waker();
    let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(Box::new(waker));
}

fn wake_tokio() {
    if let Ok(guard) = get_tokio_waker().lock()
        && let Some(ref waker) = *guard
    {
        waker();
    }
}

pub fn set_background_handler(handler: impl Fn(SystemEvent) + Send + Sync + 'static) {
    let arc = get_bg_handler();
    let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(Box::new(handler));
}

fn dispatch_event(event: SystemEvent) {
    if let Ok(guard) = get_bg_handler().lock()
        && let Some(ref handler) = *guard
    {
        handler(event);
    }
    wake_run_loop();
}

unsafe extern "C" fn main_loop_heartbeat(
    _timer: *mut std::ffi::c_void,
    _info: *mut std::ffi::c_void,
) {
    wake_tokio();
}

pub fn init() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        use objc2::ClassType;
        let process_info: *mut AnyObject = objc2::msg_send![NSProcessInfo::class(), processInfo];
        let reason = NSString::from_str("Kopuz Background Audio Playback");
        let options: u64 = 0x00FFFFFF | 0xFF00000000;
        let activity: *mut AnyObject =
            objc2::msg_send![process_info, beginActivityWithOptions: options, reason: &*reason];
        if !activity.is_null() {
            let _: *mut AnyObject = objc2::msg_send![activity, retain];
            tracing::debug!("App Nap bypassed with NSProcessInfo activity (latency-critical)");
        }

        let assertion_type = NSString::from_str("NoIdleSleepAssertion");
        let assertion_reason = NSString::from_str("Kopuz is playing audio");
        let mut assertion_id: IOPMAssertionID = 0;
        let kr = IOPMAssertionCreateWithName(
            &*assertion_type as *const objc2_foundation::NSString as *const std::ffi::c_void,
            255,
            &*assertion_reason as *const objc2_foundation::NSString as *const std::ffi::c_void,
            &mut assertion_id,
        );
        if kr == 0 {
            tracing::debug!(id = assertion_id, "IOKit power assertion created");
        } else {
            tracing::warn!(kr, "failed to create IOKit power assertion");
        }

        let session = AVAudioSession::sharedInstance();
        if let Some(category) = AVAudioSessionCategoryPlayback {
            if let Err(e) = session.setCategory_error(category) {
                tracing::warn!(error = ?e, "failed to set AVAudioSession category");
            }
            if let Err(e) = session.setActive_error(true) {
                tracing::warn!(error = ?e, "failed to activate AVAudioSession");
            } else {
                tracing::debug!("AVAudioSession configured for background playback");
            }
        } else {
            tracing::error!("AVAudioSessionCategoryPlayback not available");
        }

        let center = MPRemoteCommandCenter::sharedCommandCenter();

        center.playCommand().addTargetWithHandler(&RcBlock::new(
            move |_: NonNull<MPRemoteCommandEvent>| {
                dispatch_event(SystemEvent::Play);
                MPRemoteCommandHandlerStatus::Success
            },
        ));

        center.pauseCommand().addTargetWithHandler(&RcBlock::new(
            move |_: NonNull<MPRemoteCommandEvent>| {
                dispatch_event(SystemEvent::Pause);
                MPRemoteCommandHandlerStatus::Success
            },
        ));

        center
            .togglePlayPauseCommand()
            .addTargetWithHandler(&RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                dispatch_event(SystemEvent::Toggle);
                MPRemoteCommandHandlerStatus::Success
            }));

        center
            .nextTrackCommand()
            .addTargetWithHandler(&RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                dispatch_event(SystemEvent::Next);
                MPRemoteCommandHandlerStatus::Success
            }));

        center
            .previousTrackCommand()
            .addTargetWithHandler(&RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                dispatch_event(SystemEvent::Prev);
                MPRemoteCommandHandlerStatus::Success
            }));

        center
            .changePlaybackPositionCommand()
            .addTargetWithHandler(&RcBlock::new(
                move |event: NonNull<MPRemoteCommandEvent>| {
                    match event
                        .as_ref()
                        .downcast_ref::<MPChangePlaybackPositionCommandEvent>()
                    {
                        Some(event) => {
                            dispatch_event(SystemEvent::Seek(event.positionTime()));
                            MPRemoteCommandHandlerStatus::Success
                        }
                        None => MPRemoteCommandHandlerStatus::CommandFailed,
                    }
                },
            ));

        let fire_date = CFAbsoluteTimeGetCurrent();
        let timer = CFRunLoopTimerCreate(
            std::ptr::null(),
            fire_date,
            0.25,
            0,
            0,
            main_loop_heartbeat,
            std::ptr::null(),
        );
        if !timer.is_null() {
            CFRunLoopAddTimer(CFRunLoopGetMain(), timer, kCFRunLoopCommonModes);
            tracing::debug!("CFRunLoopTimer heartbeat started on main run loop (250ms)");
        } else {
            tracing::warn!("failed to create CFRunLoopTimer, falling back to thread");
            std::thread::spawn(|| {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    wake_tokio();
                    wake_run_loop();
                }
            });
        }
    });
}

/// Cached `MPMediaItemArtwork` keyed by source path. Building one decodes the
/// whole image file, far too slow to repeat on every position push (scrubbing
/// floods them); a cache hit is just a retain.
struct ArtworkCache {
    path: String,
    artwork: objc2::rc::Retained<MPMediaItemArtwork>,
}

unsafe impl Send for ArtworkCache {}

static ARTWORK_CACHE: StdMutex<Option<ArtworkCache>> = StdMutex::new(None);

fn artwork_for_path(path: &str) -> Option<objc2::rc::Retained<MPMediaItemArtwork>> {
    let mut cache = ARTWORK_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = cache.as_ref()
        && c.path == path
    {
        return Some(c.artwork.clone());
    }

    let artwork = unsafe {
        let ns_path = NSString::from_str(path);
        let image = NSImage::initWithContentsOfFile(NSImage::alloc(), &ns_path)?;
        use objc2::msg_send;
        let artwork_alloc = MPMediaItemArtwork::alloc();
        let artwork_ptr: *mut MPMediaItemArtwork = std::mem::transmute(artwork_alloc);
        let artwork_raw: *mut MPMediaItemArtwork = msg_send![artwork_ptr, initWithImage: &*image];
        objc2::rc::Retained::from_raw(artwork_raw)?
    };
    *cache = Some(ArtworkCache {
        path: path.to_string(),
        artwork: artwork.clone(),
    });
    Some(artwork)
}

pub fn update_now_playing(
    title: &str,
    artist: &str,
    album: &str,
    duration: f64,
    position: f64,
    playing: bool,
    artwork_path: Option<&str>,
) {
    init();

    unsafe {
        let center = MPNowPlayingInfoCenter::defaultCenter();

        let title_ns = NSString::from_str(title);
        let artist_ns = NSString::from_str(artist);
        let album_ns = NSString::from_str(album);
        let duration_ns = NSNumber::numberWithDouble(duration);
        let position_ns = NSNumber::numberWithDouble(position);
        let rate_ns = NSNumber::numberWithDouble(if playing { 1.0 } else { 0.0 });

        let info = NSMutableDictionary::<ProtocolObject<dyn NSCopying>, AnyObject>::new();

        info.setObject_forKey(
            std::mem::transmute::<&NSString, &AnyObject>(&*title_ns),
            ProtocolObject::from_ref(MPMediaItemPropertyTitle),
        );
        info.setObject_forKey(
            std::mem::transmute::<&NSString, &AnyObject>(&*artist_ns),
            ProtocolObject::from_ref(MPMediaItemPropertyArtist),
        );
        info.setObject_forKey(
            std::mem::transmute::<&NSString, &AnyObject>(&*album_ns),
            ProtocolObject::from_ref(MPMediaItemPropertyAlbumTitle),
        );
        info.setObject_forKey(
            std::mem::transmute::<&NSNumber, &AnyObject>(&*duration_ns),
            ProtocolObject::from_ref(MPMediaItemPropertyPlaybackDuration),
        );
        info.setObject_forKey(
            std::mem::transmute::<&NSNumber, &AnyObject>(&*position_ns),
            ProtocolObject::from_ref(MPNowPlayingInfoPropertyElapsedPlaybackTime),
        );
        info.setObject_forKey(
            std::mem::transmute::<&NSNumber, &AnyObject>(&*rate_ns),
            ProtocolObject::from_ref(MPNowPlayingInfoPropertyPlaybackRate),
        );

        if let Some(retained) = artwork_path.and_then(artwork_for_path) {
            let artwork_ref: &AnyObject =
                &*(&*retained as *const objc2_media_player::MPMediaItemArtwork as *const AnyObject);
            info.setObject_forKey(
                artwork_ref,
                ProtocolObject::from_ref(MPMediaItemPropertyArtwork),
            );
        }

        center.setNowPlayingInfo(Some(std::mem::transmute::<
            &NSMutableDictionary<_, _>,
            &NSDictionary<NSString, AnyObject>,
        >(&*info)));
    }
}

pub fn refresh_now_playing() {
    unsafe {
        let center = MPNowPlayingInfoCenter::defaultCenter();
        let existing = center.nowPlayingInfo();
        center.setNowPlayingInfo(existing.as_deref());
    }
}
