//! Tells speech from other sound. Loudness alone can't: typing, clicks and knocks near the
//! MacBook's mic are often louder than your voice, and kept conversation mode listening long
//! after you stopped. Silero VAD (run by whisper.cpp, shipped inside the app) can.

use std::sync::Mutex;
use std::time::Duration;
use whisper_rs::{WhisperVadContext, WhisperVadContextParams};

const MODEL: &[u8] = include_bytes!("../../assets/ggml-silero-v5.1.2.bin");
/// Silero judges 32 ms of 16 kHz audio at a time.
pub const WINDOW: usize = 512;
pub const WINDOW_TIME: Duration = Duration::from_millis(32);
/// Audio judged on each look. The model starts each look from scratch and needs some context,
/// so this is longer than the gap between looks.
pub const CONTEXT: Duration = Duration::from_millis(1536);
/// A window this likely to be speech counts as speech.
const SPEECH_PROB: f32 = 0.5;

pub struct VoiceDetector {
    /// Loaded on first use; `None` inside if it couldn't be.
    ctx: Mutex<Option<Option<WhisperVadContext>>>,
}

impl VoiceDetector {
    pub fn new() -> Self {
        Self { ctx: Mutex::new(None) }
    }

    /// For each 32 ms window of `samples` (16 kHz), whether it is speech and how loud it is (peak
    /// RMS over 10 ms, like the mic level meter). `None` if the model couldn't be loaded.
    pub fn windows(&self, samples: &[f32]) -> Option<Vec<(bool, f32)>> {
        let mut slot = self.ctx.lock().unwrap();
        let ctx = slot.get_or_insert_with(load).as_mut()?;
        if let Err(e) = ctx.detect_speech(samples) {
            mlog!("speech detector failed: {e}");
            return None;
        }
        let probs = ctx.probabilities();
        Some(
            samples
                .chunks(WINDOW)
                .zip(probs)
                .map(|(w, p)| {
                    let level = w.chunks(160).map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt()).fold(0.0, f32::max);
                    (*p >= SPEECH_PROB, level)
                })
                .collect(),
        )
    }
}

fn load() -> Option<WhisperVadContext> {
    let path = crate::config::data_dir().join("models").join("ggml-silero-v5.1.2.bin");
    if std::fs::metadata(&path).map(|m| m.len() != MODEL.len() as u64).unwrap_or(true) {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        if let Err(e) = std::fs::write(&path, MODEL) {
            mlog!("speech detector: could not write its model: {e}; judging by loudness only");
            return None;
        }
    }
    let mut params = WhisperVadContextParams::default();
    params.set_n_threads(1);
    match WhisperVadContext::new(&path.to_string_lossy(), params) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            mlog!("speech detector: could not load ({e:?}); judging by loudness only");
            None
        }
    }
}
