use anyhow::Result;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::TARGET_RATE;

pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    pub fn load(model: &Path) -> Result<Self> {
        whisper_rs::install_logging_hooks();
        let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())?;
        Ok(Self { ctx })
    }

    /// `audio` must be mono 16 kHz. `vocabulary` biases spelling of names/jargon.
    /// With `translate`, any spoken language is output as English.
    pub fn transcribe(&self, audio: &[f32], language: &str, translate: bool, vocabulary: &[String]) -> Result<String> {
        let mut state = self.ctx.create_state()?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_translate(translate);
        params.set_n_threads(threads());
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        let prompt = if vocabulary.is_empty() {
            String::new()
        } else {
            format!("Vocabulary: {}.", vocabulary.join(", "))
        };
        if !prompt.is_empty() {
            params.set_initial_prompt(&prompt);
        }

        // whisper.cpp rejects clips under 1 s; pad short ones with silence.
        let min_len = TARGET_RATE as usize * 6 / 5;
        let padded;
        let input = if audio.len() < min_len {
            padded = [audio, &vec![0.0; min_len - audio.len()]].concat();
            &padded[..]
        } else {
            audio
        };

        state.full(params, input)?;
        let mut text = String::new();
        for segment in state.as_iter() {
            text.push_str(&segment.to_str_lossy()?);
        }
        Ok(text.trim().to_string())
    }
}

fn threads() -> i32 {
    std::thread::available_parallelism().map(|n| n.get().min(8) as i32).unwrap_or(4)
}
