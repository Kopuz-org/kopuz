use super::{CameraDevice, ImagePickerOptions, ImageSource, PickedFile};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use std::sync::{mpsc, Mutex, Once};

#[link(name = "PhotosUI", kind = "framework")]
extern "C" {}

#[link(name = "Photos", kind = "framework")]
extern "C" {}

#[link(name = "UIKit", kind = "framework")]
extern "C" {}

// ── Result channel ──────────────────────────────────────────────────────────

static RESULT_TX: Mutex<Option<mpsc::Sender<Vec<String>>>> = Mutex::new(None);

fn set_result_channel() -> mpsc::Receiver<Vec<String>> {
    let (tx, rx) = mpsc::channel();
    *RESULT_TX.lock().unwrap() = Some(tx);
    rx
}

fn send_result(paths: Vec<String>) {
    if let Some(tx) = RESULT_TX.lock().unwrap().take() {
        let _ = tx.send(paths);
    }
}

// ── PHPicker delegate (iOS 14+ gallery picker) ─────────────────────────────

static REGISTER_PHPICKER_DELEGATE: Once = Once::new();
static mut PHPICKER_DELEGATE_CLASS: *const AnyClass = std::ptr::null();

fn phpicker_delegate_class() -> &'static AnyClass {
    REGISTER_PHPICKER_DELEGATE.call_once(|| {
        let superclass = class!(NSObject);
        let mut decl = ClassBuilder::new(c"GpuiPHPickerDelegate", superclass).unwrap();

        unsafe {
            decl.add_method(
                sel!(picker:didFinishPicking:),
                phpicker_did_finish
                    as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
        }

        unsafe { PHPICKER_DELEGATE_CLASS = decl.register() as *const AnyClass };
    });
    unsafe { &*PHPICKER_DELEGATE_CLASS }
}

extern "C" fn phpicker_did_finish(
    _this: *mut AnyObject,
    _sel: Sel,
    picker: *mut AnyObject,
    results: *mut AnyObject,
) {
    unsafe {
        // Dismiss the picker
        let _: () = msg_send![picker, dismissViewControllerAnimated: true, completion: std::ptr::null::<AnyObject>()];

        let count: usize = msg_send![results, count];
        if count == 0 {
            send_result(vec![]);
            return;
        }

        // Process results asynchronously — save each to temp dir
        let tmp_dir = std::env::temp_dir();
        let mut paths = Vec::with_capacity(count);

        for i in 0..count {
            let result: *mut AnyObject = msg_send![results, objectAtIndex: i];
            let item_provider: *mut AnyObject = msg_send![result, itemProvider];

            // Check if it can load as UIImage
            let image_class = class!(UIImage);
            let can_load: Bool = msg_send![item_provider, canLoadObjectOfClass: image_class];

            if can_load.as_bool() {
                // Use a synchronous approach: save to temp file
                let uuid_str = uuid::Uuid::new_v4().to_string();
                let file_name = format!("picked_image_{}.jpg", uuid_str);
                let file_path = tmp_dir.join(&file_name);
                let file_path_str = file_path.to_string_lossy().to_string();

                // Create a latch for this item
                let (item_tx, item_rx) = mpsc::channel::<Option<String>>();

                // Load the file representation
                let ns_uti = nsstring("public.image");
                let path_copy = file_path_str.clone();

                // Use loadFileRepresentationForTypeIdentifier to get a temp file URL
                let block = block2::RcBlock::new(
                    move |url: *mut AnyObject, _error: *mut AnyObject| {
                        if url.is_null() {
                            let _ = item_tx.send(None);
                            return;
                        }
                        // Copy the file to our temp directory
                        let file_mgr: *mut AnyObject =
                            msg_send![class!(NSFileManager), defaultManager];
                        let dest_url: *mut AnyObject =
                            msg_send![class!(NSURL), fileURLWithPath: nsstring(&path_copy)];
                        let ok: Bool = msg_send![file_mgr, copyItemAtURL: url, toURL: dest_url, error: std::ptr::null_mut::<*mut AnyObject>()];
                        if ok.as_bool() {
                            let _ = item_tx.send(Some(path_copy.clone()));
                        } else {
                            let _ = item_tx.send(None);
                        }
                    },
                );

                let _: *mut AnyObject = msg_send![item_provider,
                    loadFileRepresentationForTypeIdentifier: ns_uti,
                    completionHandler: &*block
                ];

                if let Ok(Some(path)) = item_rx.recv() {
                    paths.push(path);
                }
            }
        }

        send_result(paths);
    }
}

// ── UIImagePicker delegate (camera capture) ─────────────────────────────────

static REGISTER_UIPICKER_DELEGATE: Once = Once::new();
static mut UIPICKER_DELEGATE_CLASS: *const AnyClass = std::ptr::null();

fn uipicker_delegate_class() -> &'static AnyClass {
    REGISTER_UIPICKER_DELEGATE.call_once(|| {
        let superclass = class!(NSObject);
        let mut decl = ClassBuilder::new(c"GpuiUIImagePickerDelegate", superclass).unwrap();

        unsafe {
            decl.add_method(
                sel!(imagePickerController:didFinishPickingMediaWithInfo:),
                uipicker_did_finish
                    as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
            decl.add_method(
                sel!(imagePickerControllerDidCancel:),
                uipicker_did_cancel as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }

        unsafe { UIPICKER_DELEGATE_CLASS = decl.register() as *const AnyClass };
    });
    unsafe { &*UIPICKER_DELEGATE_CLASS }
}

unsafe extern "C" fn uipicker_did_finish(
    _this: *mut AnyObject,
    _sel: Sel,
    controller: *mut AnyObject,
    info: *mut AnyObject,
) {
    let _: () = msg_send![controller, dismissViewControllerAnimated: true, completion: std::ptr::null::<AnyObject>()];

    // Try to get the image URL first (for videos or saved photos)
    let media_url_key = nsstring("UIImagePickerControllerMediaURL");
    let media_url: *mut AnyObject = msg_send![info, objectForKey: media_url_key];

    if !media_url.is_null() {
        let abs_string: *mut AnyObject = msg_send![media_url, absoluteString];
        let cstr: *const std::ffi::c_char = msg_send![abs_string, UTF8String];
        if !cstr.is_null() {
            let path = std::ffi::CStr::from_ptr(cstr)
                .to_string_lossy()
                .into_owned();
            let path = path.strip_prefix("file://").unwrap_or(&path).to_string();
            send_result(vec![path]);
            return;
        }
    }

    // Fall back to getting the UIImage and saving it
    let image_key = nsstring("UIImagePickerControllerOriginalImage");
    let image: *mut AnyObject = msg_send![info, objectForKey: image_key];

    if !image.is_null() {
        // Convert to JPEG data
        // Use UIImageJPEGRepresentation C function
        let jpeg_data = uiimage_jpeg_representation(image, 0.9);
        if !jpeg_data.is_null() {
            let uuid_str = uuid::Uuid::new_v4().to_string();
            let file_name = format!("captured_{}.jpg", uuid_str);
            let file_path = std::env::temp_dir().join(file_name);
            let ns_path = nsstring(&file_path.to_string_lossy());
            let wrote: Bool = msg_send![jpeg_data, writeToFile: ns_path, atomically: true];
            if wrote.as_bool() {
                send_result(vec![file_path.to_string_lossy().into_owned()]);
                return;
            }
        }
    }

    send_result(vec![]);
}

unsafe extern "C" fn uipicker_did_cancel(
    _this: *mut AnyObject,
    _sel: Sel,
    controller: *mut AnyObject,
) {
    let _: () = msg_send![controller, dismissViewControllerAnimated: true, completion: std::ptr::null::<AnyObject>()];
    send_result(vec![]);
}

// UIImageJPEGRepresentation is a C function, not an ObjC method
extern "C" {
    fn UIImageJPEGRepresentation(image: *mut AnyObject, quality: f64) -> *mut AnyObject;
}

unsafe fn uiimage_jpeg_representation(image: *mut AnyObject, quality: f64) -> *mut AnyObject {
    UIImageJPEGRepresentation(image, quality)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

use crate::ios::util::nsstring;

unsafe fn present_vc(vc: *mut AnyObject) -> Result<(), String> {
    let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
    let key_window: *mut AnyObject = msg_send![app, keyWindow];
    if key_window.is_null() {
        return Err("No key window available".into());
    }
    let root_vc: *mut AnyObject = msg_send![key_window, rootViewController];
    if root_vc.is_null() {
        return Err("No root view controller".into());
    }
    let _: () = msg_send![root_vc,
        presentViewController: vc,
        animated: true,
        completion: std::ptr::null::<AnyObject>()
    ];
    Ok(())
}

fn path_to_picked_file(path: &str) -> PickedFile {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    PickedFile {
        path: path.to_string(),
        name,
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

pub fn pick_image(options: &ImagePickerOptions) -> Result<Option<PickedFile>, String> {
    match options.source {
        ImageSource::Gallery => {
            pick_image_from_gallery(options, false).map(|v| v.into_iter().next())
        }
        ImageSource::Camera => pick_from_camera(options, false),
    }
}

pub fn pick_multi_image(
    _max_width: Option<f64>,
    _max_height: Option<f64>,
    _image_quality: Option<u8>,
) -> Result<Vec<PickedFile>, String> {
    let options = ImagePickerOptions {
        source: ImageSource::Gallery,
        max_width: _max_width,
        max_height: _max_height,
        image_quality: _image_quality,
        preferred_camera: CameraDevice::Rear,
    };
    pick_image_from_gallery(&options, true)
}

pub fn pick_video(
    source: ImageSource,
    preferred_camera: CameraDevice,
) -> Result<Option<PickedFile>, String> {
    let options = ImagePickerOptions {
        source,
        max_width: None,
        max_height: None,
        image_quality: None,
        preferred_camera,
    };
    match source {
        ImageSource::Gallery => pick_video_from_gallery(),
        ImageSource::Camera => pick_from_camera(&options, true),
    }
}

fn pick_image_from_gallery(
    _options: &ImagePickerOptions,
    allow_multi: bool,
) -> Result<Vec<PickedFile>, String> {
    let rx = set_result_channel();

    unsafe {
        // PHPickerConfiguration
        let config: *mut AnyObject = msg_send![class!(PHPickerConfiguration), alloc];
        let config: *mut AnyObject = msg_send![config, init];

        if allow_multi {
            let _: () = msg_send![config, setSelectionLimit: 0i64]; // 0 = unlimited
        } else {
            let _: () = msg_send![config, setSelectionLimit: 1i64];
        }

        // Filter to images only
        let image_filter: *mut AnyObject = msg_send![class!(PHPickerFilter), imagesFilter];
        let _: () = msg_send![config, setFilter: image_filter];

        // Create PHPickerViewController
        let picker: *mut AnyObject = msg_send![class!(PHPickerViewController), alloc];
        let picker: *mut AnyObject = msg_send![picker, initWithConfiguration: config];
        if picker.is_null() {
            return Err("Failed to create PHPickerViewController".into());
        }

        // Set delegate
        let delegate_cls = phpicker_delegate_class();
        let delegate: *mut AnyObject = msg_send![delegate_cls, alloc];
        let delegate: *mut AnyObject = msg_send![delegate, init];
        let _: () = msg_send![picker, setDelegate: delegate];

        present_vc(picker)?;
    }

    match rx.recv() {
        Ok(paths) => Ok(paths.iter().map(|p| path_to_picked_file(p)).collect()),
        Err(_) => Ok(vec![]),
    }
}

fn pick_video_from_gallery() -> Result<Option<PickedFile>, String> {
    let rx = set_result_channel();

    unsafe {
        // PHPickerConfiguration
        let config: *mut AnyObject = msg_send![class!(PHPickerConfiguration), alloc];
        let config: *mut AnyObject = msg_send![config, init];
        let _: () = msg_send![config, setSelectionLimit: 1i64];

        // Filter to videos only
        let video_filter: *mut AnyObject = msg_send![class!(PHPickerFilter), videosFilter];
        let _: () = msg_send![config, setFilter: video_filter];

        let picker: *mut AnyObject = msg_send![class!(PHPickerViewController), alloc];
        let picker: *mut AnyObject = msg_send![picker, initWithConfiguration: config];
        if picker.is_null() {
            return Err("Failed to create PHPickerViewController".into());
        }

        let delegate_cls = phpicker_delegate_class();
        let delegate: *mut AnyObject = msg_send![delegate_cls, alloc];
        let delegate: *mut AnyObject = msg_send![delegate, init];
        let _: () = msg_send![picker, setDelegate: delegate];

        present_vc(picker)?;
    }

    match rx.recv() {
        Ok(paths) if paths.is_empty() => Ok(None),
        Ok(paths) => Ok(Some(path_to_picked_file(&paths[0]))),
        Err(_) => Ok(None),
    }
}

fn pick_from_camera(
    options: &ImagePickerOptions,
    video: bool,
) -> Result<Option<PickedFile>, String> {
    let rx = set_result_channel();

    unsafe {
        // Check if camera is available
        let source_type: i64 = 1; // UIImagePickerControllerSourceTypeCamera
        let available: Bool =
            msg_send![class!(UIImagePickerController), isSourceTypeAvailable: source_type];
        if !available.as_bool() {
            return Err("Camera is not available on this device".into());
        }

        let picker: *mut AnyObject = msg_send![class!(UIImagePickerController), alloc];
        let picker: *mut AnyObject = msg_send![picker, init];
        if picker.is_null() {
            return Err("Failed to create UIImagePickerController".into());
        }

        let _: () = msg_send![picker, setSourceType: source_type];

        // Set media types
        if video {
            let video_type = nsstring("public.movie");
            let types: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: video_type];
            let _: () = msg_send![picker, setMediaTypes: types];
        }

        // Set camera device
        let camera_device: i64 = match options.preferred_camera {
            CameraDevice::Rear => 0,  // UIImagePickerControllerCameraDeviceRear
            CameraDevice::Front => 1, // UIImagePickerControllerCameraDeviceFront
        };
        let _: () = msg_send![picker, setCameraDevice: camera_device];

        // Set delegate
        let delegate_cls = uipicker_delegate_class();
        let delegate: *mut AnyObject = msg_send![delegate_cls, alloc];
        let delegate: *mut AnyObject = msg_send![delegate, init];
        let _: () = msg_send![picker, setDelegate: delegate];

        present_vc(picker)?;
    }

    match rx.recv() {
        Ok(paths) if paths.is_empty() => Ok(None),
        Ok(paths) => Ok(Some(path_to_picked_file(&paths[0]))),
        Err(_) => Ok(None),
    }
}
