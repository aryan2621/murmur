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
    use objc2_app_kit::NSWorkspace;
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

    let Some(front) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        mlog!("focus: no frontmost app → Unknown");
        return Target::Unknown;
    };
    let pid = front.processIdentifier();
    if pid as u32 == std::process::id() {
        mlog!("focus: Murmur is the frontmost app");
        return Target::Murmur;
    }
    let bundle = front.bundleIdentifier().map(|b| b.to_string()).unwrap_or_default();

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

/// Bundle id of the frontmost app ("" if unknown).
pub fn frontmost_bundle() -> String {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .and_then(|a| a.bundleIdentifier())
            .map(|b| b.to_string())
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "macos"))]
    String::new()
}

/// Frame of the frontmost app's focused window in screen points (top-left origin):
/// `(x, y, width, height)`. Used to show the widget on the window being dictated into.
///
/// Accessibility's focused window can sit on another Space or display (Finder on the desktop
/// reports a Finder window from elsewhere, for example), so it is only trusted when it is one
/// of the app's windows actually on screen; otherwise the app's frontmost on-screen window is
/// used, and None (screen under the mouse) when the app has nothing on screen.
#[cfg(target_os = "macos")]
pub fn active_window_frame() -> Option<(f64, f64, f64, f64)> {
    use objc2_app_kit::NSWorkspace;

    let front = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = front.processIdentifier();
    let on_screen = onscreen_windows(pid);
    let focused = ax_focused_window_frame(pid);
    let same = |a: &(f64, f64, f64, f64), b: &(f64, f64, f64, f64)| {
        (a.0 - b.0).abs() < 2.0 && (a.1 - b.1).abs() < 2.0 && (a.2 - b.2).abs() < 2.0 && (a.3 - b.3).abs() < 2.0
    };
    match focused {
        Some(f) if on_screen.iter().any(|w| same(w, &f)) => Some(f),
        _ => {
            if focused.is_some() {
                mlog!("widget: focused window {focused:?} is not on screen, using {:?}", on_screen.first());
            }
            on_screen.first().copied()
        }
    }
}

/// Normal-level windows of `pid` on the Spaces currently shown, front to back, in screen points
/// (top-left origin). Needs no permission: only window names are protected.
#[cfg(target_os = "macos")]
fn onscreen_windows(pid: i32) -> Vec<(f64, f64, f64, f64)> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    struct CGRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

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
    /// Skips helper windows (tooltips, status items, invisible trackers).
    const MIN_SIDE: f64 = 50.0;

    let k_pid = CFString::from_static_string("kCGWindowOwnerPID");
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
            if int(&k_pid) != Some(pid) || int(&k_layer) != Some(0) {
                continue;
            }
            let mut alpha = 1.0f64;
            let a = CFDictionaryGetValue(info, k_alpha.as_CFTypeRef());
            if !a.is_null() {
                CFNumberGetValue(a, NUMBER_FLOAT64, &mut alpha as *mut f64 as *mut c_void);
            }
            let bounds = CFDictionaryGetValue(info, k_bounds.as_CFTypeRef());
            let mut r = CGRect::default();
            if alpha > 0.0
                && !bounds.is_null()
                && CGRectMakeWithDictionaryRepresentation(bounds, &mut r)
                && r.width >= MIN_SIDE
                && r.height >= MIN_SIDE
            {
                windows.push((r.x, r.y, r.width, r.height));
            }
        }
        CFRelease(list);
    }
    windows
}

/// The app's focused (or main) window frame as Accessibility reports it, wherever it is.
#[cfg(target_os = "macos")]
fn ax_focused_window_frame(pid: i32) -> Option<(f64, f64, f64, f64)> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    const AX_VALUE_CGPOINT: u32 = 1;
    const AX_VALUE_CGSIZE: u32 = 2;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(el: *const c_void, attr: CFStringRef, value: *mut *const c_void) -> i32;
        fn AXUIElementSetMessagingTimeout(el: *const c_void, seconds: f32) -> i32;
        fn AXValueGetValue(value: *const c_void, kind: u32, out: *mut c_void) -> bool;
    }

    unsafe fn copy_attr(el: *const c_void, name: &'static str) -> Option<CFType> {
        let mut v: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(el, CFString::from_static_string(name).as_concrete_TypeRef(), &mut v);
        (err == 0 && !v.is_null()).then(|| CFType::wrap_under_create_rule(v as _))
    }

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
        let (mut p, mut s) = (CGPoint::default(), CGSize::default());
        if !AXValueGetValue(pos.as_CFTypeRef(), AX_VALUE_CGPOINT, &mut p as *mut _ as *mut c_void)
            || !AXValueGetValue(size.as_CFTypeRef(), AX_VALUE_CGSIZE, &mut s as *mut _ as *mut c_void)
            || s.width < 1.0
            || s.height < 1.0
        {
            return None;
        }
        Some((p.x, p.y, s.width, s.height))
    }
}

#[cfg(not(target_os = "macos"))]
pub fn active_window_frame() -> Option<(f64, f64, f64, f64)> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn detect() -> Target {
    Target::Unknown
}
