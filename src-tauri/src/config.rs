use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User settings, stored as editable JSON in the app data dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Hold-to-talk key for dictation.
    /// macOS: "fn", "right_option", "right_command", "right_control", "right_shift".
    /// Windows: "right_ctrl", "right_alt", "caps_lock".
    pub hotkey: String,
    /// Hold-to-ask / tap-to-talk key for the assistant (macOS); must differ from `hotkey`.
    pub assistant_hotkey: String,
    /// Whisper model id, e.g. "large-v3-turbo-q5_0", "small.en", "base.en". Used by both
    /// dictation and the assistant.
    pub model: String,
    /// Spoken language code ("en", "hi", ...) or "auto".
    pub language: String,
    /// Output English whatever language was spoken (Whisper's translate mode).
    pub translate: bool,
    /// Microphone name; `None` follows the system default input.
    pub input_device: Option<String>,
    /// Names, jargon and acronyms Whisper should spell correctly.
    pub vocabulary: Vec<String>,
    /// Fix-ups applied after transcription: whole-word, case-insensitive `from` → `to`.
    pub replacements: Vec<Replacement>,
    /// Strip "um", "uh", stutters and similar.
    pub remove_fillers: bool,
    /// Put back whatever was on the clipboard after pasting.
    pub restore_clipboard: bool,
    /// Keep a local log of everything dictated, and of the assistant's questions and answers.
    pub save_history: bool,
    /// Soft start/stop sounds.
    pub sounds: bool,
    /// Free the model's memory after this many idle minutes (0 = keep loaded).
    pub unload_after_minutes: u32,
    /// The first-run setup has been completed (or skipped to the end).
    pub setup_done: bool,

    // ---- Assistant ----
    /// The assistant key is listened for. Off: the key does nothing and no AI is loaded.
    pub assistant_enabled: bool,
    /// "builtin" (the AI bundled with Murmur) or "ollama" (an Ollama the user runs themselves).
    pub brain: String,
    /// Which built-in model: "8b" (the default) or "4b", lighter (see `model::BRAINS`).
    pub builtin_model: String,
    /// Where Ollama is listening.
    pub ollama_url: String,
    /// Any model from `ollama list`, e.g. "qwen3:8b", "qwen2.5:7b".
    pub llm_model: String,
    /// How long the AI model stays in memory after a question ("30m", "1h", "-1" = forever).
    pub keep_alive: String,
    /// Replaces the built-in personality / instructions when set.
    pub system_prompt: Option<String>,
    /// What the assistant calls itself.
    pub assistant_name: String,
    /// macOS voice name (`say -v '?'` lists them). Premium/Enhanced voices sound best:
    /// System Settings → Accessibility → Spoken Content → System Voice → Manage Voices.
    pub voice: String,
    /// Speaking rate in words per minute.
    pub speech_rate: u32,
    /// Speak answers aloud. Off = answers only appear in the widget.
    pub speak_replies: bool,
    /// Start a fresh conversation after this many minutes without a question.
    pub forget_after_minutes: u32,
    /// How many recent question/answer pairs are sent back to the model as context.
    pub history_turns: usize,
    /// Conversation mode (tap the key): this much silence after you speak sends the question.
    pub pause_seconds: f32,
    /// Conversation mode ends after this long without hearing you.
    pub conversation_timeout_seconds: u32,
    /// Mic level (RMS) that counts as speech. Raise it in a noisy room, lower it for a quiet voice.
    pub speech_threshold: f32,
    /// Let the assistant act on the Mac: open apps and websites, set timers and reminders,
    /// read the calendar, change the volume and so on.
    pub actions: bool,
    /// Let the assistant look things up online (web search, reading pages, weather). Only the
    /// search words and page addresses leave the Mac.
    pub web_access: bool,
    /// Place used for the weather when none is named ("Pune", "London"). Empty: the location
    /// the weather service guesses from your internet connection.
    pub location: String,
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
            vocabulary: vec![],
            replacements: vec![],
            remove_fillers: true,
            restore_clipboard: true,
            save_history: true,
            sounds: true,
            unload_after_minutes: 0,
            setup_done: false,
            assistant_hotkey: "right_option".into(),
            assistant_enabled: cfg!(target_os = "macos"),
            brain: "builtin".into(),
            builtin_model: crate::model::BRAINS[0].id.into(),
            ollama_url: "http://localhost:11434".into(),
            llm_model: "qwen3:8b".into(),
            keep_alive: "30m".into(),
            system_prompt: None,
            assistant_name: "Jarvis".into(),
            voice: "Daniel".into(),
            speech_rate: 195,
            speak_replies: true,
            forget_after_minutes: 5,
            history_turns: 8,
            pause_seconds: 1.2,
            conversation_timeout_seconds: 20,
            speech_threshold: 0.012,
            actions: true,
            web_access: true,
            location: String::new(),
        }
    }
}

impl Config {
    /// `keep_alive` in minutes; 0 means keep the model loaded.
    pub fn keep_alive_minutes(&self) -> u64 {
        let v = self.keep_alive.trim();
        if v.starts_with('-') {
            return 0;
        }
        let (number, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len()));
        let n: u64 = number.parse().unwrap_or(30);
        if unit.starts_with('h') { n * 60 } else { n }
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

pub fn log_path() -> PathBuf {
    data_dir().join("murmur.log")
}

/// Load the config, writing defaults on first run so the file is easy to find and edit.
/// New settings added in later versions are written back so they show up in the file.
pub fn load() -> Config {
    let path = config_path();
    let existing = std::fs::read_to_string(&path).ok();
    let cfg = match &existing {
        Some(s) => match serde_json::from_str(s) {
            Ok(cfg) => cfg,
            Err(e) => {
                // Leave a broken file alone so the user's edits aren't lost.
                mlog!("invalid config.json ({e}), using defaults");
                return Config::default();
            }
        },
        None => Config::default(),
    };
    if let Ok(s) = serde_json::to_string_pretty(&cfg) {
        if existing.as_deref() != Some(s.as_str()) {
            let _ = std::fs::write(&path, s);
        }
    }
    cfg
}

pub fn log(msg: &str) {
    use std::io::Write;
    eprintln!("[murmur] {msg}");
    let path = log_path();
    // Keep the log small: start over once it passes 1 MB.
    if std::fs::metadata(&path).map(|m| m.len() > 1 << 20).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {msg}", chrono::Local::now().format("%H:%M:%S%.3f"));
    }
}
