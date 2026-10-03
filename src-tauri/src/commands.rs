//! Commands invoked by the main app window (Home / History / Settings).

use crate::config::{self, Config};
use crate::{audio, history, model, paste, Core};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct ModelInfo {
    id: &'static str,
    label: &'static str,
    size_mb: u32,
    note: &'static str,
    downloaded: bool,
}

const MODELS: &[(&str, &str, u32, &str)] = &[
    ("large-v3-turbo-q5_0", "Large v3 Turbo", 547, "Recommended: best accuracy for its speed. No translate mode."),
    ("large-v3-turbo", "Large v3 Turbo (full)", 1620, "Marginally more accurate, 3× the size."),
    ("large-v3-q5_0", "Large v3", 1080, "Slower, supports translate to English."),
    ("small", "Small (multilingual)", 466, "Fast; good for Hindi and translate."),
    ("small.en", "Small (English)", 466, "Fast, English only."),
    ("base.en", "Base (English)", 142, "Very fast, less accurate."),
    ("tiny.en", "Tiny (English)", 75, "Fastest, for older machines."),
];

#[derive(Serialize)]
pub struct Choice {
    id: &'static str,
    label: &'static str,
}

#[derive(Serialize)]
pub struct Permissions {
    accessibility: bool,
    /// "granted" | "denied" | "not_asked" | "unknown"
    microphone: &'static str,
}

#[derive(Serialize)]
pub struct AppState {
    config: Config,
    status: String,
    model_loaded: bool,
    /// Download progress (0–1) while the speech model is downloading.
    model_progress: Option<f32>,
    permissions: Permissions,
    models: Vec<ModelInfo>,
    devices: Vec<String>,
    hotkeys: Vec<Choice>,
    login_enabled: bool,
    version: &'static str,
    data_dir: String,
}

#[derive(Serialize)]
pub struct SaveResult {
    restarting: bool,
    reloading_model: bool,
}

fn hotkeys() -> Vec<Choice> {
    if cfg!(target_os = "macos") {
        vec![
            Choice { id: "fn", label: "Fn / 🌐" },
            Choice { id: "right_option", label: "Right Option ⌥" },
            Choice { id: "right_command", label: "Right Command ⌘" },
            Choice { id: "right_control", label: "Right Control ⌃" },
            Choice { id: "right_shift", label: "Right Shift ⇧" },
        ]
    } else {
        vec![
            Choice { id: "right_ctrl", label: "Right Ctrl" },
            Choice { id: "right_alt", label: "Right Alt" },
            Choice { id: "caps_lock", label: "Caps Lock" },
        ]
    }
}

#[tauri::command]
pub fn get_state(app: AppHandle, core: State<'_, Arc<Core>>) -> AppState {
    use tauri_plugin_autostart::ManagerExt;
    let cfg = core.cfg();
    let dir = config::data_dir();
    AppState {
        status: core.status.lock().unwrap().clone(),
        model_loaded: core.transcriber.lock().unwrap().is_some(),
        model_progress: *core.download.lock().unwrap(),
        permissions: Permissions { accessibility: paste::has_permission(false), microphone: microphone_status() },
        models: MODELS
            .iter()
            .map(|&(id, label, size_mb, note)| ModelInfo {
                id,
                label,
                size_mb,
                note,
                downloaded: model::model_path(&dir, id).exists(),
            })
            .collect(),
        devices: audio::input_device_names(),
        hotkeys: hotkeys(),
        login_enabled: app.autolaunch().is_enabled().unwrap_or(false),
        version: env!("CARGO_PKG_VERSION"),
        data_dir: dir.display().to_string(),
        config: cfg,
    }
}

/// Saves and applies settings. A hotkey change restarts the app (the key tap is set up once);
/// a model change downloads/loads it in the background; everything else applies immediately.
#[tauri::command]
pub fn save_config(app: AppHandle, core: State<'_, Arc<Core>>, config: Config) -> Result<SaveResult, String> {
    let old = core.cfg();
    config::save(&config).map_err(|e| e.to_string())?;
    *core.cfg.write().unwrap() = config.clone();
    let restarting = old.hotkey != config.hotkey;
    let reloading_model = old.model != config.model && !restarting;
    if restarting {
        mlog!("hotkey changed to {}, restarting", config.hotkey);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            app.restart();
        });
    } else if reloading_model {
        mlog!("model changed to {}", config.model);
        let core = core.inner().clone();
        std::thread::spawn(move || core.load_model());
    }
    Ok(SaveResult { restarting, reloading_model })
}

/// Newest first. `limit` defaults to 500; Insights asks for everything.
#[tauri::command]
pub fn history_list(query: String, limit: Option<usize>) -> Vec<history::Entry> {
    history::search(&query, limit.unwrap_or(500))
}

#[tauri::command]
pub fn history_delete(time: String) -> Result<(), String> {
    history::delete(&time).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_clear() -> Result<(), String> {
    history::clear().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_stats() -> history::Stats {
    history::stats()
}

#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    paste::copy(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_login(app: AppHandle, enabled: bool) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    let auto = app.autolaunch();
    let result = if enabled { auto.enable() } else { auto.disable() };
    if let Err(e) = result {
        mlog!("start at login: {e}");
    }
    auto.is_enabled().unwrap_or(false)
}

/// Shows the system permission prompt (if not decided yet) and opens the right Settings pane.
#[tauri::command]
pub fn open_privacy(pane: String) {
    if pane == "accessibility" {
        paste::has_permission(true);
    }
    #[cfg(target_os = "macos")]
    {
        let anchor = match pane.as_str() {
            "microphone" => "Privacy_Microphone",
            _ => "Privacy_Accessibility",
        };
        let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}

#[tauri::command]
pub fn open_data_folder() -> Result<(), String> {
    crate::open_path(&config::data_dir())
}

#[cfg(target_os = "macos")]
fn microphone_status() -> &'static str {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;
    use objc2_foundation::NSString;

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    let Some(class) = AnyClass::get(c"AVCaptureDevice") else { return "unknown" };
    // AVMediaTypeAudio
    let media = NSString::from_str("soun");
    let status: isize = unsafe { msg_send![class, authorizationStatusForMediaType: &*media] };
    match status {
        0 => "not_asked",
        1 | 2 => "denied",
        3 => "granted",
        _ => "unknown",
    }
}

#[cfg(not(target_os = "macos"))]
fn microphone_status() -> &'static str {
    "granted"
}

fn notes_path() -> std::path::PathBuf {
    config::data_dir().join("notes.txt")
}

/// The Home scratchpad, stored as plain text on this machine.
#[tauri::command]
pub fn get_notes() -> String {
    std::fs::read_to_string(notes_path()).unwrap_or_default()
}

#[tauri::command]
pub fn save_notes(text: String) -> Result<(), String> {
    std::fs::write(notes_path(), text).map_err(|e| e.to_string())
}

/// Setup: makes macOS ask for microphone access (does nothing once it's been answered; if it
/// was denied, setup offers System Settings instead).
#[tauri::command]
pub fn request_microphone() {
    #[cfg(target_os = "macos")]
    request_microphone_access();
}

/// Setup's download step: saves the chosen model and downloads/loads it in the background.
/// Progress arrives as `model-progress` events.
#[tauri::command]
pub fn download_model(core: State<'_, Arc<Core>>, model: String) -> Result<(), String> {
    let mut cfg = core.cfg();
    cfg.model = model;
    config::save(&cfg).map_err(|e| e.to_string())?;
    *core.cfg.write().unwrap() = cfg;
    let core = core.inner().clone();
    std::thread::spawn(move || core.load_model());
    Ok(())
}

#[tauri::command]
pub fn finish_setup(core: State<'_, Arc<Core>>) -> Result<(), String> {
    let mut cfg = core.cfg();
    cfg.setup_done = true;
    config::save(&cfg).map_err(|e| e.to_string())?;
    *core.cfg.write().unwrap() = cfg;
    Ok(())
}

#[cfg(target_os = "macos")]
/// Asks macOS for microphone access through AVFoundation, the same API the status is read from,
/// so the app sees the answer straight away (opening the mic via Core Audio also triggers the
/// prompt, but the status only updates after a restart). Shows the system prompt the first time.
fn request_microphone_access() {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, Bool};
    use objc2_foundation::NSString;

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    let Some(class) = AnyClass::get(c"AVCaptureDevice") else { return };
    // AVMediaTypeAudio
    let media = NSString::from_str("soun");
    let handler = RcBlock::new(|granted: Bool| mlog!("microphone access {}", if granted.as_bool() { "granted" } else { "denied" }));
    unsafe {
        let _: () = msg_send![class, requestAccessForMediaType: &*media, completionHandler: &*handler];
    }
}
