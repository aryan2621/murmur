//! Commands invoked by the main app window (Home / Assistant / Insights / History / Settings).

// Every command is `async`: Tauri then runs it on a worker thread instead of the main thread,
// so one that takes a while (listing voices, saving settings that restart a model, previewing a
// voice) never freezes the window or swallows clicks.

use crate::config::{self, Config};
use crate::{audio, history, model, paste, Shared};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
#[cfg(target_os = "macos")]
use crate::assistant::{self, Assistant};
#[cfg(target_os = "macos")]
use tauri::Emitter;

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

#[cfg(target_os = "macos")]
#[derive(Serialize)]
pub struct BrainInfo {
    #[serde(flatten)]
    model: &'static model::BrainModel,
    downloaded: bool,
}

#[cfg(target_os = "macos")]
#[derive(Serialize)]
pub struct OllamaState {
    running: bool,
    models: Vec<String>,
    error: Option<String>,
}

#[cfg(target_os = "macos")]
#[derive(Serialize)]
pub struct NaturalVoice {
    id: &'static str,
    label: &'static str,
}

/// What the assistant needs shown: its AI, voices, and why it can't answer (if it can't).
#[cfg(target_os = "macos")]
#[derive(Serialize)]
pub struct AssistantState {
    unavailable: Option<String>,
    brain_ready: bool,
    brain_label: &'static str,
    brain_size_mb: u32,
    ollama: OllamaState,
    voices: Vec<assistant::speech::Voice>,
    /// The natural voice: downloaded yet, its download size, and its speakers.
    natural_ready: bool,
    natural_size_mb: u32,
    natural_voices: Vec<NaturalVoice>,
    /// The built-in AI models to choose from, and the Mac's memory to choose by.
    brains: Vec<BrainInfo>,
    ram_gb: u32,
}

#[derive(Serialize)]
pub struct AppState {
    config: Config,
    status: String,
    model_loaded: bool,
    /// Download progress (0–1) while the speech model is downloading.
    model_progress: Option<f32>,
    /// Download progress (0–1) of the speech ("speech") and AI ("brain") models.
    downloads: std::collections::HashMap<&'static str, f32>,
    permissions: Permissions,
    models: Vec<ModelInfo>,
    devices: Vec<String>,
    hotkeys: Vec<Choice>,
    login_enabled: bool,
    version: &'static str,
    data_dir: String,
    /// `None` where there is no assistant (Windows).
    #[cfg(target_os = "macos")]
    assistant: Option<AssistantState>,
    #[cfg(not(target_os = "macos"))]
    assistant: Option<()>,
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

#[cfg(target_os = "macos")]
fn assistant_state(a: &Assistant, cfg: &Config) -> AssistantState {
    let dir = config::data_dir();
    let ollama = if cfg.brain == "ollama" {
        match a.brain.ollama.models() {
            Ok(models) => OllamaState { running: true, models, error: None },
            Err(e) => OllamaState { running: false, models: vec![], error: Some(e.to_string()) },
        }
    } else {
        OllamaState { running: false, models: vec![], error: None }
    };
    AssistantState {
        unavailable: a.unavailable(),
        brain_ready: model::brain_path(&dir, &cfg.builtin_model).exists(),
        brain_label: model::brain(&cfg.builtin_model).label,
        brain_size_mb: model::brain(&cfg.builtin_model).size_mb,
        ollama,
        voices: assistant::speech::voices(),
        natural_ready: model::natural_ready(&dir),
        natural_size_mb: model::natural_size_mb(),
        natural_voices: model::NATURAL_VOICES.iter().map(|&(id, label)| NaturalVoice { id, label }).collect(),
        brains: model::BRAINS.iter().map(|b| BrainInfo { model: b, downloaded: model::brain_path(&dir, b.id).exists() }).collect(),
        ram_gb: model::ram_gb(),
    }
}

#[tauri::command(async)]
pub fn get_state(app: AppHandle, shared: State<'_, Arc<Shared>>) -> AppState {
    use tauri_plugin_autostart::ManagerExt;
    let cfg = shared.cfg();
    let dir = config::data_dir();
    let downloads = shared.downloads.lock().unwrap().clone();
    AppState {
        status: shared.status.lock().unwrap().clone(),
        model_loaded: shared.model_loaded(),
        model_progress: downloads.get("speech").copied(),
        downloads,
        #[cfg(target_os = "macos")]
        assistant: Some(assistant_state(&app.state::<Arc<Assistant>>(), &cfg)),
        #[cfg(not(target_os = "macos"))]
        assistant: None,
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

/// Saves and applies settings. A key change restarts the app (the key tap is set up once);
/// a model change downloads/loads it in the background; everything else applies immediately.
#[tauri::command(async)]
pub fn save_config(app: AppHandle, shared: State<'_, Arc<Shared>>, config: Config) -> Result<SaveResult, String> {
    if config.assistant_enabled && config.assistant_hotkey == config.hotkey {
        return Err("Dictation and the assistant need different keys".into());
    }
    let old = shared.cfg();
    config::save(&config).map_err(|e| e.to_string())?;
    *shared.cfg.write().unwrap() = config.clone();
    #[cfg(target_os = "macos")]
    app.state::<Arc<Assistant>>().inner().configure(&old, &config);
    let restarting = old.hotkey != config.hotkey
        || old.assistant_enabled != config.assistant_enabled
        || (config.assistant_enabled && old.assistant_hotkey != config.assistant_hotkey);
    let reloading_model = old.model != config.model && !restarting;
    if restarting {
        mlog!("keys changed (dictation {}, assistant {}), restarting", config.hotkey, config.assistant_hotkey);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            app.restart();
        });
    } else if reloading_model {
        mlog!("model changed to {}", config.model);
        let shared = shared.inner().clone();
        std::thread::spawn(move || shared.load_model());
    } else {
        shared.ready_status();
    }
    Ok(SaveResult { restarting, reloading_model })
}

/// Newest first. `limit` defaults to 500; Insights asks for everything.
#[tauri::command(async)]
pub fn history_list(query: String, limit: Option<usize>) -> Vec<history::Entry> {
    history::search(&query, limit.unwrap_or(500))
}

/// One page of history for the History page.
#[derive(Serialize)]
pub struct HistoryPage<T> {
    entries: Vec<T>,
    /// Matches in all, to count the pages.
    total: usize,
}

/// Page `page` (from 0) of `page_size` dictations matching `query`, newest first.
#[tauri::command(async)]
pub fn history_page(query: String, page: usize, page_size: usize) -> HistoryPage<history::Entry> {
    let (entries, total) = history::page(&query, page, page_size.max(1));
    HistoryPage { entries, total }
}

#[tauri::command(async)]
pub fn history_delete(time: String) -> Result<(), String> {
    history::delete(&time).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn history_clear() -> Result<(), String> {
    history::clear().map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn get_stats() -> history::Stats {
    history::stats()
}

#[tauri::command(async)]
pub fn copy_text(text: String) -> Result<(), String> {
    paste::copy(&text).map_err(|e| e.to_string())
}

#[tauri::command(async)]
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
#[tauri::command(async)]
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

#[tauri::command(async)]
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
#[tauri::command(async)]
pub fn get_notes() -> String {
    std::fs::read_to_string(notes_path()).unwrap_or_default()
}

#[tauri::command(async)]
pub fn save_notes(text: String) -> Result<(), String> {
    std::fs::write(notes_path(), text).map_err(|e| e.to_string())
}

/// Setup: makes macOS ask for microphone access (does nothing once it's been answered; if it
/// was denied, setup offers System Settings instead).
#[tauri::command(async)]
pub fn request_microphone() {
    #[cfg(target_os = "macos")]
    request_microphone_access();
}

/// Setup's download step: saves the chosen model and downloads/loads it in the background.
/// Progress arrives as `download-progress` events.
#[tauri::command(async)]
pub fn download_model(shared: State<'_, Arc<Shared>>, model: String) -> Result<(), String> {
    let mut cfg = shared.cfg();
    cfg.model = model;
    config::save(&cfg).map_err(|e| e.to_string())?;
    *shared.cfg.write().unwrap() = cfg;
    let shared = shared.inner().clone();
    std::thread::spawn(move || shared.load_model());
    Ok(())
}

/// Setup is complete (or skipped): don't show it on launch again.
#[tauri::command(async)]
pub fn finish_setup(shared: State<'_, Arc<Shared>>) -> Result<(), String> {
    let mut cfg = shared.cfg();
    cfg.setup_done = true;
    config::save(&cfg).map_err(|e| e.to_string())?;
    *shared.cfg.write().unwrap() = cfg;
    if !shared.loading() {
        shared.ready_status();
    }
    Ok(())
}

// ---- Assistant ----

/// Downloads the assistant's built-in AI (progress arrives as `download-progress`).
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn download_brain(assistant: State<'_, Arc<Assistant>>) {
    assistant.inner().download_brain();
}

/// Downloads the natural voice (progress arrives as `download-progress` for "natural").
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn download_voice(assistant: State<'_, Arc<Assistant>>) {
    assistant.inner().download_voice();
}

/// Says a sample sentence in `voice` so it can be compared before choosing it. `engine` is
/// "system" (a macOS voice) or "natural" (one of its speakers).
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn preview_voice(assistant: State<'_, Arc<Assistant>>, engine: Option<String>, voice: String, rate: u32) {
    let engine = engine.unwrap_or_else(|| "system".into());
    let short = voice.split(" (").next().unwrap_or(&voice);
    let text = if engine == "natural" {
        let name = model::NATURAL_VOICES.iter().find(|(id, _)| *id == voice).map_or("your assistant", |(_, label)| label.split(" ·").next().unwrap_or(label));
        format!("Hi, I'm {name}. Your meeting is at three thirty, and it may rain this evening.")
    } else {
        format!("Hi, I'm {short}. Your meeting is at three thirty, and it may rain this evening.")
    };
    assistant.speaker.preview(&engine, &voice, rate, &text);
}

#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn stop_speaking(assistant: State<'_, Arc<Assistant>>) {
    assistant.inner().stop_all();
}

/// Asks a typed question; the answer streams back through the `reply` event and is spoken.
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn ask_text(assistant: State<'_, Arc<Assistant>>, question: String) -> Result<(), String> {
    assistant.inner().ask_typed(question.trim()).map_err(|e| e.to_string())
}

/// The widget's Yes / No buttons while the assistant asks before a risky action.
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn confirm_answer(assistant: State<'_, Arc<Assistant>>, yes: bool) {
    if let Some(tx) = assistant.pending_confirm.lock().unwrap().as_ref() {
        let _ = tx.send(yes);
    }
}

#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn new_conversation(assistant: State<'_, Arc<Assistant>>) {
    assistant.inner().new_conversation();
}

/// Opens System Settings where Premium / Enhanced voices can be downloaded.
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn open_voice_settings() {
    let url = "x-apple.systempreferences:com.apple.Accessibility-Settings.extension?SpokenContent";
    let _ = std::process::Command::new("open").arg(url).spawn();
}

/// The assistant's questions and answers, newest first. `limit` defaults to 500.
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn assistant_history_list(query: String, limit: Option<usize>) -> Vec<assistant::history::Entry> {
    assistant::history::search(&query, limit.unwrap_or(500))
}

/// Page `page` (from 0) of `page_size` questions and answers matching `query`, newest first.
#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn assistant_history_page(query: String, page: usize, page_size: usize) -> HistoryPage<assistant::history::Entry> {
    let (entries, total) = assistant::history::page(&query, page, page_size.max(1));
    HistoryPage { entries, total }
}

#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn assistant_history_delete(app: AppHandle, time: String) -> Result<(), String> {
    assistant::history::delete(&time).map_err(|e| e.to_string())?;
    let _ = app.emit_to("main", "history-updated", ());
    Ok(())
}

#[cfg(target_os = "macos")]
#[tauri::command(async)]
pub fn assistant_history_clear(app: AppHandle) -> Result<(), String> {
    assistant::history::clear().map_err(|e| e.to_string())?;
    let _ = app.emit_to("main", "history-updated", ());
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
