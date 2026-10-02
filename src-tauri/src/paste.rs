//! Inserts text into the focused app: clipboard + simulated paste shortcut, then puts the
//! user's previous clipboard back (all formats on macOS: images, files, rich text).

use anyhow::Result;
use std::thread::sleep;
use std::time::Duration;

/// Returns as soon as the paste keystroke is sent; the clipboard is restored in the background.
pub fn insert(text: &str, restore_clipboard: bool) -> Result<()> {
    let snapshot = if restore_clipboard { clipboard::snapshot() } else { None };
    let marker = clipboard::set_text(text)?;
    sleep(Duration::from_millis(40));
    send_paste_shortcut();
    if let Some(snapshot) = snapshot {
        std::thread::spawn(move || {
            // Give the target app time to read the clipboard before restoring it.
            sleep(Duration::from_millis(400));
            clipboard::restore(snapshot, marker);
        });
    }
    Ok(())
}

/// Puts text on the clipboard without pasting (tray "Copy Last Dictation").
pub fn copy(text: &str) -> Result<()> {
    clipboard::set_text(text).map(|_| ())
}

#[cfg(target_os = "macos")]
mod clipboard {
    use anyhow::{anyhow, Result};
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData, NSString};

    /// Every item on the pasteboard, each as (type identifier, raw bytes) pairs.
    pub struct Snapshot(Vec<Vec<(String, Vec<u8>)>>);

    pub fn snapshot() -> Option<Snapshot> {
        let pb = NSPasteboard::generalPasteboard();
        let items = pb.pasteboardItems()?;
        let saved = items
            .iter()
            .map(|item| {
                item.types()
                    .iter()
                    .filter_map(|t| item.dataForType(&t).map(|d| (t.to_string(), d.to_vec())))
                    .collect::<Vec<_>>()
            })
            .filter(|entries| !entries.is_empty())
            .collect();
        Some(Snapshot(saved))
    }

    /// Returns the pasteboard change count after writing, used to detect later changes.
    pub fn set_text(text: &str) -> Result<isize> {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        if !pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString }) {
            return Err(anyhow!("could not write to the clipboard"));
        }
        Ok(pb.changeCount())
    }

    /// Restores the snapshot unless something else changed the clipboard since we wrote it.
    pub fn restore(snapshot: Snapshot, marker: isize) {
        let pb = NSPasteboard::generalPasteboard();
        if pb.changeCount() != marker {
            return;
        }
        pb.clearContents();
        if snapshot.0.is_empty() {
            return;
        }
        let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snapshot
            .0
            .iter()
            .map(|entries| {
                let item = NSPasteboardItem::new();
                for (kind, bytes) in entries {
                    item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(kind));
                }
                ProtocolObject::from_retained(item)
            })
            .collect();
        pb.writeObjects(&NSArray::from_retained_slice(&items));
    }
}

#[cfg(not(target_os = "macos"))]
mod clipboard {
    use anyhow::Result;

    /// Text only on other platforms.
    pub struct Snapshot(Option<String>);

    pub fn snapshot() -> Option<Snapshot> {
        let mut cb = arboard::Clipboard::new().ok()?;
        Some(Snapshot(cb.get_text().ok()))
    }

    pub fn set_text(text: &str) -> Result<isize> {
        arboard::Clipboard::new()?.set_text(text)?;
        Ok(0)
    }

    pub fn restore(snapshot: Snapshot, _marker: isize) {
        if let (Some(prev), Ok(mut cb)) = (snapshot.0, arboard::Clipboard::new()) {
            let _ = cb.set_text(prev);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    use std::ffi::c_void;

    type Ptr = *mut c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceCreate(state: i32) -> Ptr;
        fn CGEventCreateKeyboardEvent(source: Ptr, keycode: u16, down: bool) -> Ptr;
        fn CGEventSetFlags(event: Ptr, flags: u64);
        fn CGEventPost(tap: u32, event: Ptr);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    }

    const HID_SYSTEM_STATE: i32 = 1;
    const HID_EVENT_TAP: u32 = 0;
    const FLAG_COMMAND: u64 = 0x0010_0000;
    const KEY_V: u16 = 9;

    pub fn send_paste_shortcut() {
        unsafe {
            let source = CGEventSourceCreate(HID_SYSTEM_STATE);
            for down in [true, false] {
                let ev = CGEventCreateKeyboardEvent(source, KEY_V, down);
                CGEventSetFlags(ev, FLAG_COMMAND);
                CGEventPost(HID_EVENT_TAP, ev);
                CFRelease(ev);
            }
            if !source.is_null() {
                CFRelease(source);
            }
        }
    }

    /// Accessibility permission is needed to post the ⌘V keystroke.
    pub fn has_permission(prompt: bool) -> bool {
        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let value = if prompt { CFBoolean::true_value() } else { CFBoolean::false_value() };
        let options = CFDictionary::from_CFType_pairs(&[(key, value)]);
        unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef() as *const c_void) }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use rdev::{simulate, EventType, Key};
    use std::thread::sleep;
    use std::time::Duration;

    pub fn send_paste_shortcut() {
        for ev in [
            EventType::KeyPress(Key::ControlLeft),
            EventType::KeyPress(Key::KeyV),
            EventType::KeyRelease(Key::KeyV),
            EventType::KeyRelease(Key::ControlLeft),
        ] {
            let _ = simulate(&ev);
            sleep(Duration::from_millis(8));
        }
    }

    pub fn has_permission(_prompt: bool) -> bool {
        true
    }
}

pub use platform::has_permission;
use platform::send_paste_shortcut;
