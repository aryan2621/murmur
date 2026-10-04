//! One-time downloads of the models Murmur runs on this machine: the Whisper speech model
//! (from the official whisper.cpp Hugging Face repo), shared by dictation and the assistant, and
//! the assistant's built-in AI model (a GGUF file).

use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const WHISPER_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// A built-in AI model the assistant can run.
#[derive(serde::Serialize)]
pub struct BrainModel {
    pub id: &'static str,
    pub label: &'static str,
    #[serde(skip)]
    file: &'static str,
    #[serde(skip)]
    url: &'static str,
    pub size_mb: u32,
    /// Least memory (GB) the Mac should have to run it comfortably.
    pub min_ram_gb: u32,
    pub note: &'static str,
}

/// The built-in AI is Qwen3 8B, 4-bit, run without its "thinking" so answers start at once:
/// good at multi-step tasks like working a web page. Qwen3 4B Instruct (2507) is the lighter
/// choice for Macs with less memory.
pub const BRAINS: [BrainModel; 2] = [
    BrainModel {
        id: "8b",
        label: "Qwen3 8B",
        file: "qwen3-8b-q4_k_m",
        url: "https://huggingface.co/unsloth/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf",
        size_mb: 4795,
        min_ram_gb: 16,
        note: "The built-in AI. Accurate with multi-step tasks like clicking through pages and apps.",
    },
    BrainModel {
        id: "4b",
        label: "Qwen3 4B",
        file: "qwen3-4b-instruct-2507-q4_k_m",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        size_mb: 2382,
        min_ram_gb: 8,
        note: "Lighter and quicker, for Macs with less than 16 GB of memory. Weaker at multi-step tasks.",
    },
];

/// The model with this id (the first one if it's unknown).
pub fn brain(id: &str) -> &'static BrainModel {
    BRAINS.iter().find(|b| b.id == id).unwrap_or(&BRAINS[0])
}

/// The Mac's memory in GB.
#[cfg(target_os = "macos")]
pub fn ram_gb() -> u32 {
    let mut bytes: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let name = c"hw.memsize";
    extern "C" {
        fn sysctlbyname(name: *const std::ffi::c_char, old: *mut std::ffi::c_void, oldlen: *mut usize, new: *const std::ffi::c_void, newlen: usize) -> i32;
    }
    let ok = unsafe { sysctlbyname(name.as_ptr(), &mut bytes as *mut u64 as *mut _, &mut len, std::ptr::null(), 0) } == 0;
    if ok { (bytes / (1 << 30)) as u32 } else { 8 }
}

#[cfg(not(target_os = "macos"))]
pub fn ram_gb() -> u32 {
    8
}

pub fn model_path(dir: &Path, model: &str) -> PathBuf {
    dir.join("models").join(format!("ggml-{model}.bin"))
}

pub fn brain_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("models").join(format!("{}.gguf", brain(id).file))
}

/// The Whisper model's local path, downloading it first if needed.
/// `progress(downloaded, total)` is called roughly every megabyte.
pub fn ensure(dir: &Path, model: &str, progress: impl Fn(u64, u64)) -> Result<PathBuf> {
    let path = model_path(dir, model);
    download(&format!("{WHISPER_BASE_URL}/ggml-{model}.bin"), &path, progress).map_err(|e| anyhow!("Model '{model}': {e}"))?;
    Ok(path)
}

/// The AI model's local path, downloading it first if needed.
pub fn ensure_brain(dir: &Path, id: &str, progress: impl Fn(u64, u64)) -> Result<PathBuf> {
    let path = brain_path(dir, id);
    download(brain(id).url, &path, progress)?;
    Ok(path)
}

/// Downloads `url` to `path` unless it's already there. Writes to a `.part` file first, so an
/// interrupted download never looks finished.
fn download(url: &str, path: &Path, progress: impl Fn(u64, u64)) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let part = path.with_extension("part");

    let client = reqwest::blocking::Client::builder().timeout(None).build()?;
    let mut resp = client.get(url).send().with_context(|| format!("downloading {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("download failed ({})", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&part)?;
    let mut buf = vec![0u8; 256 * 1024];
    let (mut done, mut last) = (0u64, 0u64);
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if done - last >= 1 << 20 {
            last = done;
            progress(done, total);
        }
    }
    file.flush()?;
    if total > 0 && done != total {
        return Err(anyhow!("download incomplete ({done} of {total} bytes)"));
    }
    std::fs::rename(&part, path)?;
    progress(done, total.max(done));
    Ok(())
}
