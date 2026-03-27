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
    MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist,
    MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle,
    MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingInfoPropertyPlaybackRate, MPRemoteCommandCenter, MPRemoteCommandEvent,
    MPRemoteCommandHandlerStatus,
};

unsafe extern "C" {
    fn CFRunLoopGetMain() -> *mut std::ffi::c_void;
    fn CFRunLoopWakeUp(rl: *mut std::ffi::c_void);
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
}

struct ThreadSafeArtwork(objc2::rc::Retained<MPMediaItemArtwork>);
unsafe impl Send for ThreadSafeArtwork {}
unsafe impl Sync for ThreadSafeArtwork {}

static BACKGROUND_HANDLER: OnceLock<Arc<StdMutex<Option<Box<dyn Fn(SystemEvent) + Send + Sync>>>>> =
    OnceLock::new();

static TOKIO_WAKER: OnceLock<Arc<StdMutex<Option<Box<dyn Fn() + Send + Sync>>>>> = OnceLock::new();

fn get_bg_handler() -> Arc<StdMutex<Option<Box<dyn Fn(SystemEvent) + Send + Sync>>>> {
    BACKGROUND_HANDLER
        .get_or_init(|| Arc::new(StdMutex::new(None)))
        .clone()
}

fn get_tokio_waker() -> Arc<StdMutex<Option<Box<dyn Fn() + Send + Sync>>>> {
    TOKIO_WAKER
        .get_or_init(|| Arc::new(StdMutex::new(None)))
        .clone()
}

pub fn set_tokio_waker(waker: impl Fn() + Send + Sync + 'static) {
    let arc = get_tokio_waker();
    let mut guard = arc.lock().unwrap();
    *guard = Some(Box::new(waker));
}

fn wake_tokio() {
    if let Ok(guard) = get_tokio_waker().lock() {
        if let Some(ref waker) = *guard {
            waker();
        }
    }
}

pub fn set_background_handler(handler: impl Fn(SystemEvent) + Send + Sync + 'static) {
    let arc = get_bg_handler();
    let mut guard = arc.lock().unwrap();
    *guard = Some(Box::new(handler));
}

fn dispatch_event(event: SystemEvent) {
    if let Ok(guard) = get_bg_handler().lock() {
        if let Some(ref handler) = *guard {
            handler(event);
        }
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
        let reason = NSString::from_str("Rusic Background Audio Playback");
        let options: u64 = 0x00FFFFFF | 0xFF00000000;
        let activity: *mut AnyObject =
            objc2::msg_send![process_info, beginActivityWithOptions: options, reason: &*reason];
        if !activity.is_null() {
            let _: *mut AnyObject = objc2::msg_send![activity, retain];
            println!("[macos] App Nap bypassed with NSProcessInfo activity (latency-critical)");
        }

        let assertion_type = NSString::from_str("NoIdleSleepAssertion");
        let assertion_reason = NSString::from_str("Rusic is playing audio");
        let mut assertion_id: IOPMAssertionID = 0;
        let kr = IOPMAssertionCreateWithName(
            std::mem::transmute::<&objc2_foundation::NSString, *const std::ffi::c_void>(
                &*assertion_type,
            ),
            255,
            std::mem::transmute::<&objc2_foundation::NSString, *const std::ffi::c_void>(
                &*assertion_reason,
            ),
            &mut assertion_id,
        );
        if kr == 0 {
            println!(
                "[macos] IOKit power assertion created (id={})",
                assertion_id
            );
        } else {
            eprintln!("[macos] Failed to create IOKit power assertion: {}", kr);
        }

        let session = AVAudioSession::sharedInstance();
        if let Err(e) = session.setCategory_error(AVAudioSessionCategoryPlayback.unwrap()) {
            eprintln!("[macos] Failed to set AVAudioSession category: {:?}", e);
        }
        if let Err(e) = session.setActive_error(true) {
            eprintln!("[macos] Failed to activate AVAudioSession: {:?}", e);
        } else {
            println!("[macos] AVAudioSession configured for background playback");
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
            println!("[macos] CFRunLoopTimer heartbeat started on main run loop (250ms)");
        } else {
            eprintln!("[macos] Failed to create CFRunLoopTimer, falling back to thread");
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

    static ARTWORK_CACHE: OnceLock<std::sync::Mutex<Option<(String, ThreadSafeArtwork)>>> =
        OnceLock::new();

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

        if let Some(path) = artwork_path {
            let cache_lock = ARTWORK_CACHE.get_or_init(|| std::sync::Mutex::new(None));
            let mut cache = cache_lock.lock().unwrap();

            let cached_artwork = if let Some((cached_path, artwork_wrapper)) = &*cache {
                if cached_path == path {
                    Some(artwork_wrapper.0.clone())
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(artwork) = cached_artwork {
                let artwork_ref: &AnyObject =
                    &*(std::mem::transmute::<_, *const AnyObject>(&*artwork));
                info.setObject_forKey(
                    artwork_ref,
                    ProtocolObject::from_ref(MPMediaItemPropertyArtwork),
                );
            } else {
                let ns_path = NSString::from_str(path);
                if let Some(image) = NSImage::initWithContentsOfFile(NSImage::alloc(), &ns_path) {
                    use objc2::msg_send;
                    let artwork_alloc = MPMediaItemArtwork::alloc();
                    let artwork_ptr: *mut MPMediaItemArtwork = std::mem::transmute(artwork_alloc);
                    let artwork_raw: *mut MPMediaItemArtwork =
                        msg_send![artwork_ptr, initWithImage: &*image];

                    if !artwork_raw.is_null() {
                        let retained: objc2::rc::Retained<MPMediaItemArtwork> =
                            objc2::rc::Retained::from_raw(artwork_raw).expect("retained artwork");

                        let artwork_ref: &AnyObject =
                            &*(std::mem::transmute::<_, *const AnyObject>(&*retained));
                        info.setObject_forKey(
                            artwork_ref,
                            ProtocolObject::from_ref(MPMediaItemPropertyArtwork),
                        );

                        *cache = Some((path.to_string(), ThreadSafeArtwork(retained)));
                    }
                }
            }
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
