//! Parakeet TDT 0.6B v3 (INT8 ONNX) through transcribe-rs, which does the NeMo preprocessing
//! and TDT decoding. Multilingual (25 European languages, auto-detected); no translate mode.

use anyhow::{anyhow, Result};
use std::path::Path;
use std::sync::Mutex;
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
use transcribe_rs::onnx::Quantization;

pub struct Parakeet(Mutex<ParakeetModel>);

impl Parakeet {
    pub fn load(dir: &Path) -> Result<Self> {
        let model = ParakeetModel::load(dir, &Quantization::Int8).map_err(|e| anyhow!("Parakeet: {e}"))?;
        Ok(Self(Mutex::new(model)))
    }

    pub fn transcribe(&self, audio: &[f32], _language: &str, translate: bool, vocabulary: &[String]) -> Result<String> {
        if translate {
            return Err(anyhow!("Parakeet does not support translate-to-English mode; switch to Whisper or turn off Translate."));
        }
        let result = self
            .0
            .lock()
            .unwrap()
            .transcribe_with(audio, &ParakeetParams::default())
            .map_err(|e| anyhow!("Parakeet: {e}"))?;
        Ok(spell_vocabulary(result.text.trim(), vocabulary))
    }
}

/// Parakeet can't be prompted, so vocabulary only fixes the spelling of words it already heard
/// ("kubernetes" → "Kubernetes").
// shortcut: single-word terms only, matched case-insensitively; upgrade to fuzzy matching if misheard terms stay wrong.
fn spell_vocabulary(text: &str, vocabulary: &[String]) -> String {
    text.split(' ')
        .map(|word| {
            let core = word.trim_matches(|c: char| !c.is_alphanumeric());
            match vocabulary.iter().find(|v| !v.contains(' ') && !core.is_empty() && v.eq_ignore_ascii_case(core)) {
                Some(v) => word.replacen(core, v, 1),
                None => word.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
