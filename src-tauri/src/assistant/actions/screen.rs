//! The few things done to apps directly: finding a running app, listing the open ones, pressing
//! keyboard shortcuts (a browser's new tab, back, reload) in the app in front, and reading the
//! text in the front window (a screenshot of it, read by Apple's on-device text recognition).

use super::*;
use std::ffi::c_void;

type Ref = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> Ref;
    // Same signatures as in `paste`, which declares these too.
    fn CGEventCreateKeyboardEvent(source: *mut c_void, keycode: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFDictionaryGetValue(dict: Ref, key: Ref) -> Ref;
    fn CFNumberGetValue(number: Ref, kind: isize, out: *mut c_void) -> bool;
    fn CFRelease(cf: Ref);
}

/// The pid of a running app by name ("Slack", "chrome").
pub(super) fn app_pid(name: &str) -> Option<(i32, String)> {
    use objc2_app_kit::NSWorkspace;
    let want = name.to_lowercase();
    let apps = NSWorkspace::sharedWorkspace().runningApplications();
    let named: Vec<(i32, String)> = apps.iter().filter_map(|a| Some((a.processIdentifier(), a.localizedName()?.to_string()))).collect();
    named
        .iter()
        .find(|(_, n)| n.to_lowercase() == want)
        .or_else(|| named.iter().find(|(_, n)| n.to_lowercase().contains(&want)))
        .cloned()
}

/// The name of the app the user is working in, read live (see `front_window`).
pub(super) fn front_app_name() -> Option<String> {
    let pid = crate::focus::active_pid()?;
    objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?.localizedName().map(|n| n.to_string())
}

/// Apps with windows (not background helpers).
pub(super) fn running_apps() -> Result<String> {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
    let workspace = NSWorkspace::sharedWorkspace();
    let mut names: Vec<String> = workspace
        .runningApplications()
        .iter()
        .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .filter_map(|a| a.localizedName().map(|n| n.to_string()))
        .filter(|n| n != "Murmur" && n != "Finder")
        .collect();
    names.sort();
    names.dedup();
    let front = front_app_name().unwrap_or_default();
    Ok(format!("In front: {front}. Open apps: {}", names.join(", ")))
}

/// Presses a key combination like "cmd+t" or "return" in `app`, which the caller has already
/// brought to the front.
pub(super) fn keys(combo: &str, app: &str) -> Result<String> {
    for part in combo.split([',', ' ']).filter(|p| !p.is_empty()) {
        press(part)?;
        std::thread::sleep(Duration::from_millis(60));
    }
    Ok(format!("Pressed {combo} in {app}"))
}

const CMD: u64 = 0x10_0000;
const SHIFT: u64 = 0x2_0000;
const ALT: u64 = 0x8_0000;
const CTRL: u64 = 0x4_0000;

/// macOS virtual key codes (US layout positions).
fn keycode(key: &str) -> Option<u16> {
    Some(match key {
        "a" => 0, "s" => 1, "d" => 2, "f" => 3, "h" => 4, "g" => 5, "z" => 6, "x" => 7, "c" => 8, "v" => 9,
        "b" => 11, "q" => 12, "w" => 13, "e" => 14, "r" => 15, "y" => 16, "t" => 17, "1" => 18, "2" => 19,
        "3" => 20, "4" => 21, "6" => 22, "5" => 23, "=" | "plus" | "equals" => 24, "9" => 25, "7" => 26,
        "-" | "minus" => 27, "8" => 28, "0" => 29, "]" => 30, "o" => 31, "u" => 32, "[" => 33, "i" => 34,
        "p" => 35, "return" | "enter" => 36, "l" => 37, "j" => 38, "'" => 39, "k" => 40, ";" => 41,
        "\\" => 42, "," | "comma" => 43, "/" | "slash" => 44, "n" => 45, "m" => 46, "." | "period" => 47,
        "tab" => 48, "space" => 49, "`" => 50, "delete" | "backspace" => 51, "escape" | "esc" => 53,
        "f5" => 96, "f6" => 97, "f7" => 98, "f3" => 99, "f8" => 100, "f9" => 101, "f11" => 103,
        "f10" => 109, "f12" => 111, "home" => 115, "pageup" | "page_up" => 116,
        "forwarddelete" | "forward_delete" | "del" => 117, "f4" => 118, "end" => 119, "f2" => 120,
        "pagedown" | "page_down" => 121, "f1" => 122, "left" => 123, "right" => 124, "down" => 125,
        "up" => 126,
        _ => return None,
    })
}

/// Presses one combination like "cmd+shift+t".
fn press(combo: &str) -> Result<()> {
    let combo = combo.to_lowercase();
    let mut flags = 0u64;
    let mut key = None;
    for part in combo.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part {
            "cmd" | "command" | "⌘" => flags |= CMD,
            "shift" | "⇧" => flags |= SHIFT,
            "alt" | "option" | "opt" | "⌥" => flags |= ALT,
            "ctrl" | "control" | "⌃" => flags |= CTRL,
            k => key = Some(keycode(k).ok_or_else(|| anyhow!("Unknown key {k}"))?),
        }
    }
    let key = key.ok_or_else(|| anyhow!("No key in {combo}"))?;
    unsafe {
        for down in [true, false] {
            let ev = CGEventCreateKeyboardEvent(std::ptr::null_mut(), key, down);
            if ev.is_null() {
                bail!("Couldn't press keys");
            }
            CGEventSetFlags(ev, flags);
            CGEventPost(0, ev);
            CFRelease(ev);
        }
    }
    Ok(())
}

// --- Reading the screen ----------------------------------------------------------------------

/// The window the user is looking at: its number (for `screencapture`), app and title. The
/// window list runs front to back; Murmur's own windows and the system's are skipped.
fn front_window() -> Result<(u32, String, String)> {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    const ON_SCREEN_ONLY: u32 = 1 << 0;
    const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const NUMBER_SINT32: isize = 3;
    let me = std::process::id() as i32;
    let key = |k: &'static str| CFString::from_static_string(k);
    let (k_pid, k_layer, k_number, k_owner, k_title) =
        (key("kCGWindowOwnerPID"), key("kCGWindowLayer"), key("kCGWindowNumber"), key("kCGWindowOwnerName"), key("kCGWindowName"));
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP_ELEMENTS, 0);
        if list.is_null() {
            bail!("Couldn't see which window is in front");
        }
        let mut found = None;
        for i in 0..CFArrayGetCount(list) {
            let info = CFArrayGetValueAtIndex(list, i);
            let int = |k: &CFString| {
                let n = CFDictionaryGetValue(info, k.as_CFTypeRef() as Ref);
                let mut v = 0i32;
                (!n.is_null() && CFNumberGetValue(n, NUMBER_SINT32, &mut v as *mut i32 as *mut c_void)).then_some(v)
            };
            let text = |k: &CFString| {
                let v = CFDictionaryGetValue(info, k.as_CFTypeRef() as Ref);
                if v.is_null() { String::new() } else { CFString::wrap_under_get_rule(v as CFStringRef).to_string() }
            };
            let (Some(pid), Some(0), Some(number)) = (int(&k_pid), int(&k_layer), int(&k_number)) else { continue };
            let owner = text(&k_owner);
            if pid == me || matches!(owner.as_str(), "Dock" | "Window Server" | "Control Center" | "Notification Center" | "WindowManager") {
                continue;
            }
            found = Some((number as u32, owner, text(&k_title)));
            break;
        }
        CFRelease(list);
        found.ok_or_else(|| anyhow!("No app window is open"))
    }
}

/// The text in the window the user is looking at, top to bottom.
pub(super) fn read_screen() -> Result<String> {
    // macOS asks once; until it's allowed, screenshots show only the desktop.
    if !unsafe { CGPreflightScreenCaptureAccess() } && !unsafe { CGRequestScreenCaptureAccess() } {
        bail!("Murmur needs Screen Recording permission to read the screen: System Settings → Privacy & Security → Screen Recording, turn on Murmur, then reopen it");
    }
    let (window, app, title) = front_window()?;
    let shot = std::env::temp_dir().join(format!("murmur-screen-{}.png", std::process::id()));
    run_cmd(Command::new("screencapture").args(["-x", "-o", "-l", &window.to_string()]).arg(&shot), Duration::from_secs(10))?;
    let text = recognise_text(&shot);
    let _ = std::fs::remove_file(&shot);
    let text = text?;
    let heading = if title.is_empty() { app } else { format!("{app}, window “{title}”") };
    Ok(if text.trim().is_empty() { format!("{heading}: no text could be read in it") } else { format!("{heading}. Its text, top to bottom:\n{text}") })
}

/// Apple's on-device text recognition (Vision), lines read top to bottom and left to right.
fn recognise_text(image: &std::path::Path) -> Result<String> {
    use objc2::rc::autoreleasepool;
    use objc2::AllocAnyThread;
    use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
    use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};
    autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&image.to_string_lossy()));
        let handler = unsafe { VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &NSDictionary::new()) };
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        let as_request: &VNRequest = &request;
        handler.performRequests_error(&NSArray::from_slice(&[as_request])).map_err(|e| anyhow!("Couldn't read the screen: {}", e.localizedDescription()))?;
        // (top edge, left edge, height, text) in 0..1 units, y measured from the bottom.
        let mut pieces: Vec<(f64, f64, f64, String)> = request
            .results()
            .map(|found| {
                found
                    .iter()
                    .filter_map(|o| {
                        let text = o.topCandidates(1).firstObject()?.string().to_string();
                        let b = unsafe { o.boundingBox() };
                        Some((b.origin.y + b.size.height, b.origin.x, b.size.height, text))
                    })
                    .collect()
            })
            .unwrap_or_default();
        pieces.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.total_cmp(&b.1)));
        // Pieces whose tops are within half a line of each other share a line.
        let mut lines: Vec<(f64, f64, Vec<(f64, String)>)> = Vec::new();
        for (top, left, height, text) in pieces {
            match lines.last_mut() {
                Some((line_top, line_height, words)) if (*line_top - top).abs() < line_height.max(height) / 2.0 => words.push((left, text)),
                _ => lines.push((top, height, vec![(left, text)])),
            }
        }
        Ok(lines
            .into_iter()
            .map(|(_, _, mut words)| {
                words.sort_by(|a, b| a.0.total_cmp(&b.0));
                words.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join("   ")
            })
            .collect::<Vec<_>>()
            .join("\n"))
    })
}
