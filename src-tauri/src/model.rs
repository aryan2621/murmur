//! One-time download of ggml Whisper models from the official whisper.cpp Hugging Face repo.
//! This is the only network access Murmur ever makes.

use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

pub fn model_path(dir: &Path, model: &str) -> PathBuf {
    dir.join("models").join(format!("ggml-{model}.bin"))
}

/// Returns the local model path, downloading it first if needed.
/// `progress(downloaded, total)` is called roughly every megabyte.
pub fn ensure(dir: &Path, model: &str, progress: impl Fn(u64, u64)) -> Result<PathBuf> {
    let path = model_path(dir, model);
    if path.exists() {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let url = format!("{BASE_URL}/ggml-{model}.bin");
    let part = path.with_extension("bin.part");

    let client = reqwest::blocking::Client::builder().timeout(None).build()?;
    let mut resp = client.get(&url).send().with_context(|| format!("downloading {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("Model '{model}' not found ({})", resp.status()));
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
        return Err(anyhow!("Model download incomplete ({done} of {total} bytes)"));
    }
    std::fs::rename(&part, &path)?;
    progress(done, total.max(done));
    Ok(path)
}
