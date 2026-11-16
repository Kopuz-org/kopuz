//! iOS text input handling.
//!
//! This module provides keyboard input support for iOS.
//! The window uses a native UITextView as its composition buffer so UIKit
//! supplies the full UITextInput protocol, including multistage Chinese IME.

use gpui::{KeyDownEvent, Keystroke, Modifiers, PlatformInput};

/// Configure the public UITextInputTraits properties on the native text view.
///
/// UIKit can implement these selectors through Objective-C forwarding rather
/// than entries in UITextView's method table. objc2's debug msg_send! verifier
/// rejects those valid messages before the runtime can forward them. Restrict
/// direct dispatch to these three known setters with their exact ABI:
/// void (id, SEL, NSInteger).
///
/// # Safety
/// `view` must be a live UITextView, accessed on the UIKit main thread.
pub(super) unsafe fn configure_keyboard_traits(
    view: *mut objc2::runtime::AnyObject,
    keyboard_type: isize,
) {
    let send: unsafe extern "C-unwind" fn(
        *mut objc2::runtime::AnyObject,
        objc2::runtime::Sel,
        isize,
    ) = unsafe { std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn()) };
    unsafe {
        send(view, objc2::sel!(setKeyboardType:), keyboard_type);
        send(view, objc2::sel!(setAutocorrectionType:), 1);
        send(view, objc2::sel!(setAutocapitalizationType:), 0);
    }
}

/// NSRange uses UTF-16 code units, matching GPUI's platform input handler.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct ObjcNSRange {
    pub location: usize,
    pub length: usize,
}

unsafe impl objc2::encode::Encode for ObjcNSRange {
    const ENCODING: objc2::encode::Encoding = objc2::encode::Encoding::Struct(
        "_NSRange",
        &[
            <usize as objc2::encode::Encode>::ENCODING,
            <usize as objc2::encode::Encode>::ENCODING,
        ],
    );
}

/// Convert a key code from UIKeyboardHIDUsage to a GPUI key string.
///
/// UIKeyboardHIDUsage values are based on the USB HID specification.
pub fn key_code_to_string(code: u32) -> String {
    match code {
        // Letters (0x04-0x1D = a-z)
        0x04..=0x1D => {
            let letter = (b'a' + (code - 0x04) as u8) as char;
            letter.to_string()
        }
        // Numbers (0x1E-0x27 = 1-9, 0)
        0x1E..=0x26 => {
            let num = ((code - 0x1E + 1) % 10) as u8 + b'0';
            (num as char).to_string()
        }
        0x27 => "0".to_string(),
        // Special keys
        0x28 => "enter".to_string(),
        0x29 => "escape".to_string(),
        0x2A => "backspace".to_string(),
        0x2B => "tab".to_string(),
        0x2C => " ".to_string(),
        0x2D => "-".to_string(),
        0x2E => "=".to_string(),
        0x2F => "[".to_string(),
        0x30 => "]".to_string(),
        0x31 => "\\".to_string(),
        0x33 => ";".to_string(),
        0x34 => "'".to_string(),
        0x35 => "`".to_string(),
        0x36 => ",".to_string(),
        0x37 => ".".to_string(),
        0x38 => "/".to_string(),
        // Arrow keys
        0x4F => "right".to_string(),
        0x50 => "left".to_string(),
        0x51 => "down".to_string(),
        0x52 => "up".to_string(),
        // Function keys
        0x3A => "f1".to_string(),
        0x3B => "f2".to_string(),
        0x3C => "f3".to_string(),
        0x3D => "f4".to_string(),
        0x3E => "f5".to_string(),
        0x3F => "f6".to_string(),
        0x40 => "f7".to_string(),
        0x41 => "f8".to_string(),
        0x42 => "f9".to_string(),
        0x43 => "f10".to_string(),
        0x44 => "f11".to_string(),
        0x45 => "f12".to_string(),
        // Other special keys
        0x49 => "insert".to_string(),
        0x4A => "home".to_string(),
        0x4B => "pageup".to_string(),
        0x4C => "delete".to_string(),
        0x4D => "end".to_string(),
        0x4E => "pagedown".to_string(),
        // Default
        _ => format!("unknown-{:02x}", code),
    }
}

/// Convert UIKeyModifierFlags to GPUI Modifiers.
///
/// UIKeyModifierFlags:
/// - alphaShift (caps lock): 1 << 16
/// - shift: 1 << 17
/// - control: 1 << 18
/// - alternate (option): 1 << 19
/// - command: 1 << 20
/// - numericPad: 1 << 21
pub fn modifier_flags_to_modifiers(flags: u32) -> Modifiers {
    const SHIFT: u32 = 1 << 17;
    const CONTROL: u32 = 1 << 18;
    const ALT: u32 = 1 << 19;
    const COMMAND: u32 = 1 << 20;

    Modifiers {
        control: flags & CONTROL != 0,
        alt: flags & ALT != 0,
        shift: flags & SHIFT != 0,
        platform: flags & COMMAND != 0,
        function: false,
    }
}

/// Create a key down event from a key code and modifiers.
pub fn key_code_to_key_down(key_code: u32, modifier_flags: u32) -> PlatformInput {
    let modifiers = modifier_flags_to_modifiers(modifier_flags);
    let key = key_code_to_string(key_code);

    let keystroke = Keystroke {
        modifiers,
        key: key.clone(),
        key_char: if key.len() == 1 {
            Some(key.clone())
        } else {
            None
        },
    };

    PlatformInput::KeyDown(KeyDownEvent {
        keystroke,
        is_held: false,
        prefer_character_input: false,
    })
}

/// Create a key up event from a key code and modifiers.
pub fn key_code_to_key_up(key_code: u32, modifier_flags: u32) -> PlatformInput {
    let modifiers = modifier_flags_to_modifiers(modifier_flags);
    let key = key_code_to_string(key_code);

    let keystroke = Keystroke {
        modifiers,
        key: key.clone(),
        key_char: if key.len() == 1 {
            Some(key.clone())
        } else {
            None
        },
    };

    PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke })
}
