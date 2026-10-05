/// Log to stderr and to `murmur.log` in the data dir (the app has no console when launched normally).
macro_rules! mlog {
    ($($t:tt)*) => { $crate::config::log(&format!($($t)*)) };
}

#[cfg(target_os = "macos")]
mod assistant;
mod audio;
mod cleanup;
mod commands;
mod config;
mod dictation;
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

#[cfg(target_os = "macos")]
use assistant::Assistant;
use audio::Recorder;
use config::Config;
use dictation::Dictation;
use hotkey::{Hotkey, HotkeyEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// Who the floating widget is showing for; each is placed where it's most useful.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Dictation,
    Assistant,
}

/// What dictation and the assistant share: settings, the mic, the speech model, the menu bar
/// status line and the floating widget. Only one of them uses the mic at a time.
pub struct Shared {
    app: AppHandle,
    cfg: RwLock<Config>,
    recorder: Recorder,
    transcriber: Mutex<Option<Arc<Transcriber>>>,
    /// Serialises model loads so a press during an idle reload doesn't load it twice.
    load_lock: Mutex<()>,
    /// The speech model is being downloaded or loaded (or waits for setup to download it).
    loading: AtomicBool,
    last_used: Mutex<Instant>,
    /// Download progress (0–1) of the speech ("speech") and AI ("brain") models, while downloading.
    downloads: Mutex<HashMap<&'static str, f32>>,
    status_item: MenuItem<Wry>,
    /// Same text as the menu bar status line, for the app window.
    status: Mutex<String>,
    /// Bumped every time the widget is shown, so a hide scheduled earlier can tell it's stale.
    widget_shown: AtomicU64,
}

impl Shared {
    fn cfg(&self) -> Config {
        self.cfg.read().unwrap().clone()
    }

    fn set_status(&self, text: &str) {
        let _ = self.status_item.set_text(text);
        *self.status.lock().unwrap() = text.to_string();
        let _ = self.app.emit_to("main", "status", text);
    }

    fn ready_status(&self) {
        if !paste::has_permission(false) {
            self.set_status("Allow Murmur in Privacy & Security → Accessibility");
            return;
        }
        let cfg = self.cfg();
        let dictate = key_label(&cfg.hotkey);
        if cfg!(target_os = "macos") && cfg.assistant_enabled {
            self.set_status(&format!("Ready — hold {dictate} to dictate, {} to ask", key_label(&cfg.assistant_hotkey)));
        } else {
            self.set_status(&format!("Ready — hold {dictate} to dictate"));
        }
    }

    fn loading(&self) -> bool {
        self.loading.load(Ordering::SeqCst)
    }

    fn model_loaded(&self) -> bool {
        self.transcriber.lock().unwrap().is_some()
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
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

    /// A recording has started: reload the model (if it was freed) while the user is talking.
    fn preload_transcriber(self: &Arc<Self>) {
        self.touch();
        if !self.model_loaded() {
            let shared = self.clone();
            std::thread::spawn(move || {
                if let Err(e) = shared.ensure_transcriber() {
                    mlog!("model reload failed: {e}");
                }
            });
        }
    }

    /// Frees the model after `unload_after_minutes` of inactivity. A transcription under way
    /// keeps its own reference, so it's never cut short.
    fn watch_idle(&self) {
        loop {
            std::thread::sleep(Duration::from_secs(30));
            let minutes = self.cfg().unload_after_minutes;
            if minutes == 0 || self.loading() {
                continue;
            }
            let idle = self.last_used.lock().unwrap().elapsed();
            if idle >= Duration::from_secs(minutes as u64 * 60) && self.transcriber.lock().unwrap().take().is_some() {
                mlog!("model unloaded after {minutes} idle minutes");
            }
        }
    }

    fn play(&self, sound: &str) {
        if self.cfg().sounds {
            play_sound(sound);
        }
    }

    fn widget(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window("widget")
    }

    fn show_widget(&self, mode: Mode) {
        let Some(w) = self.widget() else {
            mlog!("widget: window not found");
            return;
        };
        let shown = self.widget_shown.fetch_add(1, Ordering::SeqCst) + 1;
        if !widget_drag::dragging(&self.app) {
            place_widget(&self.app, &w, mode);
        }
        show_without_focus(&w);
        // Follow the active window while the widget is up: right after a switch the window in
        // front isn't settled yet (a Space still sliding in, an app still activating), and the
        // user may switch apps, swipe to another Space or move to another display mid-dictation.
        // Stops when the widget hides or is shown again (that showing starts its own follower).
        let app = self.app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(WIDGET_FOLLOW_MS));
            let Some(shared) = app.try_state::<Arc<Shared>>() else { return };
            if shared.widget_shown() != shown || !w.is_visible().unwrap_or(false) {
                return;
            }
            if !widget_drag::dragging(&app) {
                place_widget_quietly(&app, &w, mode);
            }
            #[cfg(target_os = "macos")]
            panel::keep_on_active_space(&w);
        });
    }

    /// How many times the widget has been shown; pass it to `hide_widget_since`.
    fn widget_shown(&self) -> u64 {
        self.widget_shown.load(Ordering::SeqCst)
    }

    fn hide_widget(&self) {
        if let Some(w) = self.widget() {
            hide_widget(&w);
        }
    }

    /// Hides the widget unless it was shown again (by either mode) after `shown` was read.
    fn hide_widget_since(&self, shown: u64) {
        if self.widget_shown() == shown {
            self.hide_widget();
        }
    }

    fn emit_widget(&self, state: &str, message: Option<String>, progress: Option<f32>) {
        let payload = serde_json::json!({ "mode": "dictation", "state": state, "message": message, "progress": progress });
        let _ = self.app.emit_to("widget", "widget-state", payload);
    }

    fn fail(self: &Arc<Self>, message: &str) {
        mlog!("{message}");
        self.emit_widget("error", Some(message.to_string()), None);
        self.show_widget(Mode::Dictation);
        self.hide_widget_later(Duration::from_millis(2500));
    }

    fn hide_widget_later(self: &Arc<Self>, delay: Duration) {
        let shared = self.clone();
        let shown = self.widget_shown();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            shared.hide_widget_since(shown);
        });
    }

    fn downloading(&self, which: &str) -> bool {
        self.downloads.lock().unwrap().contains_key(which)
    }

    /// Records and announces a model download's progress (`None` = finished or stopped).
    fn download_progress(&self, which: &'static str, progress: Option<f32>) {
        let mut downloads = self.downloads.lock().unwrap();
        match progress {
            Some(p) => downloads.insert(which, p),
            None => downloads.remove(which),
        };
        let _ = self.app.emit_to("main", "download-progress", serde_json::json!({ "model": which, "progress": progress }));
    }

    /// Download (first run / model change) and load the Whisper model, then warm it up.
    fn load_model(self: &Arc<Self>) {
        let _guard = self.load_lock.lock().unwrap();
        self.loading.store(true, Ordering::SeqCst);
        *self.transcriber.lock().unwrap() = None;
        let dir = config::data_dir();
        let name = self.cfg().model;
        let needs_download = !model::model_path(&dir, &name).exists();
        // During setup the app window shows the progress; afterwards the floating widget does.
        let widget = needs_download && self.cfg().setup_done;
        if needs_download {
            self.download_progress("speech", Some(0.0));
        }
        if widget {
            self.emit_widget("downloading", Some("Downloading speech model…".into()), Some(0.0));
            self.show_widget(Mode::Dictation);
        }
        let path = model::ensure(&dir, &name, |done, total| {
            let pct = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            self.set_status(&format!("Downloading speech model… {:.0}%", pct * 100.0));
            self.download_progress("speech", Some(pct));
            if widget {
                self.emit_widget("downloading", Some(format!("Downloading model {:.0}%", pct * 100.0)), Some(pct));
            }
        });
        self.download_progress("speech", None);
        let path = match path {
            Ok(p) => p,
            Err(e) => {
                self.loading.store(false, Ordering::SeqCst);
                self.set_status("Model download failed — pick the model again in Settings to retry");
                let _ = self.app.emit_to("main", "download-error", format!("Speech model: {e}"));
                if self.cfg().setup_done {
                    self.fail(&format!("Download failed: {e}"));
                }
                return;
            }
        };
        self.set_status("Loading speech model…");
        let started = Instant::now();
        match Transcriber::load(&path) {
            Ok(t) => {
                // First inference compiles GPU kernels; do it now instead of on the first dictation.
                let _ = t.transcribe(&vec![0.0; audio::TARGET_RATE as usize], &self.cfg().language, false, &[]);
                *self.transcriber.lock().unwrap() = Some(Arc::new(t));
                self.loading.store(false, Ordering::SeqCst);
                self.touch();
                mlog!("speech model {name} ready in {} ms", started.elapsed().as_millis());
                self.ready_status();
                if widget {
                    self.emit_widget("ready", Some(format!("Ready — hold {}", key_label(&self.cfg().hotkey))), None);
                    self.hide_widget_later(Duration::from_millis(2500));
                }
            }
            Err(e) => {
                self.loading.store(false, Ordering::SeqCst);
                self.set_status("Model failed to load");
                self.fail(&format!("Model load failed: {e}"));
            }
        }
    }
}

/// Everything the app runs: shared state plus the two things you can do with your voice.
pub struct Murmur {
    shared: Arc<Shared>,
    dictation: Arc<Dictation>,
    #[cfg(target_os = "macos")]
    assistant: Arc<Assistant>,
}

impl Murmur {
    /// Sends each key's events to its mode. Only one mode has the mic at a time: a press while
    /// the other mode is recording or working is ignored, except that dictating stops an
    /// assistant that is only talking or waiting in a conversation.
    /// Returns true when the event was consumed (only matters for Esc).
    fn on_hotkey(&self, key: Hotkey, event: HotkeyEvent) -> bool {
        match key {
            Hotkey::Dictation => {
                #[cfg(target_os = "macos")]
                if event == HotkeyEvent::Pressed && self.dictation.phase() == dictation::Phase::Idle {
                    use assistant::Phase;
                    let a = &self.assistant;
                    if a.phase() == Phase::Recording && !a.conversing() {
                        // Right Option is held for a question.
                        return false;
                    }
                    if a.conversing() || a.phase() == Phase::Responding {
                        mlog!("dictation takes over from the assistant");
                        a.stop_all();
                    }
                }
                self.dictation.on_hotkey(event)
            }
            Hotkey::Assistant => self.on_assistant_key(event),
        }
    }

    #[cfg(target_os = "macos")]
    fn on_assistant_key(&self, event: HotkeyEvent) -> bool {
        if event == HotkeyEvent::Pressed && self.dictation.phase() != dictation::Phase::Idle {
            return false;
        }
        self.assistant.on_hotkey(event)
    }

    /// No assistant here; its key is never listened for.
    #[cfg(not(target_os = "macos"))]
    fn on_assistant_key(&self, _event: HotkeyEvent) -> bool {
        false
    }

    /// The keys to listen for: dictation's, and the assistant's when it's on.
    fn hotkeys(&self) -> Vec<(String, Hotkey)> {
        let cfg = self.shared.cfg();
        let mut keys = vec![(cfg.hotkey.clone(), Hotkey::Dictation)];
        if cfg!(target_os = "macos") && cfg.assistant_enabled && cfg.assistant_hotkey != cfg.hotkey {
            keys.push((cfg.assistant_hotkey, Hotkey::Assistant));
        }
        keys
    }

    /// Keeps retrying until the OS grants keyboard-listening permission.
    fn start_hotkey(self: &Arc<Self>) {
        // Presses are handled on a worker thread so the key tap's callback returns immediately;
        // macOS disables taps whose callbacks are slow (starting the mic takes a moment).
        let (tx, rx) = std::sync::mpsc::channel::<(Hotkey, HotkeyEvent)>();
        let worker = self.clone();
        std::thread::spawn(move || {
            for (key, ev) in rx {
                worker.on_hotkey(key, ev);
            }
        });
        loop {
            let app = self.clone();
            let tx = tx.clone();
            let handler = Box::new(move |key: Hotkey, ev: HotkeyEvent| match ev {
                // Esc needs an answer (swallow or not), and the check is instant.
                HotkeyEvent::Escape => app.on_hotkey(key, ev),
                _ => {
                    let _ = tx.send((key, ev));
                    false
                }
            });
            match hotkey::start(&self.hotkeys(), handler) {
                Ok(()) => return,
                Err(e) => {
                    mlog!("hotkey: {e}");
                    self.shared.set_status("Grant Input Monitoring in System Settings → Privacy");
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

/// Where the widget goes: bottom-centre of the screen you're working on, the same spot every
/// time (like Capturita's camera bubble). It used to sit on the window being dictated into, but
/// that window can't be told reliably while Spaces slide or apps activate, so it landed on the
/// wrong window. The window in front only picks the screen; else the screen under the mouse.
/// A spot the user dragged it to wins.
/// How often the shown widget checks it's still on the active window's display.
const WIDGET_FOLLOW_MS: u64 = 400;

/// Places the widget for a new showing, logging how the display was chosen.
fn place_widget(app: &AppHandle, win: &WebviewWindow, mode: Mode) {
    place_widget_logged(app, win, mode, true);
}

/// Re-checks the placement while the widget is up; logs only when it has to move.
fn place_widget_quietly(app: &AppHandle, win: &WebviewWindow, mode: Mode) {
    place_widget_logged(app, win, mode, false);
}

fn place_widget_logged(app: &AppHandle, win: &WebviewWindow, mode: Mode, log: bool) {
    if let Some(p) = widget_drag::pinned(app) {
        if win.outer_position().ok() != Some(p) {
            let _ = win.set_position(p);
        }
        return;
    }
    let Ok(size) = win.outer_size() else { return };
    let frame = own_focused_window(app, log).or_else(|| focus::active_window_frame(log));
    let monitor = frame
        .and_then(|(x, y, w, h)| monitor_at_point(app, x + w / 2.0, y + h / 2.0))
        .or_else(|| {
            app.cursor_position()
                .ok()
                .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        })
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else {
        mlog!("widget: no monitor found to place it on");
        return;
    };
    let area = m.work_area();
    let margin = ((if mode == Mode::Dictation { 28.0 } else { 20.0 }) * m.scale_factor()) as i32;
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
    let target = PhysicalPosition::new(x, y);
    let moved = win.outer_position().ok() != Some(target);
    if log || moved {
        let display = m.name().cloned().unwrap_or_else(|| "a display".into());
        let why = if log { "placed" } else { "moved to follow the active window" };
        mlog!("widget: {why} on {display} at ({x}, {y}), window {frame:?}");
    }
    if moved {
        let _ = win.set_position(target);
    }
}

/// Murmur's app window when it's the one in front (the window lookup leaves out our own).
fn own_focused_window(app: &AppHandle, log: bool) -> Option<focus::Frame> {
    let main = app.get_webview_window("main")?;
    if !main.is_visible().unwrap_or(false) || !main.is_focused().unwrap_or(false) || main.is_minimized().unwrap_or(false) {
        return None;
    }
    let (p, size, s) = (main.outer_position().ok()?, main.outer_size().ok()?, main.scale_factor().ok()?);
    let f = (p.x as f64 / s, p.y as f64 / s, size.width as f64 / s, size.height as f64 / s);
    if log {
        mlog!("widget: on Murmur's window {f:?}");
    }
    Some(f)
}

/// The display containing a point in screen points (top-left origin). Monitor frames are in
/// pixels of their own scale, so each is converted back to points before comparing.
fn monitor_at_point(app: &AppHandle, x: f64, y: f64) -> Option<tauri::Monitor> {
    app.available_monitors().ok()?.into_iter().find(|m| {
        let (s, p, size) = (m.scale_factor(), m.position(), m.size());
        let (mx, my) = (p.x as f64 / s, p.y as f64 / s);
        x >= mx && x < mx + size.width as f64 / s && y >= my && y < my + size.height as f64 / s
    })
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

    /// On every Space, including other apps' full-screen ones, and left out of ⌘` and Exposé.
    fn behavior() -> objc2_app_kit::NSWindowCollectionBehavior {
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .stationary()
            .ignores_cycle()
            .into()
    }

    /// Turns the widget window into a non-activating NSPanel: the only window type that can
    /// float over full-screen apps on every Space without stealing focus.
    pub fn convert(win: &WebviewWindow) -> tauri::Result<()> {
        let panel = win.to_panel::<WidgetPanel>()?;
        panel.set_level(STATUS_WINDOW_LEVEL);
        panel.set_collection_behavior(behavior());
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
                if !visible {
                    panel.hide();
                    return;
                }
                // Set again before each showing: if anything reset them, the widget would open on
                // the desktop's Space instead of over the full-screen app being dictated into.
                let ns = panel.as_panel();
                let before = ns.collectionBehavior();
                if before != behavior() && before != follow_behavior() {
                    mlog!("widget: Spaces behavior was {:#x}, setting it again", before.0);
                }
                ns.setCollectionBehavior(behavior());
                ns.setLevel(STATUS_WINDOW_LEVEL as isize);
                panel.show();
                // Like Capturita's camera bubble: in front of everything, whichever app is active.
                ns.orderFrontRegardless();
            }
        });
        // Where it ended up is only known once the window server has caught up.
        let win = win.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            keep_on_active_space(&win);
        });
        true
    }

    /// The widget, converted to a panel after Tauri made it, doesn't always take "join all
    /// Spaces" from macOS: it then stays on the Space it was last shown on (the desktop, or
    /// Murmur's window) instead of over the app in front. If so, it's switched to moving to
    /// the active Space and ordered in again, which brings it to the Space in front. Called
    /// after each showing and while it's up, so switching Spaces mid-dictation is followed too.
    pub fn keep_on_active_space(win: &WebviewWindow) {
        let app = win.app_handle().clone();
        let label = win.label().to_string();
        let _ = win.run_on_main_thread(move || {
            let Ok(panel) = app.get_webview_panel(&label) else { return };
            let ns = panel.as_panel();
            if !ns.isVisible() || ns.isOnActiveSpace() {
                return;
            }
            // At most every 2 s: if macOS won't move it, retrying each check would make it flicker.
            static LAST_TRY: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
            let mut last = LAST_TRY.lock().unwrap();
            if last.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2)) {
                return;
            }
            *last = Some(std::time::Instant::now());
            ns.setCollectionBehavior(follow_behavior());
            ns.orderOut(None);
            ns.orderFrontRegardless();
            // Whether it arrived is only known later; the next check (every follow tick) sees it.
            mlog!("widget: wasn't on the Space in front; moving it there");
        });
    }

    /// Moves to whichever Space is in front each time it's ordered in, full-screen ones included.
    fn follow_behavior() -> objc2_app_kit::NSWindowCollectionBehavior {
        use objc2_app_kit::NSWindowCollectionBehavior as B;
        B::MoveToActiveSpace | B::FullScreenAuxiliary | B::IgnoresCycle
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

/// Opens (or focuses) the main window, optionally on a given tab ("home", "assistant", "history", …).
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

/// Opens a file or folder in its default app (Finder for folders). Failures are logged and
/// returned instead of silently ignored.
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

/// Opens a file in the default text editor, creating it if needed.
#[cfg(target_os = "macos")]
fn open_text_file(path: &std::path::Path) {
    if !path.exists() {
        let _ = std::fs::write(path, "");
    }
    match std::process::Command::new("open").arg("-t").arg(path).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => mlog!("could not open {}: {}", path.display(), String::from_utf8_lossy(&out.stderr).trim()),
        Err(e) => mlog!("could not open {}: {e}", path.display()),
    }
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

#[cfg(target_os = "macos")]
fn handlers() -> impl Fn(tauri::ipc::Invoke<Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        widget_log,
        widget_drag::widget_hit_rects,
        widget_drag::widget_drag,
        commands::get_state,
        commands::save_config,
        commands::history_list,
        commands::history_page,
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
        commands::assistant_history_list,
        commands::assistant_history_page,
        commands::assistant_history_delete,
        commands::assistant_history_clear,
        commands::download_brain,
        commands::download_voice,
        commands::preview_voice,
        commands::stop_speaking,
        commands::ask_text,
        commands::new_conversation,
        commands::open_voice_settings,
        commands::confirm_answer,
    ]
}

#[cfg(not(target_os = "macos"))]
fn handlers() -> impl Fn(tauri::ipc::Invoke<Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        widget_log,
        widget_drag::widget_hit_rects,
        widget_drag::widget_drag,
        commands::get_state,
        commands::save_config,
        commands::history_list,
        commands::history_page,
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
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None));
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    let app = builder
        .invoke_handler(handlers())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let first_run = !config::config_path().exists();
            #[allow(unused_mut)]
            let mut cfg = config::load();
            // Debug aid: the assistant's answers aren't spoken (not saved to the settings file).
            if std::env::var_os("MURMUR_QUIET").is_some() {
                cfg.speak_replies = false;
            }
            let setup_done = cfg.setup_done;
            // On a fresh install the speech model is downloaded from the setup flow, where the
            // user can pick a smaller one first, instead of silently in the background.
            let wait_for_setup = !setup_done && !model::model_path(&config::data_dir(), &cfg.model).exists();
            #[cfg(target_os = "macos")]
            fn_key::sync(if cfg.assistant_enabled && cfg.assistant_hotkey == "fn" { "fn" } else { &cfg.hotkey });

            use tauri_plugin_autostart::ManagerExt;
            let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
            let open_item = MenuItem::with_id(app, "open", "Open Murmur…", true, Some("CmdOrCtrl+,"))?;
            let history_item = MenuItem::with_id(app, "history", "History…", true, None::<&str>)?;
            let copy_last = MenuItem::with_id(app, "copy_last", "Copy Last Dictation", true, None::<&str>)?;
            let login_enabled = app.autolaunch().is_enabled().unwrap_or(false);
            let login_item = CheckMenuItem::with_id(app, "login", "Start at Login", true, login_enabled, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Murmur", true, Some("CmdOrCtrl+Q"))?;
            let sep = || PredefinedMenuItem::separator(app);
            #[cfg(target_os = "macos")]
            let menu = {
                let new_chat = MenuItem::with_id(app, "new", "New Conversation", true, Some("CmdOrCtrl+N"))?;
                let log = MenuItem::with_id(app, "log", "Open Log", true, None::<&str>)?;
                Menu::with_items(
                    app,
                    &[&status, &sep()?, &open_item, &history_item, &copy_last, &new_chat, &sep()?, &log, &login_item, &quit],
                )?
            };
            #[cfg(not(target_os = "macos"))]
            let menu = Menu::with_items(app, &[&status, &sep()?, &open_item, &history_item, &copy_last, &sep()?, &login_item, &quit])?;

            TrayIconBuilder::with_id("murmur")
                .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .icon_as_template(true)
                .tooltip("Murmur — local dictation and assistant")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, ev| match ev.id.as_ref() {
                    "quit" => app.exit(0),
                    "open" => open_main_window(app, Some("home")),
                    "history" => open_main_window(app, Some("history")),
                    "copy_last" => {
                        let murmur = app.state::<Arc<Murmur>>();
                        let text = murmur.dictation.last_text.lock().unwrap().clone().or_else(history::last_text);
                        if let Some(text) = text {
                            let _ = paste::copy(&text);
                        }
                    }
                    #[cfg(target_os = "macos")]
                    "new" => app.state::<Arc<Murmur>>().assistant.new_conversation(),
                    #[cfg(target_os = "macos")]
                    "log" => open_text_file(&config::log_path()),
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

            let shared = Arc::new(Shared {
                app: app.handle().clone(),
                cfg: RwLock::new(cfg),
                recorder: Recorder::new(),
                transcriber: Mutex::new(None),
                load_lock: Mutex::new(()),
                loading: AtomicBool::new(true),
                last_used: Mutex::new(Instant::now()),
                downloads: Mutex::new(HashMap::new()),
                status_item: status,
                status: Mutex::new("Starting…".into()),
                widget_shown: AtomicU64::new(0),
            });
            let murmur = Arc::new(Murmur {
                dictation: Arc::new(Dictation::new(shared.clone())),
                #[cfg(target_os = "macos")]
                assistant: Arc::new(Assistant::new(shared.clone())),
                shared: shared.clone(),
            });

            let trusted = paste::has_permission(true);
            let c = shared.cfg();
            mlog!(
                "started v{} — accessibility: {trusted}, dictation key: {}, assistant key: {} ({})",
                env!("CARGO_PKG_VERSION"),
                c.hotkey,
                c.assistant_hotkey,
                if c.assistant_enabled { "on" } else { "off" }
            );
            if !trusted {
                shared.set_status("Allow Murmur in Privacy & Security → Accessibility");
                // Relaunch once permission is granted so the keys can be intercepted.
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
                let d = murmur.dictation.clone();
                std::thread::spawn(move || d.demo(delay));
            }

            if wait_for_setup {
                shared.set_status("Finish setup to download the speech model");
            } else {
                let s = shared.clone();
                std::thread::spawn(move || s.load_model());
            }
            #[cfg(target_os = "macos")]
            {
                // Load the assistant's AI too, so the first question doesn't wait for it.
                let a = murmur.assistant.clone();
                std::thread::spawn(move || {
                    a.warm_up_llm();
                    // Debug aid: `MURMUR_ASK="set a 1 minute timer" Murmur` asks as if typed.
                    if let Ok(question) = std::env::var("MURMUR_ASK") {
                        std::thread::sleep(Duration::from_secs(1));
                        if let Err(e) = a.ask_typed(&question) {
                            mlog!("MURMUR_ASK: {e}");
                        }
                    }
                });
                // Free the built-in AI's memory after the idle time set in Settings.
                let a = murmur.assistant.clone();
                let s = shared.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(30));
                    if !a.brain.uses_ollama() {
                        a.brain.local.stop_if_idle(s.cfg().keep_alive_minutes());
                    }
                    a.speaker.unload_if_idle(s.cfg().keep_alive_minutes());
                });
                app.manage(murmur.assistant.clone());
            }
            let m = murmur.clone();
            std::thread::spawn(move || m.start_hotkey());
            let s = shared.clone();
            std::thread::spawn(move || s.watch_idle());
            app.manage(shared);
            app.manage(murmur.dictation.clone());
            app.manage(murmur);
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
        // Never leave the built-in AI running (and holding memory) after Murmur quits.
        #[cfg(target_os = "macos")]
        RunEvent::Exit => {
            if let Some(a) = app.try_state::<Arc<Assistant>>() {
                a.brain.local.stop();
                a.restore_sound();
            }
        }
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

/// Headless check of the voice: `murmur --say "It's raining."`.
#[cfg(target_os = "macos")]
pub fn cli_say(text: &str) {
    assistant::cli_say(text)
}

/// Headless check of the brain, actions and voice: `murmur --ask "what's the weather in Pune?"`.
#[cfg(target_os = "macos")]
pub fn cli_ask(question: &str) -> anyhow::Result<()> {
    assistant::cli_ask(question)
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
