use std::ptr::NonNull;
use std::sync::Mutex as StdMutex;
use std::sync::{Arc, OnceLock};

use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::NSImage;
use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryPlayback};
use objc2_foundation::{NSCopying, NSDictionary, NSMutableDictionary, NSNumber, NSString};
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
}

pub fn wake_run_loop() {
    unsafe { CFRunLoopWakeUp(CFRunLoopGetMain()) }
}

const NS_ACTIVITY_PREVENT_SUSPENSION: u64 =
    0x00FFFFFF | (1 << 20) | (1 << 14) | (1 << 15) | 0xFF00000000;

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

struct ThreadSafeActivity(objc2::rc::Retained<AnyObject>);
unsafe impl Send for ThreadSafeActivity {}
unsafe impl Sync for ThreadSafeActivity {}

static BACKGROUND_HANDLER: OnceLock<Arc<StdMutex<Option<Box<dyn Fn(SystemEvent) + Send + Sync>>>>> =
    OnceLock::new();

fn get_bg_handler() -> Arc<StdMutex<Option<Box<dyn Fn(SystemEvent) + Send + Sync>>>> {
    BACKGROUND_HANDLER
        .get_or_init(|| Arc::new(StdMutex::new(None)))
        .clone()
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

pub fn init() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        let process_info = objc2_foundation::NSProcessInfo::processInfo();
        let reason = NSString::from_str("Music Playback Quality of Service");
        let activity: *mut AnyObject = objc2::msg_send![
            &process_info,
            beginActivityWithOptions: NS_ACTIVITY_PREVENT_SUSPENSION,
            reason: &*reason
        ];

        let session = AVAudioSession::sharedInstance();
        if let Err(e) = session.setCategory_error(AVAudioSessionCategoryPlayback.unwrap()) {
            eprintln!("[macos] Failed to set AVAudioSession category: {:?}", e);
        }
        if let Err(e) = session.setActive_error(true) {
            eprintln!("[macos] Failed to activate AVAudioSession: {:?}", e);
        } else {
            println!("[macos] AVAudioSession configured for background playback");
        }

        static ACTIVITY_TOKEN: OnceLock<ThreadSafeActivity> = OnceLock::new();
        if !activity.is_null() {
            let retained =
                objc2::rc::Retained::from_raw(activity).expect("retained activity token");
            let _ = ACTIVITY_TOKEN.set(ThreadSafeActivity(retained));
            println!("[macos] Acquired background activity token");
        } else {
            eprintln!("[macos] Failed to acquire background activity token");
        }

        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                wake_run_loop();
            }
        });

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
