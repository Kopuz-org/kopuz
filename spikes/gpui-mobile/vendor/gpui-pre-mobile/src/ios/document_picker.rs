//! Non-blocking UIKit implementation of GPUI's file prompt.

use anyhow::{anyhow, Result};
use futures::channel::oneshot;
use gpui::PathPromptOptions;
use objc2::{
    class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, AnyProtocol, Bool, ClassBuilder, Sel},
    sel,
};
use std::{cell::RefCell, ffi::CStr, path::PathBuf, sync::OnceLock};

type PickerResult = Result<Option<Vec<PathBuf>>>;

struct PendingPicker {
    sender: oneshot::Sender<PickerResult>,
    // UIKit's delegate property is weak. Keep both objects alive until completion.
    _delegate: Retained<AnyObject>,
    picker: Retained<AnyObject>,
}

thread_local! {
    static PENDING: RefCell<Option<PendingPicker>> = const { RefCell::new(None) };
}

#[link(name = "UniformTypeIdentifiers", kind = "framework")]
extern "C" {}

fn delegate_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut builder = ClassBuilder::new(c"GpuiPathPromptDelegate", class!(NSObject)).unwrap();
        if let Some(protocol) = AnyProtocol::get(c"UIDocumentPickerDelegate") {
            builder.add_protocol(protocol);
        }
        unsafe {
            builder.add_method(
                sel!(documentPicker:didPickDocumentsAtURLs:),
                did_pick
                    as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
            builder.add_method(
                sel!(documentPickerWasCancelled:),
                did_cancel as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }
        builder.register()
    })
}

fn complete(controller: *mut AnyObject, result: PickerResult) {
    let pending = PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        if pending
            .as_ref()
            .is_some_and(|p| Retained::as_ptr(&p.picker) == controller)
        {
            pending.take()
        } else {
            None
        }
    });
    if let Some(pending) = pending {
        let _ = pending.sender.send(result);
    }
}

unsafe extern "C" fn did_pick(
    _: *mut AnyObject,
    _: Sel,
    controller: *mut AnyObject,
    urls: *mut AnyObject,
) {
    let count: usize = msg_send![urls, count];
    let mut paths = Vec::with_capacity(count);
    for index in 0..count {
        let url: *mut AnyObject = msg_send![urls, objectAtIndex: index];
        // `path` decodes URL escapes; stripping file:// from absoluteString does not.
        let path: *mut AnyObject = msg_send![url, path];
        if path.is_null() {
            complete(
                controller,
                Err(anyhow!(
                    "Document picker returned a URL without a file path"
                )),
            );
            return;
        }
        let utf8: *const std::ffi::c_char = msg_send![path, UTF8String];
        if utf8.is_null() {
            complete(
                controller,
                Err(anyhow!("Document picker returned an invalid file path")),
            );
            return;
        }
        paths.push(PathBuf::from(
            CStr::from_ptr(utf8).to_string_lossy().into_owned(),
        ));
    }
    complete(controller, Ok((!paths.is_empty()).then_some(paths)));
}

unsafe extern "C" fn did_cancel(_: *mut AnyObject, _: Sel, controller: *mut AnyObject) {
    complete(controller, Ok(None));
}

pub(super) fn prompt(options: PathPromptOptions) -> oneshot::Receiver<PickerResult> {
    let (sender, receiver) = oneshot::channel();
    // Platform prompts run on GPUI's foreground (UIKit main) thread.
    if PENDING.with(|pending| pending.borrow().is_some()) {
        let _ = sender.send(Err(anyhow!("A document picker is already open")));
        return receiver;
    }
    match unsafe { create_picker(&options) } {
        Ok((picker, delegate, presenter)) => {
            let picker_ptr = Retained::as_ptr(&picker);
            PENDING.with(|pending| {
                *pending.borrow_mut() = Some(PendingPicker {
                    sender,
                    _delegate: delegate,
                    picker,
                });
            });
            unsafe {
                let _: () = msg_send![presenter, presentViewController: picker_ptr,
                    animated: Bool::YES, completion: std::ptr::null::<AnyObject>()];
            }
        }
        Err(error) => {
            let _ = sender.send(Err(error));
        }
    }
    receiver
}

unsafe fn create_picker(
    options: &PathPromptOptions,
) -> Result<(Retained<AnyObject>, Retained<AnyObject>, *mut AnyObject)> {
    if !options.files && !options.directories {
        return Err(anyhow!("The file prompt must allow files or directories"));
    }
    if options.files && options.directories {
        return Err(anyhow!(
            "iOS does not support mixed file and directory selection"
        ));
    }
    let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
    let window: *mut AnyObject = msg_send![app, keyWindow];
    if window.is_null() {
        return Err(anyhow!("No active iOS window to present a document picker"));
    }
    let mut presenter: *mut AnyObject = msg_send![window, rootViewController];
    if presenter.is_null() {
        return Err(anyhow!(
            "No iOS view controller to present a document picker"
        ));
    }
    loop {
        let presented: *mut AnyObject = msg_send![presenter, presentedViewController];
        if presented.is_null() {
            break;
        }
        presenter = presented;
    }
    let identifier = super::util::nsstring(if options.directories {
        "public.folder"
    } else {
        "public.item"
    });
    let content_type: *mut AnyObject = msg_send![class!(UTType), typeWithIdentifier: identifier];
    let types: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: content_type];
    let picker: *mut AnyObject = msg_send![class!(UIDocumentPickerViewController), alloc];
    // Import a sandbox copy so callers can read the path asynchronously without
    // depending on a security-scoped URL's lifetime or a remote file provider.
    let picker: *mut AnyObject =
        msg_send![picker, initForOpeningContentTypes: types, asCopy: Bool::YES];
    let picker =
        Retained::from_raw(picker).ok_or_else(|| anyhow!("Could not create document picker"))?;
    let delegate: *mut AnyObject = msg_send![delegate_class(), new];
    let delegate = Retained::from_raw(delegate)
        .ok_or_else(|| anyhow!("Could not create document picker delegate"))?;
    let _: () = msg_send![&*picker, setDelegate: &*delegate];
    let _: () = msg_send![&*picker, setAllowsMultipleSelection: Bool::from(options.multiple)];
    // Full screen requires explicit cancel, ensuring the receiver always resolves.
    let _: () = msg_send![&*picker, setModalPresentationStyle: 0_isize];
    if let Some(prompt) = &options.prompt {
        let _: () = msg_send![&*picker, setTitle: super::util::nsstring(prompt)];
    }
    Ok((picker, delegate, presenter))
}
