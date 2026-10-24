//! Optional voice cloning sidecar (`openvoice/speak.py`) for the assistant's speech: Chatterbox,
//! F5-TTS or OpenVoice, picked in Settings. Reference audio lives under
//! `~/Library/Application Support/Murmur/voices/{id}/`. Murmur installs the engines itself into
//! `voice-clone/` there: uv (one binary) fetches a private Python, the engine's packages and its
//! model weights, so nothing has to be set up in Terminal.

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

/// uv release used to set up the private Python.
const UV_VERSION: &str = "0.13.5";
const PYTHON_VERSION: &str = "3.11";

fn clone_dir() -> PathBuf {
    config::data_dir().join("voice-clone")
}

fn venv_python() -> PathBuf {
    clone_dir().join("venv/bin/python")
}

/// The Python package for an engine Murmur can install (`None`: manual setup only).
fn package(engine: &str) -> Option<&'static str> {
    match engine {
        "chatterbox" => Some("chatterbox-tts"),
        "f5" => Some("f5-tts"),
        _ => None,
    }
}

/// Installed by `install` (the marker is written last, so a half-finished install isn't ready).
pub fn engine_ready(engine: &str) -> bool {
    clone_dir().join(format!("ready-{engine}")).exists()
}

/// Keeps uv's Python, its cache and the engines' model weights inside `voice-clone/`.
fn private_env(cmd: &mut Command) -> &mut Command {
    let dir = clone_dir();
    cmd.env("UV_PYTHON_INSTALL_DIR", dir.join("python")).env("UV_CACHE_DIR", dir.join("cache")).env("HF_HOME", dir.join("models"))
}

/// Runs a setup step; on failure the error carries the end of what it printed.
fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.stdin(Stdio::null()).output().with_context(|| format!("{what}: couldn't start"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    mlog!("voice cloning setup: {what} failed:\n{err}");
    let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).rev().take(2).collect();
    Err(anyhow!("{what} failed: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" ")))
}

/// Downloads and sets up `engine` (a few GB, mostly PyTorch and the voice model), reporting
/// `progress(fraction, step)`. Safe to run again after a failure: finished steps are skipped.
pub fn install(engine: &str, progress: impl Fn(f32, &str)) -> Result<()> {
    let package = package(engine).ok_or_else(|| anyhow!("{engine} can't be installed from Murmur"))?;
    let dir = clone_dir();
    std::fs::create_dir_all(&dir)?;
    let uv = dir.join("uv");
    if !uv.exists() {
        progress(0.0, "Downloading the installer");
        let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
        let url = format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/uv-{arch}-apple-darwin.tar.gz");
        let tarball = dir.join("uv.tar.gz");
        let _ = std::fs::remove_file(&tarball);
        crate::model::download(&url, &tarball, |done, total| progress(0.05 * done as f32 / total.max(1) as f32, "Downloading the installer"))?;
        run(Command::new("tar").arg("xzf").arg(&tarball).arg("-C").arg(&dir).args(["--strip-components", "1"]), "Unpacking the installer")?;
        let _ = std::fs::remove_file(&tarball);
    }
    if !venv_python().exists() {
        progress(0.05, "Setting up Python");
        run(private_env(Command::new(&uv).args(["venv", "--python", PYTHON_VERSION]).arg(dir.join("venv"))), "Setting up Python")?;
    }
    progress(0.15, "Installing the voice engine (about 2 GB)");
    run(private_env(Command::new(&uv).args(["pip", "install", "--python"]).arg(venv_python()).arg(package)), "Installing the voice engine")?;
    progress(0.8, "Downloading the voice model");
    run(private_env(Command::new(venv_python()).arg(script_path()?).args(["--engine", engine, "--prefetch"])), "Downloading the voice model")?;
    std::fs::write(dir.join(format!("ready-{engine}")), UV_VERSION)?;
    progress(1.0, "Ready");
    Ok(())
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
/// The cloning engine chosen in Settings ("chatterbox", "f5", "openvoice").
static ENGINE: Mutex<String> = Mutex::new(String::new());
/// Once the sidecar says it can't run, stop spawning python3 for every sentence until the engine changes.
static UNUSABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Switches the cloning engine; the running sidecar is stopped if it was another one.
pub fn set_engine(engine: &str) {
    let mut current = ENGINE.lock().unwrap();
    if *current != engine {
        *current = engine.to_string();
        drop(current);
        UNUSABLE.store(false, std::sync::atomic::Ordering::Relaxed);
        unload();
    }
}

fn engine() -> String {
    let e = ENGINE.lock().unwrap().clone();
    if e.is_empty() { "chatterbox".into() } else { e }
}

fn spawn() -> Result<Sidecar> {
    // Murmur's own Python when the engine was installed from Settings, else the system one.
    let python = if venv_python().exists() { venv_python() } else { PathBuf::from("python3") };
    let mut child = private_env(&mut Command::new(python))
        .arg(script_path()?)
        .args(["--engine", &engine()])
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
    // A fresh start: an engine installed since it last failed gets another try.
    UNUSABLE.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Some(mut sc) = SIDECAR.lock().unwrap().take() {
        let _ = sc.child.kill();
        let _ = sc.child.wait();
    }
}

/// Synthesize speech at 24 kHz mono for the shared `Player`.
pub fn synthesize(text: &str, voice_id: &str) -> Result<Vec<f32>> {
    if UNUSABLE.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(anyhow!("Voice cloning ({}) is not available", engine()));
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
            Some(e) => Err(anyhow!("Voice cloning: {e}")),
        },
        Err(code) => {
            *slot = None;
            if matches!(code, Some(2..=4)) {
                UNUSABLE.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            match code {
                Some(3) => Err(anyhow!("OpenVoice checkpoints are missing: unzip checkpoints_v2 into Murmur/openvoice in Application Support")),
                Some(2) => Err(anyhow!("The voice cloning engine isn't installed yet: Settings → Voice → Install")),
                code => Err(anyhow!("Voice cloning sidecar failed (exit {code:?})")),
            }
        }
    }
}

