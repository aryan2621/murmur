use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User settings, stored as editable JSON in the app data dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Hold-to-talk key.
    /// macOS: "fn", "right_option", "right_command", "right_control", "right_shift".
    /// Windows: "right_ctrl", "right_alt", "caps_lock".
    pub hotkey: String,
    /// Whisper model id, e.g. "large-v3-turbo-q5_0", "small.en", "base.en".
    pub model: String,
    /// Spoken language code ("en", "hi", ...) or "auto".
    pub language: String,
    /// Output English whatever language was spoken (Whisper's translate mode).
    pub translate: bool,
    /// Microphone name; `None` follows the system default input.
    pub input_device: Option<String>,
    /// Cancel out sound playing from the Mac's own speakers (music, videos) with Apple's voice
    /// processing. Only with the system default microphone; opens the mic ~0.25 s slower.
    pub echo_cancellation: bool,
    /// Names, jargon and acronyms Whisper should spell correctly.
    pub vocabulary: Vec<String>,
    /// Fix-ups applied after transcription: whole-word, case-insensitive `from` → `to`.
    pub replacements: Vec<Replacement>,
    /// Strip "um", "uh", stutters and similar.
    pub remove_fillers: bool,
    /// Put back whatever was on the clipboard after pasting.
    pub restore_clipboard: bool,
    /// Keep a local log of everything dictated.
    pub save_history: bool,
    /// Soft start/stop sounds.
    pub sounds: bool,
    /// Free the model's memory after this many idle minutes (0 = keep loaded).
    pub unload_after_minutes: u32,
    /// The first-run setup has been completed (or skipped to the end).
    pub setup_done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: if cfg!(target_os = "macos") { "fn" } else { "right_ctrl" }.into(),
            model: "large-v3-turbo-q5_0".into(),
            language: "en".into(),
            translate: false,
            input_device: None,
            echo_cancellation: true,
            vocabulary: vec![],
            replacements: vec![],
            remove_fillers: true,
            restore_clipboard: true,
            save_history: true,
            sounds: true,
            unload_after_minutes: 0,
            setup_done: false,
        }
    }
}

/// Root folder for config, models and history: `~/Library/Application Support/Murmur`.
///
/// It used to be named after the bundle id (`com.murmur.app`), but macOS treats any folder
/// ending in `.app` as an application, so Finder couldn't open it. The old folder is moved
/// here once.
pub fn data_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("Murmur");
        let legacy = base.join("com.murmur.app");
        if legacy.is_dir() && !dir.exists() {
            if let Err(e) = std::fs::rename(&legacy, &dir) {
                eprintln!("[murmur] could not move {} to {}: {e}", legacy.display(), dir.display());
                return legacy;
            }
        }
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn save(cfg: &Config) -> anyhow::Result<()> {
    std::fs::write(config_path(), serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

/// Load the config, writing defaults on first run so the file is easy to find and edit.
pub fn load() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            mlog!("invalid config.json ({e}), using defaults");
            Config::default()
        }),
        Err(_) => {
            let cfg = Config::default();
            if let Ok(s) = serde_json::to_string_pretty(&cfg) {
                let _ = std::fs::write(&path, s);
            }
            cfg
        }
    }
}

pub fn log(msg: &str) {
    use std::io::Write;
    eprintln!("[murmur] {msg}");
    let path = data_dir().join("murmur.log");
    // Keep the log small: start over once it passes 1 MB.
    if std::fs::metadata(&path).map(|m| m.len() > 1 << 20).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {msg}", chrono::Local::now().format("%H:%M:%S%.3f"));
    }
}
