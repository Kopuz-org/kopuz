use super::{
    CameraDescription, CameraHandle, CameraLensDirection, CapturedImage, ExposureMode, FlashMode,
    FocusMode, RecordedVideo, ResolutionPreset,
};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use std::collections::HashMap;
use std::sync::{mpsc, Mutex, Once};

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {}

// ── Session state ───────────────────────────────────────────────────────────

struct SessionState {
    session: *mut AnyObject,       // AVCaptureSession
    device: *mut AnyObject,        // AVCaptureDevice
    device_input: *mut AnyObject,  // AVCaptureDeviceInput
    photo_output: *mut AnyObject,  // AVCapturePhotoOutput
    video_output: *mut AnyObject,  // AVCaptureMovieFileOutput (may be null)
    preview_layer: *mut AnyObject, // AVCaptureVideoPreviewLayer (may be null)
    _enable_audio: bool,
    _audio_input: *mut AnyObject, // AVCaptureDeviceInput for audio (may be null)
}

// SAFETY: all pointers are ObjC objects accessed on the main thread or under lock
unsafe impl Send for SessionState {}

static SESSIONS: Mutex<Option<HashMap<usize, SessionState>>> = Mutex::new(None);
static mut NEXT_ID: usize = 1;

fn sessions() -> std::sync::MutexGuard<'static, Option<HashMap<usize, SessionState>>> {
    let mut guard = SESSIONS.lock().unwrap();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

fn next_id() -> usize {
    unsafe {
        let id = NEXT_ID;
        NEXT_ID += 1;
        id
    }
}

/// Get the raw AVCaptureSession pointer for a given session ID.
pub fn get_session_ptr(id: usize) -> Option<*mut AnyObject> {
    let guard = sessions();
    guard
        .as_ref()
        .and_then(|map| map.get(&id).map(|s| s.session))
}

// ── Photo capture delegate ──────────────────────────────────────────────────

static PHOTO_RESULT: Mutex<Option<mpsc::Sender<Result<CapturedImage, String>>>> = Mutex::new(None);

static REGISTER_PHOTO_DELEGATE: Once = Once::new();
static mut PHOTO_DELEGATE_CLASS: *const AnyClass = std::ptr::null();

fn photo_delegate_class() -> &'static AnyClass {
    REGISTER_PHOTO_DELEGATE.call_once(|| {
        let superclass = class!(NSObject);
        let mut decl = ClassBuilder::new(c"GpuiPhotoCaptureDelegate", superclass).unwrap();

        unsafe {
            // captureOutput:didFinishProcessingPhoto:error:
            decl.add_method(
                sel!(captureOutput:didFinishProcessingPhoto:error:),
                photo_did_finish
                    as extern "C" fn(
                        *mut AnyObject,
                        Sel,
                        *mut AnyObject,
                        *mut AnyObject,
                        *mut AnyObject,
                    ),
            );
        }

        unsafe { PHOTO_DELEGATE_CLASS = decl.register() as *const AnyClass };
    });
    unsafe { &*PHOTO_DELEGATE_CLASS }
}

extern "C" fn photo_did_finish(
    _this: *mut AnyObject,
    _sel: Sel,
    _output: *mut AnyObject,
    photo: *mut AnyObject,
    error: *mut AnyObject,
) {
    let result = unsafe {
        if !error.is_null() {
            let desc: *mut AnyObject = msg_send![error, localizedDescription];
            let cstr: *const std::ffi::c_char = msg_send![desc, UTF8String];
            let msg = if !cstr.is_null() {
                std::ffi::CStr::from_ptr(cstr)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "Unknown capture error".to_string()
            };
            Err(msg)
        } else {
            // Get JPEG data
            let jpeg_data: *mut AnyObject = msg_send![photo, fileDataRepresentation];
            if jpeg_data.is_null() {
                Err("Failed to get photo data".to_string())
            } else {
                // Save to temp file
                let uuid_str = uuid::Uuid::new_v4().to_string();
                let file_name = format!("camera_{}.jpg", uuid_str);
                let file_path = std::env::temp_dir().join(&file_name);
                let ns_path = nsstring(&file_path.to_string_lossy());
                let wrote: Bool = msg_send![jpeg_data, writeToFile: ns_path, atomically: true];

                if wrote.as_bool() {
                    // Get dimensions from CGImage
                    let cg_image: *mut AnyObject = msg_send![photo, CGImageRepresentation];
                    let (width, height) = if !cg_image.is_null() {
                        let _: usize = msg_send![class!(UIImage), imageWithCGImage: cg_image];
                        // Use CGImageGetWidth/Height
                        let w: usize = CGImageGetWidth(cg_image);
                        let h: usize = CGImageGetHeight(cg_image);
                        (w as u32, h as u32)
                    } else {
                        (0, 0)
                    };

                    Ok(CapturedImage {
                        path: file_path.to_string_lossy().into_owned(),
                        width,
                        height,
                    })
                } else {
                    Err("Failed to write photo to file".to_string())
                }
            }
        }
    };

    if let Some(tx) = PHOTO_RESULT.lock().unwrap().take() {
        let _ = tx.send(result);
    }
}

extern "C" {
    fn CGImageGetWidth(image: *mut AnyObject) -> usize;
    fn CGImageGetHeight(image: *mut AnyObject) -> usize;
}

// ── Video recording delegate ────────────────────────────────────────────────

static VIDEO_RESULT: Mutex<Option<mpsc::Sender<Result<RecordedVideo, String>>>> = Mutex::new(None);

static REGISTER_VIDEO_DELEGATE: Once = Once::new();
static mut VIDEO_DELEGATE_CLASS: *const AnyClass = std::ptr::null();

fn video_delegate_class() -> &'static AnyClass {
    REGISTER_VIDEO_DELEGATE.call_once(|| {
        let superclass = class!(NSObject);
        let mut decl = ClassBuilder::new(c"GpuiVideoRecordingDelegate", superclass).unwrap();

        unsafe {
            // captureOutput:didFinishRecordingToOutputFileAtURL:fromConnections:error:
            decl.add_method(
                sel!(captureOutput:didFinishRecordingToOutputFileAtURL:fromConnections:error:),
                video_did_finish
                    as extern "C" fn(
                        *mut AnyObject,
                        Sel,
                        *mut AnyObject,
                        *mut AnyObject,
                        *mut AnyObject,
                        *mut AnyObject,
                    ),
            );
        }

        unsafe { VIDEO_DELEGATE_CLASS = decl.register() as *const AnyClass };
    });
    unsafe { &*VIDEO_DELEGATE_CLASS }
}

extern "C" fn video_did_finish(
    _this: *mut AnyObject,
    _sel: Sel,
    _output: *mut AnyObject,
    url: *mut AnyObject,
    _connections: *mut AnyObject,
    error: *mut AnyObject,
) {
    let result = unsafe {
        if !error.is_null() {
            let desc: *mut AnyObject = msg_send![error, localizedDescription];
            let cstr: *const std::ffi::c_char = msg_send![desc, UTF8String];
            let msg = if !cstr.is_null() {
                std::ffi::CStr::from_ptr(cstr)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "Unknown recording error".to_string()
            };
            Err(msg)
        } else if url.is_null() {
            Err("No output URL".to_string())
        } else {
            let path_obj: *mut AnyObject = msg_send![url, path];
            let cstr: *const std::ffi::c_char = msg_send![path_obj, UTF8String];
            if !cstr.is_null() {
                let path = std::ffi::CStr::from_ptr(cstr)
                    .to_string_lossy()
                    .into_owned();
                Ok(RecordedVideo { path })
            } else {
                Err("Failed to get video path".to_string())
            }
        }
    };

    if let Some(tx) = VIDEO_RESULT.lock().unwrap().take() {
        let _ = tx.send(result);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

use crate::ios::util::nsstring;

fn resolution_to_preset(resolution: ResolutionPreset) -> &'static str {
    match resolution {
        ResolutionPreset::Low => "AVCaptureSessionPresetLow",
        ResolutionPreset::Medium => "AVCaptureSessionPresetMedium",
        ResolutionPreset::High => "AVCaptureSessionPreset1280x720",
        ResolutionPreset::VeryHigh => "AVCaptureSessionPreset1920x1080",
        ResolutionPreset::UltraHigh => "AVCaptureSessionPreset3840x2160",
        ResolutionPreset::Max => "AVCaptureSessionPresetPhoto",
    }
}

unsafe fn find_device_by_name(name: &str) -> *mut AnyObject {
    // Use AVCaptureDeviceDiscoverySession to find all cameras
    let device_types: *mut AnyObject = {
        let wide = nsstring("AVCaptureDeviceTypeBuiltInWideAngleCamera");
        let ultra = nsstring("AVCaptureDeviceTypeBuiltInUltraWideCamera");
        let tele = nsstring("AVCaptureDeviceTypeBuiltInTelephotoCamera");
        msg_send![class!(NSArray),
            arrayWithObjects: [wide, ultra, tele].as_ptr(),
            count: 3usize
        ]
    };

    let media_type = nsstring("vide"); // AVMediaTypeVideo
    let position: i64 = 0; // AVCaptureDevicePositionUnspecified

    let discovery: *mut AnyObject = msg_send![class!(AVCaptureDeviceDiscoverySession),
        discoverySessionWithDeviceTypes: device_types,
        mediaType: media_type,
        position: position
    ];

    if discovery.is_null() {
        return std::ptr::null_mut();
    }

    let devices: *mut AnyObject = msg_send![discovery, devices];
    let count: usize = msg_send![devices, count];

    for i in 0..count {
        let device: *mut AnyObject = msg_send![devices, objectAtIndex: i];
        let unique_id: *mut AnyObject = msg_send![device, uniqueID];
        let cstr: *const std::ffi::c_char = msg_send![unique_id, UTF8String];
        if !cstr.is_null() {
            let id = std::ffi::CStr::from_ptr(cstr).to_string_lossy();
            if id == name {
                return device;
            }
        }
    }

    std::ptr::null_mut()
}

// ── Public API ──────────────────────────────────────────────────────────────

pub fn available_cameras() -> Result<Vec<CameraDescription>, String> {
    unsafe {
        // AVCaptureDeviceType is an NSString typedef, not an ObjC class.
        // Use the string constants directly.
        let device_types: *mut AnyObject = {
            let wide = nsstring("AVCaptureDeviceTypeBuiltInWideAngleCamera");
            let ultra = nsstring("AVCaptureDeviceTypeBuiltInUltraWideCamera");
            let tele = nsstring("AVCaptureDeviceTypeBuiltInTelephotoCamera");
            msg_send![class!(NSArray),
                arrayWithObjects: [wide, ultra, tele].as_ptr(),
                count: 3usize
            ]
        };

        let media_type = nsstring("vide"); // AVMediaTypeVideo
        let position: i64 = 0; // AVCaptureDevicePositionUnspecified

        let discovery: *mut AnyObject = msg_send![class!(AVCaptureDeviceDiscoverySession),
            discoverySessionWithDeviceTypes: device_types,
            mediaType: media_type,
            position: position
        ];

        if discovery.is_null() {
            return Ok(vec![]);
        }

        let devices: *mut AnyObject = msg_send![discovery, devices];
        let count: usize = msg_send![devices, count];
        let mut cameras = Vec::with_capacity(count);

        for i in 0..count {
            let device: *mut AnyObject = msg_send![devices, objectAtIndex: i];

            // Get unique ID
            let unique_id: *mut AnyObject = msg_send![device, uniqueID];
            let cstr: *const std::ffi::c_char = msg_send![unique_id, UTF8String];
            let name = if !cstr.is_null() {
                std::ffi::CStr::from_ptr(cstr)
                    .to_string_lossy()
                    .into_owned()
            } else {
                format!("camera_{i}")
            };

            // Get position -> lens direction
            let position: i64 = msg_send![device, position];
            let lens_direction = match position {
                1 => CameraLensDirection::Back,
                2 => CameraLensDirection::Front,
                _ => CameraLensDirection::External,
            };

            cameras.push(CameraDescription {
                name,
                lens_direction,
                sensor_orientation: 0, // iOS handles rotation automatically
            });
        }

        Ok(cameras)
    }
}

pub fn create_camera(
    camera: &CameraDescription,
    resolution: ResolutionPreset,
    enable_audio: bool,
) -> Result<usize, String> {
    unsafe {
        // Find the device
        let device = find_device_by_name(&camera.name);
        if device.is_null() {
            return Err(format!("Camera '{}' not found", camera.name));
        }

        // Create session
        let session: *mut AnyObject = msg_send![class!(AVCaptureSession), alloc];
        let session: *mut AnyObject = msg_send![session, init];
        if session.is_null() {
            return Err("Failed to create AVCaptureSession".into());
        }

        // Begin configuration
        let _: () = msg_send![session, beginConfiguration];

        // Set resolution preset
        let preset = nsstring(resolution_to_preset(resolution));
        let can_set: Bool = msg_send![session, canSetSessionPreset: preset];
        if can_set.as_bool() {
            let _: () = msg_send![session, setSessionPreset: preset];
        }

        // Add video input
        let mut error_ptr: *mut AnyObject = std::ptr::null_mut();
        let input: *mut AnyObject = msg_send![class!(AVCaptureDeviceInput),
            deviceInputWithDevice: device,
            error: &mut error_ptr
        ];
        if input.is_null() || !error_ptr.is_null() {
            let _: () = msg_send![session, commitConfiguration];
            return Err("Failed to create device input".into());
        }

        let can_add: Bool = msg_send![session, canAddInput: input];
        if !can_add.as_bool() {
            let _: () = msg_send![session, commitConfiguration];
            return Err("Cannot add camera input to session".into());
        }
        let _: () = msg_send![session, addInput: input];

        // Add audio input if requested
        let mut audio_input: *mut AnyObject = std::ptr::null_mut();
        if enable_audio {
            let audio_type = nsstring("soun"); // AVMediaTypeAudio
            let audio_device: *mut AnyObject = msg_send![class!(AVCaptureDevice),
                defaultDeviceWithMediaType: audio_type
            ];
            if !audio_device.is_null() {
                let mut audio_err: *mut AnyObject = std::ptr::null_mut();
                let a_input: *mut AnyObject = msg_send![class!(AVCaptureDeviceInput),
                    deviceInputWithDevice: audio_device,
                    error: &mut audio_err
                ];
                if !a_input.is_null() && audio_err.is_null() {
                    let can_add_audio: Bool = msg_send![session, canAddInput: a_input];
                    if can_add_audio.as_bool() {
                        let _: () = msg_send![session, addInput: a_input];
                        audio_input = a_input;
                    }
                }
            }
        }

        // Add photo output
        let photo_output: *mut AnyObject = msg_send![class!(AVCapturePhotoOutput), alloc];
        let photo_output: *mut AnyObject = msg_send![photo_output, init];
        let can_add_photo: Bool = msg_send![session, canAddOutput: photo_output];
        if !can_add_photo.as_bool() {
            let _: () = msg_send![session, commitConfiguration];
            return Err("Cannot add photo output to session".into());
        }
        let _: () = msg_send![session, addOutput: photo_output];

        // Add video file output
        let video_output: *mut AnyObject = msg_send![class!(AVCaptureMovieFileOutput), alloc];
        let video_output: *mut AnyObject = msg_send![video_output, init];
        let can_add_video: Bool = msg_send![session, canAddOutput: video_output];
        if can_add_video.as_bool() {
            let _: () = msg_send![session, addOutput: video_output];
        } else {
            // Video recording won't be available, but that's okay
            log::warn!("Cannot add video output to session");
        }

        // Commit configuration
        let _: () = msg_send![session, commitConfiguration];

        let id = next_id();
        sessions().as_mut().unwrap().insert(
            id,
            SessionState {
                session,
                device,
                device_input: input,
                photo_output,
                video_output,
                preview_layer: std::ptr::null_mut(),
                _enable_audio: enable_audio,
                _audio_input: audio_input,
            },
        );

        Ok(id)
    }
}

pub fn stop_preview_session(handle: &CameraHandle) -> Result<(), String> {
    unsafe {
        let mut guard = sessions();
        let state = guard
            .as_mut()
            .unwrap()
            .get_mut(&handle.id)
            .ok_or("Invalid camera handle")?;
        let _: () = msg_send![state.session, stopRunning];
        // Preview layer removal is handled by the platform view system
        Ok(())
    }
}

pub fn take_picture(handle: &CameraHandle) -> Result<CapturedImage, String> {
    let (tx, rx) = mpsc::channel();
    *PHOTO_RESULT.lock().unwrap() = Some(tx);

    unsafe {
        let guard = sessions();
        let state = guard
            .as_ref()
            .unwrap()
            .get(&handle.id)
            .ok_or("Invalid camera handle")?;

        // Create photo settings
        let settings: *mut AnyObject = msg_send![class!(AVCapturePhotoSettings), photoSettings];
        if settings.is_null() {
            return Err("Failed to create photo settings".into());
        }

        // Create delegate
        let delegate_cls = photo_delegate_class();
        let delegate: *mut AnyObject = msg_send![delegate_cls, alloc];
        let delegate: *mut AnyObject = msg_send![delegate, init];

        // Capture
        let _: () = msg_send![state.photo_output,
            capturePhotoWithSettings: settings,
            delegate: delegate
        ];
    }

    // Wait for result
    rx.recv()
        .map_err(|_| "Photo capture channel closed".to_string())?
}

pub fn start_video_recording(handle: &CameraHandle) -> Result<(), String> {
    unsafe {
        let guard = sessions();
        let state = guard
            .as_ref()
            .unwrap()
            .get(&handle.id)
            .ok_or("Invalid camera handle")?;

        if state.video_output.is_null() {
            return Err("Video output not available".into());
        }

        let is_recording: Bool = msg_send![state.video_output, isRecording];
        if is_recording.as_bool() {
            return Err("Already recording".into());
        }

        // Create output file URL
        let uuid_str = uuid::Uuid::new_v4().to_string();
        let file_name = format!("video_{}.mov", uuid_str);
        let file_path = std::env::temp_dir().join(&file_name);
        let ns_path = nsstring(&file_path.to_string_lossy());
        let file_url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: ns_path];

        // Create delegate
        let delegate_cls = video_delegate_class();
        let delegate: *mut AnyObject = msg_send![delegate_cls, alloc];
        let delegate: *mut AnyObject = msg_send![delegate, init];

        let _: () = msg_send![state.video_output,
            startRecordingToOutputFileURL: file_url,
            recordingDelegate: delegate
        ];

        Ok(())
    }
}

pub fn stop_video_recording(handle: &CameraHandle) -> Result<RecordedVideo, String> {
    let (tx, rx) = mpsc::channel();
    *VIDEO_RESULT.lock().unwrap() = Some(tx);

    unsafe {
        let guard = sessions();
        let state = guard
            .as_ref()
            .unwrap()
            .get(&handle.id)
            .ok_or("Invalid camera handle")?;

        if state.video_output.is_null() {
            return Err("Video output not available".into());
        }

        let _: () = msg_send![state.video_output, stopRecording];
    }

    rx.recv()
        .map_err(|_| "Video recording channel closed".to_string())?
}

pub fn set_flash_mode(handle: &CameraHandle, mode: FlashMode) -> Result<(), String> {
    unsafe {
        let guard = sessions();
        let state = guard
            .as_ref()
            .unwrap()
            .get(&handle.id)
            .ok_or("Invalid camera handle")?;

        let device = state.device;

        // For torch mode, use torchMode; for flash, it's set on capture settings
        if mode == FlashMode::Torch {
            let has_torch: Bool = msg_send![device, hasTorch];
            if !has_torch.as_bool() {
                return Err("Device does not have a torch".into());
            }
            let mut err: *mut AnyObject = std::ptr::null_mut();
            let locked: Bool =
                msg_send![device, lockForConfiguration: &mut err as *mut *mut AnyObject];
            if !locked.as_bool() {
                return Err("Failed to lock device for configuration".into());
            }
            let _: () = msg_send![device, setTorchMode: 1i64]; // AVCaptureTorchModeOn
            let _: () = msg_send![device, unlockForConfiguration];
        } else {
            // Turn off torch if it was on
            let has_torch: Bool = msg_send![device, hasTorch];
            if has_torch.as_bool() {
                let torch_mode: i64 = msg_send![device, torchMode];
                if torch_mode != 0 {
                    let mut err: *mut AnyObject = std::ptr::null_mut();
                    let locked: Bool =
                        msg_send![device, lockForConfiguration: &mut err as *mut *mut AnyObject];
                    if locked.as_bool() {
                        let _: () = msg_send![device, setTorchMode: 0i64]; // AVCaptureTorchModeOff
                        let _: () = msg_send![device, unlockForConfiguration];
                    }
                }
            }
            // Flash mode is applied per-capture in take_picture via AVCapturePhotoSettings
            // We store it but don't apply to device directly
        }

        Ok(())
    }
}

pub fn set_focus_mode(handle: &CameraHandle, mode: FocusMode) -> Result<(), String> {
    unsafe {
        let guard = sessions();
        let state = guard
            .as_ref()
            .unwrap()
            .get(&handle.id)
            .ok_or("Invalid camera handle")?;

        let device = state.device;
        let avf_mode: i64 = match mode {
            FocusMode::Auto => 2,   // AVCaptureFocusModeContinuousAutoFocus
            FocusMode::Locked => 0, // AVCaptureFocusModeLocked
        };

        let is_supported: Bool = msg_send![device, isFocusModeSupported: avf_mode];
        if !is_supported.as_bool() {
            return Err("Focus mode not supported".into());
        }

        let mut err: *mut AnyObject = std::ptr::null_mut();
        let locked: Bool = msg_send![device, lockForConfiguration: &mut err as *mut *mut AnyObject];
        if !locked.as_bool() {
            return Err("Failed to lock device for configuration".into());
        }

        let _: () = msg_send![device, setFocusMode: avf_mode];
        let _: () = msg_send![device, unlockForConfiguration];

        Ok(())
    }
}

pub fn set_exposure_mode(handle: &CameraHandle, mode: ExposureMode) -> Result<(), String> {
    unsafe {
        let guard = sessions();
        let state = guard
            .as_ref()
            .unwrap()
            .get(&handle.id)
            .ok_or("Invalid camera handle")?;

        let device = state.device;
        let avf_mode: i64 = match mode {
            ExposureMode::Auto => 2,   // AVCaptureExposureModeContinuousAutoExposure
            ExposureMode::Locked => 0, // AVCaptureExposureModeLocked
        };

        let is_supported: Bool = msg_send![device, isExposureModeSupported: avf_mode];
        if !is_supported.as_bool() {
            return Err("Exposure mode not supported".into());
        }

        let mut err: *mut AnyObject = std::ptr::null_mut();
        let locked: Bool = msg_send![device, lockForConfiguration: &mut err as *mut *mut AnyObject];
        if !locked.as_bool() {
            return Err("Failed to lock device for configuration".into());
        }

        let _: () = msg_send![device, setExposureMode: avf_mode];
        let _: () = msg_send![device, unlockForConfiguration];

        Ok(())
    }
}

pub fn get_min_zoom(_handle: &CameraHandle) -> Result<f64, String> {
    // AVCaptureDevice minimum zoom is always 1.0
    Ok(1.0)
}

pub fn get_max_zoom(handle: &CameraHandle) -> Result<f64, String> {
    unsafe {
        let guard = sessions();
        let state = guard
            .as_ref()
            .unwrap()
            .get(&handle.id)
            .ok_or("Invalid camera handle")?;

        let max_zoom: f64 = msg_send![state.device, maxAvailableVideoZoomFactor];
        Ok(max_zoom)
    }
}

pub fn set_zoom(handle: &CameraHandle, zoom: f64) -> Result<(), String> {
    unsafe {
        let guard = sessions();
        let state = guard
            .as_ref()
            .unwrap()
            .get(&handle.id)
            .ok_or("Invalid camera handle")?;

        let device = state.device;
        let max_zoom: f64 = msg_send![device, maxAvailableVideoZoomFactor];
        let clamped = zoom.max(1.0).min(max_zoom);

        let mut err: *mut AnyObject = std::ptr::null_mut();
        let locked: Bool = msg_send![device, lockForConfiguration: &mut err as *mut *mut AnyObject];
        if !locked.as_bool() {
            return Err("Failed to lock device for configuration".into());
        }

        let _: () = msg_send![device, setVideoZoomFactor: clamped];
        let _: () = msg_send![device, unlockForConfiguration];

        Ok(())
    }
}

pub fn set_camera(handle: &CameraHandle, camera: &CameraDescription) -> Result<(), String> {
    unsafe {
        let new_device = find_device_by_name(&camera.name);
        if new_device.is_null() {
            return Err(format!("Camera '{}' not found", camera.name));
        }

        let mut guard = sessions();
        let state = guard
            .as_mut()
            .unwrap()
            .get_mut(&handle.id)
            .ok_or("Invalid camera handle")?;

        let session = state.session;

        let _: () = msg_send![session, beginConfiguration];

        // Remove old input
        let _: () = msg_send![session, removeInput: state.device_input];

        // Create new input
        let mut error_ptr: *mut AnyObject = std::ptr::null_mut();
        let new_input: *mut AnyObject = msg_send![class!(AVCaptureDeviceInput),
            deviceInputWithDevice: new_device,
            error: &mut error_ptr as *mut *mut AnyObject
        ];

        if new_input.is_null() || !error_ptr.is_null() {
            // Rollback: re-add old input
            let _: () = msg_send![session, addInput: state.device_input];
            let _: () = msg_send![session, commitConfiguration];
            return Err("Failed to create input for new camera".into());
        }

        let can_add: Bool = msg_send![session, canAddInput: new_input];
        if !can_add.as_bool() {
            let _: () = msg_send![session, addInput: state.device_input];
            let _: () = msg_send![session, commitConfiguration];
            return Err("Cannot add new camera input".into());
        }

        let _: () = msg_send![session, addInput: new_input];

        state.device = new_device;
        state.device_input = new_input;

        let _: () = msg_send![session, commitConfiguration];

        Ok(())
    }
}

pub fn dispose(handle: CameraHandle) -> Result<(), String> {
    let state = {
        let mut guard = sessions();
        guard.as_mut().unwrap().remove(&handle.id)
    };

    if let Some(state) = state {
        unsafe {
            let _: () = msg_send![state.session, stopRunning];

            if !state.preview_layer.is_null() {
                let _: () = msg_send![state.preview_layer, removeFromSuperlayer];
            }
        }
    }

    Ok(())
}
