mod whisper;

#[cfg(target_os = "macos")]
mod parakeet;

use anyhow::Result;
#[cfg(not(target_os = "macos"))]
use anyhow::anyhow;
use std::path::Path;

use crate::model;

pub use whisper::Whisper;

/// Loaded speech-to-text backend (Whisper or Parakeet).
pub enum SttBackend {
    Whisper(Whisper),
    #[cfg(target_os = "macos")]
    Parakeet(parakeet::Parakeet),
}

impl SttBackend {
    pub fn load(engine: &str, path: &Path) -> Result<Self> {
        match engine {
            "parakeet" => {
                #[cfg(target_os = "macos")]
                {
                    Ok(Self::Parakeet(parakeet::Parakeet::load(path)?))
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = path;
                    Err(anyhow!("Parakeet is only available on macOS"))
                }
            }
            _ => Ok(Self::Whisper(Whisper::load(path)?)),
        }
    }

    pub fn transcribe(&self, audio: &[f32], language: &str, translate: bool, vocabulary: &[String]) -> Result<String> {
        match self {
            Self::Whisper(w) => w.transcribe(audio, language, translate, vocabulary),
            #[cfg(target_os = "macos")]
            Self::Parakeet(p) => p.transcribe(audio, language, translate, vocabulary),
        }
    }

    pub fn warmup(&self, language: &str) -> Result<()> {
        let silence = vec![0.0f32; crate::audio::TARGET_RATE as usize];
        let _ = self.transcribe(&silence, language, false, &[])?;
        Ok(())
    }
}

/// Path the model for `engine`/`id` loads from.
pub fn model_path(engine: &str, dir: &Path, id: &str) -> std::path::PathBuf {
    match engine {
        "parakeet" => model::parakeet_dir(dir, id),
        _ => model::model_path(dir, id),
    }
}

/// Download (if needed) the model files for `engine`/`id`; returns the path to load.
pub fn ensure_model(engine: &str, id: &str, dir: &Path, progress: impl Fn(u64, u64)) -> Result<std::path::PathBuf> {
    match engine {
        "parakeet" => model::ensure_parakeet(dir, id, progress),
        _ => model::ensure(dir, id, progress),
    }
}

pub fn model_ready(engine: &str, dir: &Path, id: &str) -> bool {
    match engine {
        "parakeet" => model::parakeet_ready(dir, id),
        _ => model::model_path(dir, id).exists(),
    }
}

