use super::{AudioFormat, Recording, RecordingConfig};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::sync::Mutex;

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

struct RecorderState {
    recorder: *mut AnyObject, // AVAudioRecorder
    path: String,
    start_time: std::time::Instant,
}

unsafe impl Send for RecorderState {}

static STATE: Mutex<Option<RecorderState>> = Mutex::new(None);

pub fn is_available() -> bool {
    // AVAudioRecorder is available on all iOS versions we support
    true
}

pub fn start_recording(config: &RecordingConfig) -> Result<String, String> {
    let mut guard = STATE.lock().unwrap();
    if guard.is_some() {
        return Err("Already recording".into());
    }

    unsafe {
        // Generate output path
        let ext = match config.format {
            AudioFormat::Aac => "m4a",
            AudioFormat::Wav => "wav",
            AudioFormat::Amr => "amr",
        };
        let uuid_str = uuid::Uuid::new_v4().to_string();
        let file_name = format!("recording_{}.{}", uuid_str, ext);
        let file_path = std::env::temp_dir().join(&file_name);
        let path_str = file_path.to_string_lossy().into_owned();

        // Create NSURL
        let ns_path = nsstring(&path_str);
        let file_url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: ns_path];
        if file_url.is_null() {
            return Err("Failed to create file URL".into());
        }

        // Build settings dictionary
        let keys: [*mut AnyObject; 4] = [
            nsstring("AVFormatIDKey"),
            nsstring("AVSampleRateKey"),
            nsstring("AVNumberOfChannelsKey"),
            nsstring("AVEncoderBitRateKey"),
        ];

        let format_id: i32 = match config.format {
            AudioFormat::Aac => 1633772320, // kAudioFormatMPEG4AAC
            AudioFormat::Wav => 1819304813, // kAudioFormatLinearPCM
            AudioFormat::Amr => 1935764850, // kAudioFormatAMR (not well supported on iOS)
        };

        let values: [*mut AnyObject; 4] = [
            number_with_int(format_id),
            number_with_float(config.sample_rate as f64),
            number_with_int(config.channels as i32),
            number_with_int(config.bit_rate as i32),
        ];

        let settings: *mut AnyObject = msg_send![class!(NSDictionary),
            dictionaryWithObjects: values.as_ptr(),
            forKeys: keys.as_ptr(),
            count: 4usize
        ];

        // Create AVAudioRecorder
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let recorder: *mut AnyObject = msg_send![class!(AVAudioRecorder), alloc];
        let recorder: *mut AnyObject = msg_send![recorder,
            initWithURL: file_url,
            settings: settings,
            error: &mut error as *mut *mut AnyObject
        ];

        if recorder.is_null() || !error.is_null() {
            let err_msg = if !error.is_null() {
                let desc: *mut AnyObject = msg_send![error, localizedDescription];
                objc_string_to_rust(desc)
            } else {
                "Failed to create recorder".to_string()
            };
            return Err(err_msg);
        }

        // Enable metering for amplitude
        let _: () = msg_send![recorder, setMeteringEnabled: true];

        // Activate audio session
        let session: *mut AnyObject = msg_send![class!(AVAudioSession), sharedInstance];
        let record_category = nsstring("AVAudioSessionCategoryRecord");
        let mut session_error: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![session,
            setCategory: record_category,
            error: &mut session_error as *mut *mut AnyObject
        ];
        let _: () = msg_send![session,
            setActive: true,
            error: &mut session_error as *mut *mut AnyObject
        ];

        // Start recording
        let ok: bool = msg_send![recorder, record];
        if !ok {
            return Err("Failed to start recording".into());
        }

        *guard = Some(RecorderState {
            recorder,
            path: path_str.clone(),
            start_time: std::time::Instant::now(),
        });

        Ok(path_str)
    }
}

pub fn stop_recording() -> Result<Recording, String> {
    let mut guard = STATE.lock().unwrap();
    let state = guard.take().ok_or("Not recording")?;

    unsafe {
        let _: () = msg_send![state.recorder, stop];
        let duration_ms = state.start_time.elapsed().as_millis() as u64;

        // Deactivate audio session
        let session: *mut AnyObject = msg_send![class!(AVAudioSession), sharedInstance];
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![session,
            setActive: false,
            error: &mut error as *mut *mut AnyObject
        ];

        Ok(Recording {
            path: state.path,
            duration_ms,
        })
    }
}

pub fn is_recording() -> bool {
    let guard = STATE.lock().unwrap();
    if let Some(state) = guard.as_ref() {
        unsafe { msg_send![state.recorder, isRecording] }
    } else {
        false
    }
}

pub fn pause_recording() -> Result<(), String> {
    let guard = STATE.lock().unwrap();
    let state = guard.as_ref().ok_or("Not recording")?;
    unsafe {
        let _: () = msg_send![state.recorder, pause];
    }
    Ok(())
}

pub fn resume_recording() -> Result<(), String> {
    let guard = STATE.lock().unwrap();
    let state = guard.as_ref().ok_or("Not recording")?;
    unsafe {
        let ok: bool = msg_send![state.recorder, record];
        if ok {
            Ok(())
        } else {
            Err("Failed to resume recording".into())
        }
    }
}

pub fn get_amplitude() -> Result<f64, String> {
    let guard = STATE.lock().unwrap();
    let state = guard.as_ref().ok_or("Not recording")?;
    unsafe {
        let _: () = msg_send![state.recorder, updateMeters];
        let level: f32 = msg_send![state.recorder, averagePowerForChannel: 0i64];
        // Convert from dB (range: -160 to 0) to linear (0.0 to 1.0)
        let linear = if level <= -160.0 {
            0.0
        } else {
            10.0_f64.powf(level as f64 / 20.0)
        };
        Ok(linear)
    }
}

// Helpers

use crate::ios::util::nsstring;

unsafe fn number_with_int(val: i32) -> *mut AnyObject {
    msg_send![class!(NSNumber), numberWithInt: val]
}

unsafe fn number_with_float(val: f64) -> *mut AnyObject {
    msg_send![class!(NSNumber), numberWithDouble: val]
}

unsafe fn objc_string_to_rust(ns: *mut AnyObject) -> String {
    if ns.is_null() {
        return String::new();
    }
    let cstr: *const std::ffi::c_char = msg_send![ns, UTF8String];
    if cstr.is_null() {
        return String::new();
    }
    std::ffi::CStr::from_ptr(cstr)
        .to_string_lossy()
        .into_owned()
}
