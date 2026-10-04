//! macOS listener built on a CGEventTap. Watching `flagsChanged` is the only way to see the
//! Fn/Globe key, which never produces normal key-down events.
//!
//! Preferred mode is an *active* tap at the HID level that swallows the hotkey's own events,
//! so macOS never sees Fn and doesn't open the emoji picker / input switcher / dictation.
//! That needs Accessibility permission; without it we fall back to a listen-only tap
//! (Input Monitoring), where the user must set "Press 🌐 key to" → Do Nothing.

use super::{Handler, Hotkey, HotkeyEvent};
use anyhow::{anyhow, Result};
use std::cell::Cell;
use std::ffi::c_void;
use std::sync::mpsc;

type Ptr = *mut c_void;
type TapCallback = extern "C" fn(proxy: Ptr, etype: u32, event: Ptr, user: Ptr) -> Ptr;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, cb: TapCallback, user: Ptr) -> Ptr;
    fn CGEventTapEnable(tap: Ptr, enable: bool);
    fn CGEventTapIsEnabled(tap: Ptr) -> bool;
    fn CGEventGetFlags(event: Ptr) -> u64;
    fn CGEventGetIntegerValueField(event: Ptr, field: u32) -> i64;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFMachPortCreateRunLoopSource(alloc: Ptr, port: Ptr, order: isize) -> Ptr;
    fn CFRunLoopGetCurrent() -> Ptr;
    fn CFRunLoopAddSource(rl: Ptr, source: Ptr, mode: *const c_void);
    fn CFRunLoopRun();
    static kCFRunLoopCommonModes: *const c_void;
}

const HID_TAP: u32 = 0;
const SESSION_TAP: u32 = 1;
const HEAD_INSERT: u32 = 0;
const ACTIVE_FILTER: u32 = 0;
const LISTEN_ONLY: u32 = 1;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_USER: u32 = 0xFFFF_FFFF;
const FIELD_KEYCODE: u32 = 9;

const KEYCODE_FN: i64 = 63;
/// After a bare Fn tap, macOS synthesises a key-down/up with this keycode; it is what
/// actually opens the emoji picker / switches input source.
const KEYCODE_GLOBE: i64 = 179;
const KEYCODE_ESCAPE: i64 = 53;

struct Binding {
    id: Hotkey,
    keycode: i64,
    flag: u64,
    held: Cell<bool>,
}

struct Ctx {
    keys: Vec<Binding>,
    tap: Cell<Ptr>,
    /// True when the tap is active and may drop events.
    swallow: Cell<bool>,
    handler: Handler,
}

/// (virtual keycode, device-dependent modifier flag bit)
fn key_spec(name: &str) -> Result<(i64, u64)> {
    Ok(match name {
        "fn" | "globe" => (KEYCODE_FN, 0x0080_0000),
        "right_command" => (54, 0x10),
        "right_option" => (61, 0x40),
        "right_control" | "right_ctrl" => (62, 0x2000),
        "right_shift" => (60, 0x4),
        other => return Err(anyhow!("Unsupported hotkey '{other}' on macOS")),
    })
}

extern "C" fn callback(_proxy: Ptr, etype: u32, event: Ptr, user: Ptr) -> Ptr {
    let ctx = unsafe { &*(user as *const Ctx) };
    let drop_it = std::ptr::null_mut();
    match etype {
        TAP_DISABLED_TIMEOUT | TAP_DISABLED_USER => unsafe { CGEventTapEnable(ctx.tap.get(), true) },
        FLAGS_CHANGED => {
            let code = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYCODE) };
            if let Some(key) = ctx.keys.iter().find(|k| k.keycode == code) {
                let down = unsafe { CGEventGetFlags(event) } & key.flag != 0;
                if down && !key.held.get() {
                    key.held.set(true);
                    (ctx.handler)(key.id, HotkeyEvent::Pressed);
                } else if !down && key.held.get() {
                    key.held.set(false);
                    (ctx.handler)(key.id, HotkeyEvent::Released);
                }
                if ctx.swallow.get() {
                    return drop_it;
                }
            }
        }
        KEY_DOWN | KEY_UP => {
            let code = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYCODE) };
            if code == KEYCODE_GLOBE && ctx.keys.iter().any(|k| k.keycode == KEYCODE_FN) && ctx.swallow.get() {
                return drop_it;
            }
            if etype == KEY_DOWN {
                if code == KEYCODE_ESCAPE {
                    // Only swallowed when it actually cancelled something.
                    for key in &ctx.keys {
                        if (ctx.handler)(key.id, HotkeyEvent::Escape) && ctx.swallow.get() {
                            key.held.set(false);
                            return drop_it;
                        }
                    }
                } else {
                    for key in ctx.keys.iter().filter(|k| k.held.get()) {
                        key.held.set(false);
                        (ctx.handler)(key.id, HotkeyEvent::Cancelled);
                    }
                }
            }
        }
        _ => {}
    }
    event
}

/// Starts one listener for all `keys` on its own run-loop thread. Errors if a key is unknown or
/// neither Accessibility nor Input Monitoring permission has been granted yet.
pub fn start(keys: &[(String, Hotkey)], handler: Handler) -> Result<()> {
    let keys = keys
        .iter()
        .map(|(name, id)| key_spec(name).map(|(keycode, flag)| Binding { id: *id, keycode, flag, held: Cell::new(false) }))
        .collect::<Result<Vec<_>>>()?;
    let (tx, rx) = mpsc::channel::<Result<()>>();

    std::thread::Builder::new().name("murmur-hotkey".into()).spawn(move || unsafe {
        CGRequestListenEventAccess();
        let ctx = Box::into_raw(Box::new(Ctx {
            keys,
            tap: Cell::new(std::ptr::null_mut()),
            swallow: Cell::new(true),
            handler,
        }));
        let mask = (1u64 << FLAGS_CHANGED) | (1u64 << KEY_DOWN) | (1u64 << KEY_UP);
        let mut tap = CGEventTapCreate(HID_TAP, HEAD_INSERT, ACTIVE_FILTER, mask, callback, ctx as Ptr);
        if tap.is_null() {
            mlog!("active key tap unavailable (no Accessibility permission); listening only");
            (*ctx).swallow.set(false);
            tap = CGEventTapCreate(SESSION_TAP, HEAD_INSERT, LISTEN_ONLY, mask, callback, ctx as Ptr);
        }
        if tap.is_null() {
            drop(Box::from_raw(ctx));
            let _ = tx.send(Err(anyhow!("Accessibility or Input Monitoring permission is required")));
            return;
        }
        mlog!("key tap ready ({})", if (*ctx).swallow.get() { "active, Fn is intercepted" } else { "listen-only" });
        (*ctx).tap.set(tap);
        let source = CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        let _ = tx.send(Ok(()));
        // Watchdog: macOS can switch a tap off (e.g. after a slow callback) without us noticing.
        let tap_addr = tap as usize;
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let tap = tap_addr as Ptr;
            if !CGEventTapIsEnabled(tap) {
                mlog!("key tap was disabled by macOS; re-enabling");
                CGEventTapEnable(tap, true);
            }
        });
        CFRunLoopRun();
    })?;

    rx.recv()?
}
