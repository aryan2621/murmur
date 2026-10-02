//! Takes over the Fn/Globe key on macOS by setting "Press 🌐 key to" → Do Nothing, so the
//! emoji picker / input switcher / system dictation doesn't fire alongside Murmur.
//! The user's original choice is saved and restored if they move Murmur off the Fn key.

use std::process::Command;

const DOMAIN: &str = "com.apple.HIToolbox";
const KEY: &str = "AppleFnUsageType";
/// 0 = Do Nothing, 1 = Change Input Source, 2 = Show Emoji & Symbols, 3 = Start Dictation.
const DO_NOTHING: &str = "0";
/// Marker for "the key did not exist" (macOS default).
const UNSET: &str = "unset";

fn backup_path() -> std::path::PathBuf {
    crate::config::data_dir().join("fn_key_original.txt")
}

fn read() -> Option<String> {
    let out = Command::new("defaults").args(["read", DOMAIN, KEY]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn write(value: &str) {
    if value == UNSET {
        let _ = Command::new("defaults").args(["delete", DOMAIN, KEY]).status();
    } else {
        let _ = Command::new("defaults").args(["write", DOMAIN, KEY, "-int", value]).status();
    }
    // Applies keyboard preference changes without logging out.
    let _ = Command::new("/System/Library/PrivateFrameworks/SystemAdministration.framework/Resources/activateSettings")
        .arg("-u")
        .status();
}

/// Call at startup. Claims Fn when it's the hotkey; otherwise restores the user's setting.
pub fn sync(hotkey: &str) {
    let backup = backup_path();
    if hotkey == "fn" || hotkey == "globe" {
        let current = read();
        if current.as_deref() == Some(DO_NOTHING) {
            return;
        }
        if !backup.exists() {
            let _ = std::fs::write(&backup, current.as_deref().unwrap_or(UNSET));
        }
        write(DO_NOTHING);
        mlog!("set \"Press 🌐 key to\" → Do Nothing (was {})", current.as_deref().unwrap_or("default"));
    } else if let Ok(original) = std::fs::read_to_string(&backup) {
        write(original.trim());
        let _ = std::fs::remove_file(&backup);
        mlog!("restored original 🌐 key setting");
    }
}
