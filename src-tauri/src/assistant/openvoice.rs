//! Optional OpenVoice sidecar for cloned assistant speech (MIT). Reference audio lives under
//! `~/Library/Application Support/Murmur/voices/{id}/`.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::config;

pub const DEFAULT_VOICE_ID: &str = "default";

pub fn voices_dir() -> PathBuf {
    config::data_dir().join("voices")
}

pub fn profile_dir(id: &str) -> PathBuf {
    voices_dir().join(id)
}

pub fn reference_path(id: &str) -> PathBuf {
    profile_dir(id).join("reference.wav")
}

pub fn profile_ready(id: &str) -> bool {
    reference_path(id).exists()
}

pub fn save_reference(id: &str, samples: &[f32]) -> Result<()> {
    std::fs::create_dir_all(profile_dir(id))?;
    crate::audio::write_wav(&reference_path(id), samples)
}

fn script_path() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let bundled = exe
        .parent()
        .ok_or_else(|| anyhow!("no exe dir"))?
        .join("../Resources/openvoice/speak.py");
    if bundled.exists() {
        return Ok(bundled);
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("openvoice/speak.py");
    if dev.exists() {
        return Ok(dev);
    }
    Err(anyhow!("OpenVoice script is missing from this build"))
}

/// Synthesize speech at 24 kHz mono for the shared `Player`.
pub fn synthesize(text: &str, voice_id: &str) -> Result<Vec<f32>> {
    // Once the sidecar says it can't run, stop spawning python3 for every sentence this session.
    static UNUSABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if UNUSABLE.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(anyhow!("OpenVoice is not available"));
    }
    let reference = reference_path(voice_id);
    if !reference.exists() {
        return Err(anyhow!("Record a voice sample in Settings → Voice → Custom clone first"));
    }
    let out = profile_dir(voice_id).join("last-clone.wav");
    let script = script_path()?;
    let status = Command::new("python3")
        .arg(script)
        .args(["--ref", &reference.to_string_lossy(), "--text", text, "--out", &out.to_string_lossy()])
        .status()
        .context("spawn OpenVoice sidecar")?;
    if matches!(status.code(), Some(2 | 3)) {
        UNUSABLE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    match status.code() {
        Some(0) => crate::audio::read_wav(&out, 24_000),
        Some(3) => Err(anyhow!("OpenVoice inference isn't wired up in this build yet")),
        Some(2) => Err(anyhow!(
            "Install OpenVoice for cloned speech: pip install openvoice-v2 (see Settings → Voice)"
        )),
        code => Err(anyhow!("OpenVoice sidecar failed (exit {code:?})")),
    }
}
