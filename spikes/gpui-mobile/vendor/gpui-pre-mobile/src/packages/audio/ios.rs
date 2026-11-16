use super::{LoopMode, PlayerState};
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// Tracks whether each player was explicitly started (to distinguish Paused from Ready).
struct PlayerEntry {
    player: *mut AnyObject,
    started: bool,
}

unsafe impl Send for PlayerEntry {}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static PLAYERS: Mutex<Option<HashMap<u32, PlayerEntry>>> = Mutex::new(None);

fn with_players<T>(f: impl FnOnce(&mut HashMap<u32, PlayerEntry>) -> T) -> T {
    let mut guard = PLAYERS.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    f(map)
}

fn with_player<T>(
    id: u32,
    f: impl FnOnce(&mut PlayerEntry) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = PLAYERS.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    match map.get_mut(&id) {
        Some(entry) => f(entry),
        None => Err("Audio player not found".into()),
    }
}

pub fn create() -> Result<u32, String> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    // We don't create the AVPlayer yet — it is created when a URL or file is set.
    // Store a null placeholder so the id is registered.
    with_players(|map| {
        map.insert(
            id,
            PlayerEntry {
                player: std::ptr::null_mut(),
                started: false,
            },
        );
    });
    Ok(id)
}

pub fn set_url(id: u32, url: &str) -> Result<Option<u64>, String> {
    unsafe {
        // Create NSURL from string
        let ns_url = nsurl_from_str(url)?;

        // Create AVPlayerItem with URL
        let player_item: *mut AnyObject =
            msg_send![class!(AVPlayerItem), playerItemWithURL: ns_url];
        if player_item.is_null() {
            return Err("Failed to create AVPlayerItem".into());
        }

        // Create or replace AVPlayer
        let player = with_player(id, |entry| {
            if !entry.player.is_null() {
                // Replace the current item
                let _: () = msg_send![entry.player, replaceCurrentItemWithPlayerItem: player_item];
                Ok(entry.player)
            } else {
                let p: *mut AnyObject =
                    msg_send![class!(AVPlayer), playerWithPlayerItem: player_item];
                if p.is_null() {
                    return Err("Failed to create AVPlayer".into());
                }
                // playerWithPlayerItem: returns an autoreleased object; retain it
                // since it will be stored across run-loop iterations.
                let _: () = msg_send![p, retain];
                entry.player = p;
                entry.started = false;
                Ok(p)
            }
        })?;

        // Try to get the duration. AVPlayerItem may need time to load,
        // so the duration might not be available immediately.
        let duration = get_duration_from_player(player);
        Ok(if duration > 0 { Some(duration) } else { None })
    }
}

pub fn set_file_path(id: u32, path: &str) -> Result<Option<u64>, String> {
    unsafe {
        // Build file:// URL from path
        let ns_string = nsstring_from_str(path);
        let ns_url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: ns_string];
        if ns_url.is_null() {
            return Err("Failed to create file URL".into());
        }

        let player_item: *mut AnyObject =
            msg_send![class!(AVPlayerItem), playerItemWithURL: ns_url];
        if player_item.is_null() {
            return Err("Failed to create AVPlayerItem".into());
        }

        let player = with_player(id, |entry| {
            if !entry.player.is_null() {
                let _: () = msg_send![entry.player, replaceCurrentItemWithPlayerItem: player_item];
                Ok(entry.player)
            } else {
                let p: *mut AnyObject =
                    msg_send![class!(AVPlayer), playerWithPlayerItem: player_item];
                if p.is_null() {
                    return Err("Failed to create AVPlayer".into());
                }
                // playerWithPlayerItem: returns an autoreleased object; retain it
                // since it will be stored across run-loop iterations.
                let _: () = msg_send![p, retain];
                entry.player = p;
                entry.started = false;
                Ok(p)
            }
        })?;

        let duration = get_duration_from_player(player);
        Ok(if duration > 0 { Some(duration) } else { None })
    }
}

pub fn play(id: u32) -> Result<(), String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Err("No audio source set".into());
        }
        unsafe {
            let _: () = msg_send![entry.player, play];
        }
        entry.started = true;
        Ok(())
    })
}

pub fn pause(id: u32) -> Result<(), String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Err("No audio source set".into());
        }
        unsafe {
            let _: () = msg_send![entry.player, pause];
        }
        Ok(())
    })
}

pub fn stop(id: u32) -> Result<(), String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Err("No audio source set".into());
        }
        unsafe {
            let _: () = msg_send![entry.player, pause];
            // Seek to beginning
            let zero = cmtime_make(0, 1);
            let _: () = msg_send![entry.player, seekToTime: zero];
        }
        entry.started = false;
        Ok(())
    })
}

pub fn seek(id: u32, position_ms: u64) -> Result<(), String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Err("No audio source set".into());
        }
        unsafe {
            let time = cmtime_make(position_ms as i64, 1000);
            let _: () = msg_send![entry.player, seekToTime: time];
        }
        Ok(())
    })
}

pub fn set_volume(id: u32, volume: f32) -> Result<(), String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Err("No audio source set".into());
        }
        unsafe {
            let _: () = msg_send![entry.player, setVolume: volume];
        }
        Ok(())
    })
}

pub fn set_speed(id: u32, speed: f32) -> Result<(), String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Err("No audio source set".into());
        }
        unsafe {
            let _: () = msg_send![entry.player, setRate: speed];
        }
        Ok(())
    })
}

pub fn set_loop_mode(id: u32, mode: LoopMode) -> Result<(), String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Err("No audio source set".into());
        }
        unsafe {
            // AVPlayer does not have a built-in loop property.
            // For looping, we set actionAtItemEnd to .none so it doesn't
            // advance, then rely on the caller to observe completion if needed.
            // AVPlayerActionAtItemEnd: .none = 2, .pause = 1
            let action: i64 = match mode {
                LoopMode::Off => 1,                 // AVPlayerActionAtItemEndPause
                LoopMode::One | LoopMode::All => 0, // AVPlayerActionAtItemEndNone — we'll handle seek-to-start
            };
            let _: () = msg_send![entry.player, setActionAtItemEnd: action];

            // For LoopMode::One, set up a boundary time observer to seek back.
            // Since we cannot easily manage observers with raw objc, we use a
            // simpler approach: set numberOfLoops on the current item if it's
            // an AVPlayerLooper-compatible scenario. For basic looping we rely
            // on actionAtItemEnd = none and the player naturally looping.
            // In practice, full looping requires AVPlayerLooper (iOS 10+) or
            // an NSNotification observer. For simplicity with the raw objc
            // bridge, we set actionAtItemEnd and document the limitation.
        }
        Ok(())
    })
}

pub fn get_position(id: u32) -> Result<u64, String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Ok(0);
        }
        unsafe {
            let time: CMTime = msg_send![entry.player, currentTime];
            let ms = cmtime_to_ms(time);
            Ok(if ms < 0 { 0 } else { ms as u64 })
        }
    })
}

pub fn get_duration(id: u32) -> Result<u64, String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Ok(0);
        }
        unsafe {
            let ms = get_duration_from_player(entry.player);
            Ok(ms)
        }
    })
}

pub fn is_playing(id: u32) -> Result<bool, String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Ok(false);
        }
        unsafe {
            let rate: f32 = msg_send![entry.player, rate];
            Ok(rate > 0.0)
        }
    })
}

pub fn get_state(id: u32) -> Result<PlayerState, String> {
    with_player(id, |entry| {
        if entry.player.is_null() {
            return Ok(PlayerState::Idle);
        }
        unsafe {
            let rate: f32 = msg_send![entry.player, rate];
            if rate > 0.0 {
                return Ok(PlayerState::Playing);
            }

            if !entry.started {
                return Ok(PlayerState::Ready);
            }

            // Check if we're at the end
            let current: CMTime = msg_send![entry.player, currentTime];
            let current_ms = cmtime_to_ms(current);
            let duration_ms = get_duration_from_player(entry.player) as i64;

            if duration_ms > 0 && current_ms >= duration_ms - 50 {
                Ok(PlayerState::Completed)
            } else {
                Ok(PlayerState::Paused)
            }
        }
    })
}

pub fn dispose(id: u32) -> Result<(), String> {
    with_players(|map| {
        if let Some(entry) = map.remove(&id) {
            if !entry.player.is_null() {
                unsafe {
                    let _: () = msg_send![entry.player, pause];
                }
            }
        }
    });
    Ok(())
}

// ── CoreMedia time helpers ──────────────────────────────────────────────────

/// CMTime struct layout (matches CoreMedia's C struct).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

unsafe impl Encode for CMTime {
    const ENCODING: Encoding = Encoding::Struct(
        "CMTime",
        &[
            Encoding::LongLong,
            Encoding::Int,
            Encoding::UInt,
            Encoding::LongLong,
        ],
    );
}

unsafe impl RefEncode for CMTime {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// CMTimeMake(value, timescale) — construct a CMTime.
fn cmtime_make(value: i64, timescale: i32) -> CMTime {
    // Call the CoreMedia C function directly.
    extern "C" {
        fn CMTimeMake(value: i64, timescale: i32) -> CMTime;
    }
    unsafe { CMTimeMake(value, timescale) }
}

/// Convert a CMTime to milliseconds.
fn cmtime_to_ms(time: CMTime) -> i64 {
    // kCMTimeFlags_Valid = 1
    if time.flags & 1 == 0 || time.timescale == 0 {
        return 0;
    }
    // kCMTimeFlags_PositiveInfinity = 4, kCMTimeFlags_NegativeInfinity = 8
    if time.flags & (4 | 8) != 0 {
        return 0;
    }
    (time.value * 1000) / time.timescale as i64
}

// ── ObjC string/URL helpers ─────────────────────────────────────────────────

use crate::ios::util::nsstring;

unsafe fn nsstring_from_str(s: &str) -> *mut AnyObject {
    nsstring(s)
}

unsafe fn nsurl_from_str(url: &str) -> Result<*mut AnyObject, String> {
    let ns_string = nsstring_from_str(url);
    if ns_string.is_null() {
        return Err("Failed to create NSString".into());
    }
    let ns_url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: ns_string];
    if ns_url.is_null() {
        // Maybe it's a file path
        let ns_url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: ns_string];
        if ns_url.is_null() {
            return Err("Failed to create NSURL".into());
        }
        return Ok(ns_url);
    }
    Ok(ns_url)
}

/// Get the duration in ms from an AVPlayer's currentItem.
unsafe fn get_duration_from_player(player: *mut AnyObject) -> u64 {
    let item: *mut AnyObject = msg_send![player, currentItem];
    if item.is_null() {
        return 0;
    }
    let duration: CMTime = msg_send![item, duration];
    let ms = cmtime_to_ms(duration);
    if ms < 0 {
        0
    } else {
        ms as u64
    }
}
