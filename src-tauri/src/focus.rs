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
    /// Couldn't tell; paste as usual.
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

    let Some(front) = NSWorkspace::sharedWorkspace().frontmostApplication() else { return Target::Unknown };
    let pid = front.processIdentifier();
    if pid as u32 == std::process::id() {
        return Target::Murmur;
    }
    let bundle = front.bundleIdentifier().map(|b| b.to_string()).unwrap_or_default();

    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
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
            return classify(&bundle, None, false);
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

#[cfg(not(target_os = "macos"))]
pub fn detect() -> Target {
    Target::Unknown
}
