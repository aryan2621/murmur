/// Log to stderr and to `murmur.log` in the data dir (the app has no console when launched normally).
macro_rules! mlog {
    ($($t:tt)*) => { $crate::config::log(&format!($($t)*)) };
}

mod audio;
mod cleanup;
mod commands;
mod config;
#[cfg(target_os = "macos")]
mod fn_key;
mod focus;
mod gesture;
mod history;
mod hotkey;
mod model;
mod paste;
mod transcribe;
mod widget_drag;

use audio::Recorder;
use config::Config;
use gesture::ReleaseAction;
use hotkey::HotkeyEvent;
use serde::Serialize;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};
use transcribe::Transcriber;

/// Clips shorter than this are treated as accidental taps.
const MIN_SECONDS: f32 = 0.3;
/// Below this peak RMS the clip is considered silence (prevents Whisper hallucinations).
const SILENCE_RMS: f32 = 0.006;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Loading,
    Idle,
    Recording,
    Transcribing,
}

#[derive(Serialize, Clone)]
struct WidgetState<'a> {
    state: &'a str,
    message: Option<String>,
    progress: Option<f32>,
}

/// Hotkey timing state used to tell holds, taps and double-taps apart.
#[derive(Default)]
struct Gesture {
    pressed_at: Option<Instant>,
    last_tap_end: Option<Instant>,
    gap_before_press: Option<Duration>,
    /// Hands-free recording: runs until the next press.
    locked: bool,
    /// The press that stopped a hands-free recording; its release is not a gesture.
    ignore_release: bool,
}

struct Core {
    app: AppHandle,
    cfg: RwLock<Config>,
    recorder: Recorder,
    transcriber: Mutex<Option<Arc<Transcriber>>>,
    phase: Mutex<Phase>,
    gesture: Mutex<Gesture>,
    last_text: Mutex<Option<String>>,
    status_item: MenuItem<Wry>,
    status: Mutex<String>,
    /// Serialises model loads so a press during an idle reload doesn't load it twice.
    load_lock: Mutex<()>,
    last_used: Mutex<Instant>,
    /// Speech model download progress (0–1) while one is running.
    download: Mutex<Option<f32>>,
}

impl Core {
    fn cfg(&self) -> Config {
        self.cfg.read().unwrap().clone()
    }

    fn widget(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window("widget")
    }

    fn set_status(&self, text: &str) {
        let _ = self.status_item.set_text(text);
        *self.status.lock().unwrap() = text.to_string();
        let _ = self.app.emit_to("main", "status", text);
    }

    fn ready_status(&self) {
        if paste::has_permission(false) {
            self.set_status(&format!("Ready — hold {} to dictate", key_label(&self.cfg().hotkey)));
        } else {
            self.set_status("Allow Murmur in Privacy & Security → Accessibility");
        }
    }

    /// The loaded model, loading it first if it was unloaded to save memory.
    fn ensure_transcriber(&self) -> anyhow::Result<Arc<Transcriber>> {
        if let Some(t) = self.transcriber.lock().unwrap().clone() {
            return Ok(t);
        }
        let _guard = self.load_lock.lock().unwrap();
        if let Some(t) = self.transcriber.lock().unwrap().clone() {
            return Ok(t);
        }
        let path = model::model_path(&config::data_dir(), &self.cfg().model);
        if !path.exists() {
            return Err(anyhow::anyhow!("Speech model is not downloaded yet"));
        }
        let started = Instant::now();
        let t = Arc::new(Transcriber::load(&path)?);
        mlog!("model reloaded in {} ms", started.elapsed().as_millis());
        *self.transcriber.lock().unwrap() = Some(t.clone());
        Ok(t)
    }

    /// Frees the model after `unload_after_minutes` of inactivity.
    fn watch_idle(self: &Arc<Self>) {
        loop {
            std::thread::sleep(Duration::from_secs(30));
            let minutes = self.cfg().unload_after_minutes;
            if minutes == 0 || self.phase() != Phase::Idle {
                continue;
            }
            let idle = self.last_used.lock().unwrap().elapsed();
            if idle >= Duration::from_secs(minutes as u64 * 60) && self.transcriber.lock().unwrap().take().is_some() {
                mlog!("model unloaded after {minutes} idle minutes");
            }
        }
    }

    fn phase(&self) -> Phase {
        *self.phase.lock().unwrap()
    }

    fn set_phase(&self, p: Phase) {
        *self.phase.lock().unwrap() = p;
    }

    /// Murmur's app window is open and has keyboard focus, so its page can take dictation.
    fn main_window_focused(&self) -> bool {
        self.app
            .get_webview_window("main")
            .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
    }

    fn emit(&self, state: &str, message: Option<String>, progress: Option<f32>) {
        let _ = self.app.emit_to("widget", "widget-state", WidgetState { state, message, progress });
    }

    fn show_widget(&self) {
        let Some(w) = self.widget() else {
            mlog!("widget: window not found");
            return;
        };
        let before = focus::frontmost_bundle();
        if !widget_drag::dragging(&self.app) {
            place_widget(&self.app, &w);
        }
        show_without_focus(&w);
        // Diagnostics: showing the widget must never change which app (or Space) is in front.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let after = focus::frontmost_bundle();
            if after != before {
                mlog!("widget: front app changed from {before} to {after} while showing");
            }
        });
    }

    fn play(&self, sound: &str) {
        if self.cfg().sounds {
            play_sound(sound);
        }
    }

    /// Hides after `delay`, unless a new recording started in the meantime.
    fn hide_widget_later(self: &Arc<Self>, delay: Duration) {
        let core = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if matches!(core.phase(), Phase::Idle | Phase::Loading) {
                if let Some(w) = core.widget() {
                    hide_widget(&w);
                }
            }
        });
    }

    /// Returns true when the event was consumed (only matters for Esc).
    fn on_hotkey(self: &Arc<Self>, event: HotkeyEvent) -> bool {
        let now = Instant::now();
        match event {
            HotkeyEvent::Pressed => {
                let mut g = self.gesture.lock().unwrap();
                if self.phase() == Phase::Recording && g.locked {
                    g.locked = false;
                    g.ignore_release = true;
                    drop(g);
                    mlog!("hands-free stopped");
                    self.finish_recording();
                } else if self.phase() == Phase::Idle {
                    g.pressed_at = Some(now);
                    g.gap_before_press = g.last_tap_end.map(|t| now - t);
                    drop(g);
                    self.start_recording();
                } else if self.phase() == Phase::Loading {
                    drop(g);
                    self.emit("error", Some("Model still loading…".into()), None);
                    self.show_widget();
                    self.hide_widget_later(Duration::from_millis(1500));
                }
                false
            }
            HotkeyEvent::Released => {
                let mut g = self.gesture.lock().unwrap();
                if std::mem::take(&mut g.ignore_release) || g.locked || self.phase() != Phase::Recording {
                    return false;
                }
                let held = g.pressed_at.map(|t| now - t).unwrap_or_default();
                let action = gesture::on_release(held, g.gap_before_press);
                mlog!("hotkey released after {} ms → {action:?}", held.as_millis());
                match action {
                    ReleaseAction::Finish => {
                        g.last_tap_end = None;
                        drop(g);
                        self.finish_recording();
                    }
                    ReleaseAction::DiscardTap => {
                        g.last_tap_end = Some(now);
                        drop(g);
                        self.cancel_recording();
                    }
                    ReleaseAction::LockHandsFree => {
                        g.last_tap_end = None;
                        g.locked = true;
                        drop(g);
                        self.emit("locked", None, None);
                        self.show_widget();
                        self.play("Tink");
                    }
                }
                false
            }
            HotkeyEvent::Cancelled => {
                if !self.gesture.lock().unwrap().locked {
                    self.cancel_recording();
                }
                false
            }
            HotkeyEvent::Escape => {
                if self.phase() != Phase::Recording {
                    return false;
                }
                mlog!("cancelled with Esc");
                self.gesture.lock().unwrap().locked = false;
                self.cancel_recording();
                true
            }
        }
    }

    /// Opens the mic for this press. Returns at once (the hotkey's thread must stay free to see
    /// the release when it happens); the rest runs once the mic is open.
    fn start_recording(self: &Arc<Self>) {
        self.set_phase(Phase::Recording);
        let app = self.app.clone();
        let on_level = Box::new(move |level: f32| {
            let _ = app.emit_to("widget", "level", level);
        });
        let cfg = self.cfg();
        let opened = self.recorder.start_async(cfg.input_device, on_level);
        let core = self.clone();
        std::thread::spawn(move || match opened.recv().unwrap_or_else(|e| Err(anyhow::anyhow!("{e}"))) {
            Ok(()) => core.recording_started(),
            Err(e) => {
                core.set_phase(Phase::Idle);
                core.fail(&format!("Mic error: {e}"));
            }
        });
    }

    fn recording_started(self: &Arc<Self>) {
        if self.phase() != Phase::Recording {
            // Released and handled while the mic was still opening.
            return;
        }
        self.emit("recording", None, None);
        *self.last_used.lock().unwrap() = Instant::now();
        if self.transcriber.lock().unwrap().is_none() {
            // Reload while the user is still talking.
            let core = self.clone();
            std::thread::spawn(move || {
                if let Err(e) = core.ensure_transcriber() {
                    mlog!("model reload failed: {e}");
                }
            });
        }
        // Show only if this turns out to be a hold, so quick taps never flash the widget.
        let core = self.clone();
        let pressed_at = self.gesture.lock().unwrap().pressed_at;
        std::thread::spawn(move || {
            std::thread::sleep(gesture::SHOW_DELAY);
            let g = core.gesture.lock().unwrap();
            let same_press = g.pressed_at == pressed_at;
            let locked = g.locked;
            drop(g);
            if core.phase() == Phase::Recording && same_press && !locked {
                core.show_widget();
                core.play("Tink");
            }
        });
    }

    fn cancel_recording(self: &Arc<Self>) {
        if self.phase() != Phase::Recording {
            return;
        }
        let _ = self.recorder.stop();
        self.set_phase(Phase::Idle);
        self.hide_widget_later(Duration::ZERO);
    }

    fn finish_recording(self: &Arc<Self>) {
        if self.phase() != Phase::Recording {
            return;
        }
        self.set_phase(Phase::Transcribing);
        self.play("Pop");
        let core = self.clone();
        std::thread::spawn(move || {
            let result = core.process();
            core.set_phase(Phase::Idle);
            match result {
                Ok(hide_after) => core.hide_widget_later(hide_after),
                Err(e) => core.fail(&e.to_string()),
            }
        });
    }

    /// Records → text → inserted where it makes sense. Returns how long to keep the widget up.
    fn process(&self) -> anyhow::Result<Duration> {
        let quick = Duration::from_millis(350);
        let rec = self.recorder.stop()?;
        if rec.seconds() < MIN_SECONDS || rec.peak_rms < SILENCE_RMS {
            mlog!("skipped clip ({:.2}s, peak rms {:.4})", rec.seconds(), rec.peak_rms);
            return Ok(Duration::ZERO);
        }
        self.emit("transcribing", None, None);
        let transcriber = self.ensure_transcriber()?;
        let started = Instant::now();
        let cfg = self.cfg();
        let raw = transcriber.transcribe(&rec.samples, &cfg.language, cfg.translate, &cfg.vocabulary)?;
        let elapsed = started.elapsed().as_millis();
        mlog!("transcribed {:.1}s audio in {elapsed} ms", rec.seconds());
        if cleanup::is_hallucination(&raw) {
            return Ok(Duration::ZERO);
        }
        let text = cleanup::apply_replacements(&cleanup::clean(&raw, cfg.remove_fillers), &cfg.replacements);
        if text.is_empty() {
            return Ok(Duration::ZERO);
        }
        let copied = Duration::from_millis(2200);
        let hide_after = match focus::detect() {
            // Murmur's own window: the page puts it in the focused field or the scratchpad.
            focus::Target::Murmur if self.main_window_focused() => {
                let _ = self.app.emit_to("main", "dictation", &text);
                self.emit("done", None, None);
                quick
            }
            // Murmur is in front with no window of its own (a menu-bar app stays in front after
            // its window closes), or nothing can take text: keep it on the clipboard and say so
            // instead of a false ✓.
            focus::Target::Murmur | focus::Target::NotEditable { .. } => {
                paste::copy(&text)?;
                self.emit("copied", None, None);
                copied
            }
            // A text field: type it there (trailing space so dictations flow together).
            focus::Target::Editable => {
                paste::insert(&format!("{text} "), cfg.restore_clipboard)?;
                self.emit("done", None, None);
                quick
            }
            // Can't tell if anything will take the text: paste anyway, but keep it on the
            // clipboard so it is never lost.
            focus::Target::Unknown => {
                paste::insert(&format!("{text} "), false)?;
                self.emit("pasted", None, None);
                copied
            }
        };
        *self.last_text.lock().unwrap() = Some(text.clone());
        *self.last_used.lock().unwrap() = Instant::now();
        if cfg.save_history {
            history::append(&text, &raw, rec.seconds(), elapsed);
            let _ = self.app.emit_to("main", "history-updated", ());
        }
        Ok(hide_after)
    }

    fn fail(self: &Arc<Self>, message: &str) {
        mlog!("{message}");
        self.emit("error", Some(message.to_string()), None);
        self.show_widget();
        self.hide_widget_later(Duration::from_millis(2500));
    }

    /// Download (first run / model change) and load the Whisper model, then warm it up.
    fn load_model(self: &Arc<Self>) {
        let _guard = self.load_lock.lock().unwrap();
        self.set_phase(Phase::Loading);
        *self.transcriber.lock().unwrap() = None;
        let dir = config::data_dir();
        let needs_download = !model::model_path(&dir, &self.cfg().model).exists();
        if needs_download {
            self.emit("downloading", Some("Downloading speech model…".into()), Some(0.0));
            self.show_widget();
        }
        let path = model::ensure(&dir, &self.cfg().model, |done, total| {
            let pct = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            self.set_status(&format!("Downloading model… {:.0}%", pct * 100.0));
            self.emit("downloading", Some(format!("Downloading model {:.0}%", pct * 100.0)), Some(pct));
            self.set_download(Some(pct));
        });
        self.set_download(None);
        let path = match path {
            Ok(p) => p,
            Err(e) => {
                self.set_phase(Phase::Idle);
                self.set_status("Model download failed — pick the model again in Settings to retry");
                self.fail(&format!("Download failed: {e}"));
                return;
            }
        };
        self.set_status("Loading model…");
        match Transcriber::load(&path) {
            Ok(t) => {
                // First inference compiles GPU kernels; do it now instead of on the first dictation.
                let _ = t.transcribe(&vec![0.0; audio::TARGET_RATE as usize], &self.cfg().language, false, &[]);
                *self.transcriber.lock().unwrap() = Some(Arc::new(t));
                self.set_phase(Phase::Idle);
                *self.last_used.lock().unwrap() = Instant::now();
                self.ready_status();
                if needs_download {
                    self.emit("ready", Some(format!("Ready — hold {}", key_label(&self.cfg().hotkey))), None);
                    self.hide_widget_later(Duration::from_millis(2500));
                }
            }
            Err(e) => {
                self.set_phase(Phase::Idle);
                self.set_status("Model failed to load");
                self.fail(&format!("Model load failed: {e}"));
            }
        }
    }

    /// Download progress for the app window (setup shows it as a progress bar).
    fn set_download(&self, progress: Option<f32>) {
        *self.download.lock().unwrap() = progress;
        let _ = self.app.emit_to("main", "model-progress", progress);
    }

    /// Keeps retrying until the OS grants keyboard-listening permission.
    fn start_hotkey(self: &Arc<Self>) {
        // Presses are handled on a worker thread so the key tap's callback returns immediately;
        // macOS disables taps whose callbacks are slow (starting the mic takes a moment).
        let (tx, rx) = std::sync::mpsc::channel::<HotkeyEvent>();
        let worker = self.clone();
        std::thread::spawn(move || {
            for ev in rx {
                worker.on_hotkey(ev);
            }
        });
        loop {
            let core = self.clone();
            let tx = tx.clone();
            let handler = Box::new(move |ev: HotkeyEvent| match ev {
                // Esc needs an answer (swallow or not), and the check is instant.
                HotkeyEvent::Escape => core.on_hotkey(ev),
                _ => {
                    let _ = tx.send(ev);
                    false
                }
            });
            match hotkey::start(&self.cfg().hotkey, handler) {
                Ok(()) => return,
                Err(e) => {
                    mlog!("hotkey: {e}");
                    self.set_status("Grant Input Monitoring in System Settings → Privacy");
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        }
    }
}

fn key_label(key: &str) -> &str {
    match key {
        "fn" | "globe" => "Fn",
        "right_command" => "Right ⌘",
        "right_option" => "Right ⌥",
        "right_control" | "right_ctrl" => "Right Ctrl",
        "right_shift" => "Right Shift",
        "right_alt" => "Right Alt",
        "caps_lock" => "Caps Lock",
        other => other,
    }
}

/// Bottom-centre of the window being dictated into, so the widget shows up where you're
/// looking. Falls back to the screen under the mouse when the window can't be found.
/// A spot the user dragged it to wins over both.
fn place_widget(app: &AppHandle, win: &WebviewWindow) {
    if let Some(p) = widget_drag::pinned(app) {
        let _ = win.set_position(p);
        return;
    }
    const MARGIN: f64 = 24.0;
    let (Ok(size), Ok(scale)) = (win.outer_size(), win.scale_factor()) else { return };
    let (w, h) = (size.width as f64 / scale, size.height as f64 / scale);

    if let Some((x, y, fw, fh)) = focus::active_window_frame() {
        // Short windows (e.g. a one-line palette): sit just below them instead of covering them.
        let top = if fh > h + MARGIN * 3.0 { y + fh - h - MARGIN } else { y + fh + 8.0 };
        let _ = win.set_position(tauri::LogicalPosition::new(x + (fw - w) / 2.0, top));
        return;
    }

    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else {
        mlog!("widget: no monitor found to place it on");
        return;
    };
    let area = m.work_area();
    let margin = (28.0 * m.scale_factor()) as i32;
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

#[cfg(target_os = "macos")]
mod panel {
    use tauri::{Manager, WebviewWindow};
    use tauri_nspanel::{tauri_panel, CollectionBehavior, ManagerExt, StyleMask, WebviewWindowExt};

    tauri_panel! {
        panel!(WidgetPanel {
            config: {
                can_become_key_window: false,
                can_become_main_window: false,
                is_floating_panel: true
            }
        })
    }

    const STATUS_WINDOW_LEVEL: i64 = 25;

    /// Turns the widget window into a non-activating NSPanel: the only window type that can
    /// float over full-screen apps on every Space without stealing focus.
    pub fn convert(win: &WebviewWindow) -> tauri::Result<()> {
        let panel = win.to_panel::<WidgetPanel>()?;
        panel.set_level(STATUS_WINDOW_LEVEL);
        panel.set_collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary()
                .stationary()
                .ignores_cycle()
                .into(),
        );
        if let Err(e) = panel.add_style_mask(StyleMask::empty().nonactivating_panel().value()) {
            mlog!("widget: could not make panel non-activating: {e}");
        }
        // Click-through until the mouse is over what the widget shows (see widget_drag).
        panel.set_ignores_mouse_events(true);
        Ok(())
    }

    pub fn show(win: &WebviewWindow) -> bool {
        set_visible(win, true)
    }

    pub fn hide(win: &WebviewWindow) -> bool {
        set_visible(win, false)
    }

    fn set_visible(win: &WebviewWindow, visible: bool) -> bool {
        let app = win.app_handle().clone();
        if app.get_webview_panel(win.label()).is_err() {
            return false;
        }
        let label = win.label().to_string();
        let _ = win.run_on_main_thread(move || {
            if let Ok(panel) = app.get_webview_panel(&label) {
                if visible { panel.show() } else { panel.hide() }
            }
        });
        true
    }
}

/// Shows the widget without it becoming key or stealing focus from the app being dictated into.
fn show_without_focus(win: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    if panel::show(win) {
        return;
    }
    let _ = win.show();
}

fn hide_widget(win: &WebviewWindow) {
    widget_drag::release(win.app_handle());
    #[cfg(target_os = "macos")]
    if panel::hide(win) {
        return;
    }
    let _ = win.hide();
}

/// Opens (or focuses) the main window, optionally on a given tab ("home", "history", "settings").
/// Murmur stays a menu-bar app (no Dock icon) even while this window is open: macOS only lets
/// a menu-bar app's floating widget appear on other apps' full-screen Spaces.
fn open_main_window(app: &AppHandle, tab: Option<&str>) {
    if let Some(w) = app.get_webview_window("main") {
        if let Some(tab) = tab {
            let _ = w.emit("navigate", tab);
        }
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let url = format!("app.html#{}", tab.unwrap_or("home"));
    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App(url.into()))
        .title("Murmur")
        .inner_size(1000.0, 700.0)
        .min_inner_size(820.0, 560.0)
        .center();
    // Content runs under the traffic lights, like native Mac apps; the page provides drag regions.
    // Translucent window + the system sidebar material = native vibrancy behind the sidebar;
    // the page paints its own opaque background for the content area.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .transparent(true)
        .effects(
            tauri::window::EffectsBuilder::new()
                .effect(tauri::window::Effect::Sidebar)
                .state(tauri::window::EffectState::FollowsWindowActiveState)
                .build(),
        );
    let built = builder.build();
    match built {
        Ok(w) => {
            // Follow the user to their current Space instead of pulling them to this window's.
            #[cfg(target_os = "macos")]
            if let Ok(ns) = w.ns_window() {
                let ns = ns as usize;
                let _ = w.run_on_main_thread(move || unsafe { move_to_active_space(ns as *mut std::ffi::c_void) });
            }
            let _ = w.set_focus();
        }
        Err(e) => mlog!("could not open main window: {e}"),
    }
}

/// Opens a file or folder in its default app (Finder for folders). Failures are logged and
/// returned instead of silently ignored.
/// Adds NSWindowCollectionBehaviorMoveToActiveSpace to a window.
#[cfg(target_os = "macos")]
unsafe fn move_to_active_space(ns_window: *mut std::ffi::c_void) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    const MOVE_TO_ACTIVE_SPACE: usize = 1 << 1;
    let window = &*(ns_window as *mut AnyObject);
    let current: usize = msg_send![window, collectionBehavior];
    let _: () = msg_send![window, setCollectionBehavior: current | MOVE_TO_ACTIVE_SPACE];
}

fn open_path(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";
    if !path.exists() {
        let _ = std::fs::write(path, "");
    }
    let result = std::process::Command::new(cmd).arg(path).output();
    let error = match result {
        Ok(out) if out.status.success() => return Ok(()),
        Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
        Err(e) => e.to_string(),
    };
    mlog!("could not open {}: {error}", path.display());
    Err(error)
}

/// Short, quiet macOS system sound ("Tink", "Pop", …). Non-blocking; no-op elsewhere.
fn play_sound(name: &str) {
    #[cfg(target_os = "macos")]
    {
        let path = format!("/System/Library/Sounds/{name}.aiff");
        let _ = std::process::Command::new("afplay").args(["-v", "0.35", &path]).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = name;
}

/// Lets the widget page write into murmur.log.
#[tauri::command]
fn widget_log(message: String) {
    mlog!("widget js: {message}");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None));
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    let app = builder
        .invoke_handler(tauri::generate_handler![
            widget_log,
            widget_drag::widget_hit_rects,
            widget_drag::widget_drag,
            commands::get_state,
            commands::save_config,
            commands::history_list,
            commands::history_delete,
            commands::history_clear,
            commands::get_stats,
            commands::copy_text,
            commands::set_login,
            commands::open_privacy,
            commands::open_data_folder,
            commands::get_notes,
            commands::save_notes,
            commands::request_microphone,
            commands::download_model,
            commands::finish_setup,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let first_run = !config::config_path().exists();
            let cfg = config::load();
            let setup_done = cfg.setup_done;
            // On a fresh install the speech model is downloaded from the setup flow, where the
            // user can pick a smaller one first, instead of silently in the background.
            let wait_for_setup = !setup_done && !model::model_path(&config::data_dir(), &cfg.model).exists();
            #[cfg(target_os = "macos")]
            fn_key::sync(&cfg.hotkey);

            use tauri_plugin_autostart::ManagerExt;
            let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
            let open_item = MenuItem::with_id(app, "open", "Open Murmur…", true, Some("CmdOrCtrl+,"))?;
            let history_item = MenuItem::with_id(app, "history", "History…", true, None::<&str>)?;
            let copy_last = MenuItem::with_id(app, "copy_last", "Copy Last Dictation", true, None::<&str>)?;
            let login_enabled = app.autolaunch().is_enabled().unwrap_or(false);
            let login_item = CheckMenuItem::with_id(app, "login", "Start at Login", true, login_enabled, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Murmur", true, Some("CmdOrCtrl+Q"))?;
            let sep = || PredefinedMenuItem::separator(app);
            let menu = Menu::with_items(
                app,
                &[&status, &sep()?, &open_item, &history_item, &copy_last, &sep()?, &login_item, &quit],
            )?;

            TrayIconBuilder::with_id("murmur")
                .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .icon_as_template(true)
                .tooltip("Murmur — local dictation")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, ev| match ev.id.as_ref() {
                    "quit" => app.exit(0),
                    "open" => open_main_window(app, Some("home")),
                    "history" => open_main_window(app, Some("history")),
                    "copy_last" => {
                        let core = app.state::<Arc<Core>>();
                        let text = core.last_text.lock().unwrap().clone().or_else(history::last_text);
                        if let Some(text) = text {
                            let _ = paste::copy(&text);
                        }
                    }
                    "login" => {
                        let enabled = commands::set_login(app.clone(), !app.autolaunch().is_enabled().unwrap_or(false));
                        let _ = login_item.set_checked(enabled);
                    }
                    _ => {}
                })
                .build(app)?;

            app.manage(widget_drag::WidgetDrag::default());
            if let Some(w) = app.get_webview_window("widget") {
                let _ = w.set_ignore_cursor_events(true);
                #[cfg(target_os = "macos")]
                if let Err(e) = panel::convert(&w) {
                    mlog!("widget: panel conversion failed: {e}");
                }
                let handle = app.handle().clone();
                w.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(pos) = event {
                        widget_drag::on_moved(&handle, *pos);
                    }
                });
                let handle = app.handle().clone();
                std::thread::spawn(move || widget_drag::track(handle));
            }

            let core = Arc::new(Core {
                app: app.handle().clone(),
                cfg: RwLock::new(cfg),
                recorder: Recorder::new(),
                transcriber: Mutex::new(None),
                phase: Mutex::new(Phase::Loading),
                gesture: Mutex::new(Gesture::default()),
                last_text: Mutex::new(None),
                status_item: status,
                status: Mutex::new("Starting…".into()),
                load_lock: Mutex::new(()),
                last_used: Mutex::new(Instant::now()),
                download: Mutex::new(None),
            });

            let trusted = paste::has_permission(true);
            mlog!("started v{} — accessibility: {trusted}, hotkey: {}", env!("CARGO_PKG_VERSION"), core.cfg().hotkey);
            if !trusted {
                core.set_status("Allow Murmur in Privacy & Security → Accessibility");
                // Relaunch once permission is granted so the Fn key can be intercepted.
                let handle = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(2));
                    if paste::has_permission(false) {
                        mlog!("accessibility granted, restarting");
                        handle.restart();
                    }
                });
            }

            // Debug aid: `MURMUR_DEMO=1` cycles the widget through recording → transcribing → done.
            // `MURMUR_DEMO=<seconds>` waits that long first (default 2).
            if let Some(delay) = std::env::var("MURMUR_DEMO").ok() {
                let delay = delay.parse::<u64>().unwrap_or(2);
                let c = core.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(delay));
                    c.emit("recording", None, None);
                    c.show_widget();
                    for i in 0..90 {
                        let _ = c.app.emit_to("widget", "level", 0.02 + 0.05 * ((i as f32) * 0.4).sin().abs());
                        std::thread::sleep(Duration::from_millis(33));
                    }
                    c.emit("transcribing", None, None);
                    std::thread::sleep(Duration::from_secs(3));
                    c.emit("done", None, None);
                    std::thread::sleep(Duration::from_millis(1500));
                    if let Some(w) = c.widget() {
                        hide_widget(&w);
                    }
                });
            }

            if wait_for_setup {
                core.set_status("Finish setup to download the speech model");
            } else {
                let c = core.clone();
                std::thread::spawn(move || c.load_model());
            }
            let c = core.clone();
            std::thread::spawn(move || c.start_hotkey());
            let c = core.clone();
            std::thread::spawn(move || c.watch_idle());
            app.manage(core);
            // Onboarding: show the app window on first run or while a permission is missing.
            // Debug aid: `open --env MURMUR_TAB=insights Murmur.app` opens straight onto a page.
            let debug_tab = std::env::var("MURMUR_TAB").ok();
            if first_run || !trusted || !setup_done || debug_tab.is_some() {
                open_main_window(app.handle(), Some(debug_tab.as_deref().unwrap_or("home")));
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Murmur");

    app.run(|app, event| match event {
        // Tray app: keep running when the windows are closed.
        RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
        // Launching Murmur again (Spotlight, Finder, Dock) opens the app window.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => open_main_window(app, None),
        _ => {}
    });
}

/// Debug aid: `murmur --focus-test` logs what Murmur thinks is focused after 3 s.
pub fn cli_focus_test() {
    std::thread::sleep(Duration::from_secs(3));
    let target = focus::detect();
    mlog!("focus-test → {target:?}");
    println!("{target:?}");
}

/// Headless check of the speech pipeline: `murmur --transcribe file.wav`.
pub fn cli_transcribe(path: &str) -> anyhow::Result<()> {
    let cfg = config::load();
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.map(|v| v as f32 / max)).collect::<Result<_, _>>()?
        }
    };
    let ch = spec.channels as usize;
    let mono: Vec<f32> = interleaved.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect();
    let samples = audio::resample(&mono, spec.sample_rate, audio::TARGET_RATE);

    let model_path = model::ensure(&config::data_dir(), &cfg.model, |d, t| {
        eprint!("\rdownloading model {} / {} MB", d >> 20, t >> 20);
    })?;
    let t0 = Instant::now();
    let tr = Transcriber::load(&model_path)?;
    eprintln!("model loaded in {} ms", t0.elapsed().as_millis());
    let _ = tr.transcribe(&vec![0.0; audio::TARGET_RATE as usize], &cfg.language, false, &[]);

    let t1 = Instant::now();
    let raw = tr.transcribe(&samples, &cfg.language, cfg.translate, &cfg.vocabulary)?;
    let ms = t1.elapsed().as_millis();
    println!("audio:   {:.1} s", samples.len() as f32 / audio::TARGET_RATE as f32);
    println!("time:    {ms} ms");
    println!("raw:     {raw}");
    println!("cleaned: {}", cleanup::apply_replacements(&cleanup::clean(&raw, cfg.remove_fillers), &cfg.replacements));
    Ok(())
}
