//! Where would a paste land? Uses the Accessibility API to look at the focused UI element of
//! the frontmost app, so Murmur doesn't "paste" into nothing and claim success.
//!
//! Deliberately biased towards pasting: only a confidently non-text focus counts as no target,
//! because some apps don't expose their text fields well. A wrong "no target" costs one ⌘V.

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A text field, text area, search box, editor…
    Editable,
    /// Murmur's own window is in front.
    Murmur,
    /// Something that clearly can't take text (desktop, a list, a video, a page body).
    NotEditable { app: String, role: String },
    /// Couldn't tell; paste, but keep the text on the clipboard in case it landed nowhere.
    Unknown,
}

const TEXT_ROLES: &[&str] = &["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"];
const NON_TEXT_ROLES: &[&str] = &[
    "AXWebArea", "AXGroup", "AXScrollArea", "AXList", "AXTable", "AXOutline", "AXImage", "AXWindow",
    "AXButton", "AXCell", "AXRow", "AXStaticText", "AXSplitGroup", "AXBrowser", "AXToolbar",
    "AXTabGroup", "AXLayoutArea", "AXMenuButton", "AXCheckBox", "AXRadioButton", "AXSlider",
];

/// The decision itself. `role` is None when the app reports no focused element.
pub fn classify(app_bundle: &str, role: Option<&str>, value_settable: bool) -> Target {
    match role {
        Some(r) if TEXT_ROLES.contains(&r) || value_settable && r != "AXSlider" => Target::Editable,
        Some(r) if NON_TEXT_ROLES.contains(&r) => Target::NotEditable { app: app_bundle.into(), role: r.into() },
        // Finder with nothing focused = the desktop or a plain Finder window.
        None if app_bundle == "com.apple.finder" => Target::NotEditable { app: app_bundle.into(), role: "none".into() },
        _ => Target::Unknown,
    }
}

#[cfg(target_os = "macos")]
pub fn detect() -> Target {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;

    type AXUIElementRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(el: AXUIElementRef, attr: CFStringRef, value: *mut *const c_void) -> i32;
        fn AXUIElementSetAttributeValue(el: AXUIElementRef, attr: CFStringRef, value: *const c_void) -> i32;
        fn AXUIElementIsAttributeSettable(el: AXUIElementRef, attr: CFStringRef, settable: *mut u8) -> i32;
        fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, seconds: f32) -> i32;
    }

    /// kAXErrorNoValue: the app answered, and nothing in it has keyboard focus.
    const AX_ERROR_NO_VALUE: i32 = -25212;

    // Live: NSWorkspace's frontmostApplication can be stale when read off the main thread.
    let Some(pid) = ax::focused_pid().or_else(active_pid) else {
        mlog!("focus: no app has focus → Unknown");
        return Target::Unknown;
    };
    if pid as u32 == std::process::id() {
        mlog!("focus: Murmur is the frontmost app");
        return Target::Murmur;
    }
    let bundle = bundle_of(pid);

    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            mlog!("focus: app={bundle} has no accessibility element → Unknown");
            return Target::Unknown;
        }
        let app_cf = CFType::wrap_under_create_rule(app as _);
        AXUIElementSetMessagingTimeout(app, 0.25);
        // Electron apps (Slack, VS Code, Cursor…) only expose their text fields once asked to.
        AXUIElementSetAttributeValue(
            app,
            CFString::from_static_string("AXManualAccessibility").as_concrete_TypeRef(),
            CFBoolean::true_value().as_CFTypeRef(),
        );

        let mut focused: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            app,
            CFString::from_static_string("AXFocusedUIElement").as_concrete_TypeRef(),
            &mut focused,
        );
        drop(app_cf);
        if err != 0 || focused.is_null() {
            // Nothing focused at all can't take a paste; any other error just means we can't tell.
            let target = if err == AX_ERROR_NO_VALUE {
                Target::NotEditable { app: bundle.clone(), role: "none".into() }
            } else {
                classify(&bundle, None, false)
            };
            mlog!("focus: app={bundle} no focused element (AX error {err}) → {target:?}");
            return target;
        }
        let focused_cf = CFType::wrap_under_create_rule(focused as _);

        let mut role_ref: *const c_void = std::ptr::null();
        let role = if AXUIElementCopyAttributeValue(
            focused,
            CFString::from_static_string("AXRole").as_concrete_TypeRef(),
            &mut role_ref,
        ) == 0
            && !role_ref.is_null()
        {
            Some(CFString::wrap_under_create_rule(role_ref as CFStringRef).to_string())
        } else {
            None
        };

        let mut settable = 0u8;
        AXUIElementIsAttributeSettable(
            focused,
            CFString::from_static_string("AXValue").as_concrete_TypeRef(),
            &mut settable,
        );
        drop(focused_cf);

        let target = classify(&bundle, role.as_deref(), settable != 0);
        mlog!("focus: app={bundle} role={} settable={} → {target:?}", role.as_deref().unwrap_or("none"), settable != 0);
        target
    }
}

/// Bundle id of the app the user is working in ("" if unknown).
pub fn frontmost_bundle() -> String {
    #[cfg(target_os = "macos")]
    {
        active_pid().map(bundle_of).unwrap_or_default()
    }
    #[cfg(not(target_os = "macos"))]
    String::new()
}

/// Bundle id of a running process ("" if unknown).
#[cfg(target_os = "macos")]
fn bundle_of(pid: i32) -> String {
    objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|a| a.bundleIdentifier())
        .map(|b| b.to_string())
        .unwrap_or_default()
}

// Where the user is working, read live every time.
//
// Two macOS shortcuts look right but aren't:
// - NSWorkspace's frontmostApplication is a cached value kept current by activation
//   notifications delivered to a thread's run loop. Read from our hotkey and worker threads,
//   which run none, it can lag or stay stuck on an app the user already left until relaunch.
// - The first normal-level window in the window server's list isn't always the app's main
//   window: full-screen browsers and Electron apps stack toolbar strips above the page as
//   separate windows.
// So: the app with keyboard focus comes from Accessibility (a live query to that app; the same
// source screen readers use), and its focused window is checked against the windows actually on
// screen by overlap, not exact size (a full-screen window reports the whole screen while its
// pieces don't). Without Accessibility, the owner of the topmost window, and its largest window.

/// A frame in screen points, top-left origin: (x, y, width, height).
pub type Frame = (f64, f64, f64, f64);

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// The connected displays, in screen points (top-left origin, like window frames).
#[cfg(target_os = "macos")]
fn display_frames() -> Vec<Frame> {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> CGRect;
    }
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    if unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } != 0 {
        return Vec::new();
    }
    ids[..count as usize]
        .iter()
        .map(|&id| {
            let r = unsafe { CGDisplayBounds(id) };
            (r.x, r.y, r.width, r.height)
        })
        .collect()
}

#[cfg(target_os = "macos")]
/// Mostly on a display. While Spaces slide past, the window server reports windows at in-between
/// positions off every display; those aren't where the user is looking.
fn on_display(f: &Frame, displays: &[Frame]) -> bool {
    displays.is_empty() || displays.iter().map(|d| overlap(f, d)).sum::<f64>() >= 0.5 * area(f)
}

#[cfg(target_os = "macos")]
/// The part of a window that's on its display, so the widget isn't placed below the screen.
fn visible_part(f: Frame, displays: &[Frame]) -> Frame {
    let Some(d) = displays.iter().max_by(|a, b| overlap(&f, a).total_cmp(&overlap(&f, b))).filter(|d| overlap(&f, d) > 0.0) else {
        return f;
    };
    let (x, y) = (f.0.max(d.0), f.1.max(d.1));
    (x, y, (f.0 + f.2).min(d.0 + d.2) - x, (f.1 + f.3).min(d.1 + d.3) - y)
}

/// A normal window on the Spaces currently shown.
#[cfg(target_os = "macos")]
struct OnScreen {
    pid: i32,
    owner: String,
    frame: Frame,
}

/// Normal-level windows on the Spaces currently shown, front to back, leaving out ours, the
/// system's, invisible ones, tiny helpers and ones off every display. Needs no permission (only
/// window titles are protected).
#[cfg(target_os = "macos")]
fn onscreen_windows(displays: &[Frame]) -> Vec<OnScreen> {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
        fn CGRectMakeWithDictionaryRepresentation(dict: *const c_void, rect: *mut CGRect) -> bool;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: *const c_void, kind: isize, out: *mut c_void) -> bool;
        fn CFRelease(cf: *const c_void);
    }
    const ON_SCREEN_ONLY: u32 = 1 << 0;
    const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const NUMBER_SINT32: isize = 3;
    const NUMBER_FLOAT64: isize = 6;
    const MIN_SIDE: f64 = 50.0;
    const OURS: [&str; 3] = ["Murmur", "Jarvis", "Capturita"];
    const SYSTEM: [&str; 6] = ["Window Server", "Dock", "Control Center", "Notification Center", "WindowManager", "Screenshot"];

    let me = std::process::id() as i32;
    let k_pid = CFString::from_static_string("kCGWindowOwnerPID");
    let k_owner = CFString::from_static_string("kCGWindowOwnerName");
    let k_layer = CFString::from_static_string("kCGWindowLayer");
    let k_alpha = CFString::from_static_string("kCGWindowAlpha");
    let k_bounds = CFString::from_static_string("kCGWindowBounds");
    let mut windows = Vec::new();
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP_ELEMENTS, 0);
        if list.is_null() {
            return windows;
        }
        for i in 0..CFArrayGetCount(list) {
            let info = CFArrayGetValueAtIndex(list, i);
            let int = |key: &CFString| {
                let n = CFDictionaryGetValue(info, key.as_CFTypeRef());
                let mut v = 0i32;
                (!n.is_null() && CFNumberGetValue(n, NUMBER_SINT32, &mut v as *mut i32 as *mut c_void)).then_some(v)
            };
            let (Some(pid), Some(0)) = (int(&k_pid), int(&k_layer)) else { continue };
            if pid == me {
                continue;
            }
            let owner = CFDictionaryGetValue(info, k_owner.as_CFTypeRef());
            let owner = if owner.is_null() { String::new() } else { CFString::wrap_under_get_rule(owner as CFStringRef).to_string() };
            if OURS.contains(&owner.as_str()) || SYSTEM.contains(&owner.as_str()) {
                continue;
            }
            let mut alpha = 1.0f64;
            let a = CFDictionaryGetValue(info, k_alpha.as_CFTypeRef());
            if !a.is_null() {
                CFNumberGetValue(a, NUMBER_FLOAT64, &mut alpha as *mut f64 as *mut c_void);
            }
            let bounds = CFDictionaryGetValue(info, k_bounds.as_CFTypeRef());
            let mut r = CGRect::default();
            if alpha > 0.0 && !bounds.is_null() && CGRectMakeWithDictionaryRepresentation(bounds, &mut r) && r.width >= MIN_SIDE && r.height >= MIN_SIDE {
                let frame = (r.x, r.y, r.width, r.height);
                if on_display(&frame, displays) {
                    windows.push(OnScreen { pid, owner, frame });
                }
            }
        }
        CFRelease(list);
    }
    windows
}

#[cfg(target_os = "macos")]
mod ax {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateSystemWide() -> *const c_void;
        fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(el: *const c_void, attr: CFStringRef, value: *mut *const c_void) -> i32;
        fn AXUIElementSetMessagingTimeout(el: *const c_void, seconds: f32) -> i32;
        fn AXUIElementGetPid(el: *const c_void, pid: *mut i32) -> i32;
        fn AXValueGetValue(value: *const c_void, kind: u32, out: *mut c_void) -> bool;
    }

    unsafe fn copy_attr(el: *const c_void, name: &'static str) -> Option<CFType> {
        let mut v: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(el, CFString::from_static_string(name).as_concrete_TypeRef(), &mut v);
        (err == 0 && !v.is_null()).then(|| CFType::wrap_under_create_rule(v as _))
    }

    /// The process with keyboard focus, asked of Accessibility now (not a cached value).
    pub fn focused_pid() -> Option<i32> {
        unsafe {
            let system = AXUIElementCreateSystemWide();
            if system.is_null() {
                return None;
            }
            let _system = CFType::wrap_under_create_rule(system as _);
            AXUIElementSetMessagingTimeout(system, 0.25);
            let app = copy_attr(system, "AXFocusedApplication")?;
            let mut pid = 0;
            (AXUIElementGetPid(app.as_CFTypeRef(), &mut pid) == 0 && pid > 0).then_some(pid)
        }
    }

    /// The app's focused (else main) window frame, wherever it is.
    pub fn focused_window(pid: i32) -> Option<super::Frame> {
        #[repr(C)]
        #[derive(Default)]
        struct Pair(f64, f64);
        const AX_VALUE_CGPOINT: u32 = 1;
        const AX_VALUE_CGSIZE: u32 = 2;
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return None;
            }
            let _app = CFType::wrap_under_create_rule(app as _);
            AXUIElementSetMessagingTimeout(app, 0.25);
            let window = copy_attr(app, "AXFocusedWindow").or_else(|| copy_attr(app, "AXMainWindow"))?;
            let pos = copy_attr(window.as_CFTypeRef(), "AXPosition")?;
            let size = copy_attr(window.as_CFTypeRef(), "AXSize")?;
            let (mut p, mut s) = (Pair::default(), Pair::default());
            let ok = AXValueGetValue(pos.as_CFTypeRef(), AX_VALUE_CGPOINT, &mut p as *mut _ as *mut c_void)
                && AXValueGetValue(size.as_CFTypeRef(), AX_VALUE_CGSIZE, &mut s as *mut _ as *mut c_void);
            (ok && s.0 >= 1.0 && s.1 >= 1.0).then_some((p.0, p.1, s.0, s.1))
        }
    }
}

/// The process the user is working in: the app with keyboard focus, else the owner of the
/// topmost window.
#[cfg(target_os = "macos")]
pub fn active_pid() -> Option<i32> {
    ax::focused_pid()
        .filter(|&pid| pid != std::process::id() as i32)
        .or_else(|| onscreen_windows(&display_frames()).first().map(|w| w.pid))
}

fn area(f: &Frame) -> f64 {
    f.2 * f.3
}

fn overlap(a: &Frame, b: &Frame) -> f64 {
    let w = (a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0);
    let h = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

/// The app's window nearest the front that isn't a strip inside a bigger one of its windows
/// (full-screen browsers and Electron apps stack toolbars over the page as separate windows).
#[cfg(target_os = "macos")]
fn front_window<'a>(mine: &[&'a OnScreen]) -> Option<&'a OnScreen> {
    mine.iter()
        .copied()
        .find(|w| !mine.iter().any(|o| area(&o.frame) > area(&w.frame) && overlap(&o.frame, &w.frame) >= 0.9 * area(&w.frame)))
}

/// System agents that take keyboard focus for a moment (a menu bar item, a notification,
/// Spotlight) without being where the user works.
#[cfg(target_os = "macos")]
const SYSTEM_UI: [&str; 5] = [
    "com.apple.controlcenter",
    "com.apple.notificationcenterui",
    "com.apple.UserNotificationCenter",
    "com.apple.Spotlight",
    "com.apple.systemuiserver",
];

/// The window the user is working in, on screen now (the part of it on its display); None when
/// it can't be told (the caller then uses the screen under the mouse). `log` writes how it was
/// chosen (off for the repeated checks while the widget follows the active window).
#[cfg(target_os = "macos")]
pub fn active_window_frame(log: bool) -> Option<Frame> {
    let displays = display_frames();
    let windows = onscreen_windows(&displays);
    let focused = ax::focused_pid().filter(|&pid| pid != std::process::id() as i32);
    // The focused app, else (no Accessibility answer) the owner of the topmost window.
    let mut pid = focused.or_else(|| windows.first().map(|w| w.pid))?;
    if SYSTEM_UI.contains(&bundle_of(pid).as_str()) {
        if let Some(top) = windows.first() {
            if log {
                mlog!("widget: {} has focus for a moment; using the window in front, {}'s", bundle_of(pid), top.owner);
            }
            pid = top.pid;
        }
    }
    let mine: Vec<&OnScreen> = windows.iter().filter(|w| w.pid == pid).collect();
    let Some(front) = front_window(&mine) else {
        // The focused app has nothing on screen here (e.g. Finder with only the desktop, or a
        // Space still sliding in).
        if log {
            mlog!("widget: the app in front (pid {pid}) has no window on screen; using the screen under the mouse");
        }
        return None;
    };
    // Its focused window, when that is on this Space: mostly covered by its windows here.
    if let Some(f) = ax::focused_window(pid) {
        let covered: f64 = mine.iter().map(|w| overlap(&w.frame, &f)).sum();
        if covered >= 0.5 * area(&f) {
            if log {
                mlog!("widget: on {}'s focused window {f:?}", front.owner);
            }
            return Some(visible_part(f, &displays));
        }
        if log {
            mlog!("widget: {}'s focused window {f:?} is elsewhere; using its front window {:?}", front.owner, front.frame);
        }
    } else if log {
        mlog!("widget: on {}'s front window {:?}", front.owner, front.frame);
    }
    Some(visible_part(front.frame, &displays))
}

#[cfg(not(target_os = "macos"))]
pub fn active_window_frame(_log: bool) -> Option<(f64, f64, f64, f64)> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn detect() -> Target {
    Target::Unknown
}
