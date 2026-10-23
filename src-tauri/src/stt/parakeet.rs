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

    pub fn transcribe(&self, audio: &[f32], _language: &str, translate: bool, _vocabulary: &[String]) -> Result<String> {
        if translate {
            return Err(anyhow!("Parakeet does not support translate-to-English mode; switch to Whisper or turn off Translate."));
        }
        let result = self
            .0
            .lock()
            .unwrap()
            .transcribe_with(audio, &ParakeetParams::default())
            .map_err(|e| anyhow!("Parakeet: {e}"))?;
        Ok(result.text.trim().to_string())
    }
}
