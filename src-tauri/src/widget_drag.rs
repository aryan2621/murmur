//! Dragging the widget. Clicks fall through it everywhere except on its visible parts (the
//! window is larger than what it shows), and once dragged it stays where it was put, across
//! launches, until "Reset Widget Position" in the menu bar.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};

/// A rectangle in the widget page, in CSS pixels: (x, y, width, height).
type Rect = (f64, f64, f64, f64);

#[derive(Default)]
pub struct WidgetDrag {
    /// The visible parts of the widget page; clicks anywhere else go to the apps underneath.
    hit: Mutex<Vec<Rect>>,
    /// A drag started on the widget and the mouse button hasn't been released yet.
    armed: AtomicBool,
    /// Where the user put the widget; None = place it automatically.
    pinned: Mutex<Option<PhysicalPosition<i32>>>,
}

fn position_file() -> PathBuf {
    crate::config::data_dir().join("widget_position.txt")
}

impl WidgetDrag {
    pub fn load() -> Self {
        let pinned = std::fs::read_to_string(position_file()).ok().and_then(|s| {
            let (x, y) = s.trim().split_once(',')?;
            Some(PhysicalPosition::new(x.parse().ok()?, y.parse().ok()?))
        });
        Self { pinned: Mutex::new(pinned), ..Default::default() }
    }
}

/// Where the user put the widget, if that spot is still on a connected display.
pub fn pinned(app: &AppHandle) -> Option<PhysicalPosition<i32>> {
    let p = (*app.state::<WidgetDrag>().pinned.lock().unwrap())?;
    let on_screen = app.available_monitors().ok()?.iter().any(|m| {
        let (mp, ms) = (m.position(), m.size());
        p.x >= mp.x && p.y >= mp.y && p.x < mp.x + ms.width as i32 && p.y < mp.y + ms.height as i32
    });
    on_screen.then_some(p)
}

/// True while the user is dragging the widget, so it isn't moved out from under them.
pub fn dragging(app: &AppHandle) -> bool {
    app.state::<WidgetDrag>().armed.load(Ordering::SeqCst) && left_button_down()
}

/// From the widget window's Moved event: only moves made by the user's drag are remembered.
pub fn on_moved(app: &AppHandle, pos: PhysicalPosition<i32>) {
    if dragging(app) {
        *app.state::<WidgetDrag>().pinned.lock().unwrap() = Some(pos);
    }
}

/// Back to automatic placement.
pub fn reset(app: &AppHandle) {
    *app.state::<WidgetDrag>().pinned.lock().unwrap() = None;
    let _ = std::fs::remove_file(position_file());
    if let Some(w) = app.get_webview_window("widget") {
        crate::place_widget(app, &w);
    }
}

/// Keeps clicks going through the widget except over its visible parts, and saves the position
/// once a drag ends. Runs for the life of the app.
pub fn track(app: AppHandle) {
    #[cfg(target_os = "macos")]
    loop {
        std::thread::sleep(std::time::Duration::from_millis(40));
        let state = app.state::<WidgetDrag>();
        if state.armed.load(Ordering::SeqCst) && !left_button_down() {
            state.armed.store(false, Ordering::SeqCst);
            if let Some(p) = *state.pinned.lock().unwrap() {
                let _ = std::fs::write(position_file(), format!("{},{}", p.x, p.y));
            }
        }
        let Some(w) = app.get_webview_window("widget") else { continue };
        let Ok(ns) = w.ns_window() else { continue };
        let ns = ns as usize;
        let rects = state.hit.lock().unwrap().clone();
        let _ = w.run_on_main_thread(move || unsafe { update_click_through(ns as *mut std::ffi::c_void, &rects) });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

fn left_button_down() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        let buttons: usize = objc2::msg_send![objc2::class!(NSEvent), pressedMouseButtons];
        buttons & 1 != 0
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Takes the mouse only while it is over one of `rects`. Main thread only.
#[cfg(target_os = "macos")]
unsafe fn update_click_through(ns_window: *mut std::ffi::c_void, rects: &[Rect]) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send, Encode, Encoding};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    unsafe impl Encode for CGPoint {
        const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    unsafe impl Encode for CGSize {
        const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }
    unsafe impl Encode for CGRect {
        const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
    }

    let window = &*(ns_window as *mut AnyObject);
    let visible: bool = msg_send![window, isVisible];
    let frame: CGRect = msg_send![window, frame];
    let mouse: CGPoint = msg_send![class!(NSEvent), mouseLocation];
    // Page coordinates: from the window's top-left corner, in points (= CSS pixels).
    let (x, y) = (mouse.x - frame.origin.x, frame.origin.y + frame.size.height - mouse.y);
    let over = visible && rects.iter().any(|&(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh);
    let ignoring: bool = msg_send![window, ignoresMouseEvents];
    if ignoring == over {
        let _: () = msg_send![window, setIgnoresMouseEvents: !over];
    }
}

/// The widget page reports its visible parts whenever they change.
#[tauri::command]
pub fn widget_hit_rects(app: AppHandle, rects: Vec<Rect>) {
    *app.state::<WidgetDrag>().hit.lock().unwrap() = rects;
}

/// Mouse down on the widget: hand the drag to the system (moves the window without focusing it).
#[tauri::command]
pub fn widget_drag(app: AppHandle, window: WebviewWindow) {
    app.state::<WidgetDrag>().armed.store(true, Ordering::SeqCst);
    if let Err(e) = window.start_dragging() {
        mlog!("widget: drag failed: {e}");
    }
}
