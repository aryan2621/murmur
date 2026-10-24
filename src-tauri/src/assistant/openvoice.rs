//! Optional OpenVoice sidecar for cloned assistant speech (MIT). Reference audio lives under
//! `~/Library/Application Support/Murmur/voices/{id}/`.

use anyhow::{anyhow, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

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

/// The sidecar process, kept running so its models load once instead of for every sentence.
struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

static SIDECAR: Mutex<Option<Sidecar>> = Mutex::new(None);

fn spawn() -> Result<Sidecar> {
    let mut child = Command::new("python3")
        .arg(script_path()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawn OpenVoice sidecar")?;
    let stdin = child.stdin.take().ok_or_else(|| anyhow!("no sidecar stdin"))?;
    let stdout = BufReader::new(child.stdout.take().ok_or_else(|| anyhow!("no sidecar stdout"))?);
    Ok(Sidecar { child, stdin, stdout })
}

/// Sends one request and waits for its reply line. `Err` with the exit code if the sidecar died.
fn request(sc: &mut Sidecar, req: &serde_json::Value) -> std::result::Result<serde_json::Value, Option<i32>> {
    let dead = |sc: &mut Sidecar| sc.child.wait().ok().and_then(|s| s.code());
    if writeln!(sc.stdin, "{req}").and_then(|_| sc.stdin.flush()).is_err() {
        return Err(dead(sc));
    }
    let mut line = String::new();
    loop {
        line.clear();
        match sc.stdout.read_line(&mut line) {
            Ok(0) | Err(_) => return Err(dead(sc)),
            Ok(_) => {
                if let Ok(reply) = serde_json::from_str::<serde_json::Value>(&line) {
                    return Ok(reply);
                }
            }
        }
    }
}

/// Stops the sidecar and gives its memory back; it starts again on the next cloned sentence.
pub fn unload() {
    if let Some(mut sc) = SIDECAR.lock().unwrap().take() {
        let _ = sc.child.kill();
        let _ = sc.child.wait();
    }
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
    let req = serde_json::json!({ "ref": reference, "text": text, "out": out });
    let mut slot = SIDECAR.lock().unwrap();
    if slot.is_none() {
        *slot = Some(spawn()?);
    }
    match request(slot.as_mut().unwrap(), &req) {
        Ok(reply) => match reply["error"].as_str() {
            None => crate::audio::read_wav(&out, 24_000),
            Some(e) => Err(anyhow!("OpenVoice: {e}")),
        },
        Err(code) => {
            *slot = None;
            if matches!(code, Some(2 | 3)) {
                UNUSABLE.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            match code {
                Some(3) => Err(anyhow!("OpenVoice checkpoints are missing: unzip checkpoints_v2 into Murmur/openvoice in Application Support")),
                Some(2) => Err(anyhow!("Install OpenVoice for cloned speech: pip install openvoice-v2 melotts (see Settings → Voice)")),
                code => Err(anyhow!("OpenVoice sidecar failed (exit {code:?})")),
            }
        }
    }
}
