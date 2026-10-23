//! Speech backend smoke tests (Whisper when a model is present on disk).

use murmur_lib::config;
use murmur_lib::model;
use murmur_lib::stt::SttBackend;
use std::f32::consts::PI;

#[test]
fn config_migrates_stt_model_from_legacy_model_field() {
    let mut cfg = config::Config::default();
    cfg.stt_model.clear();
    cfg.model = "small.en".into();
    cfg.migrate_from_legacy();
    assert_eq!(cfg.effective_stt_model(), "small.en");
}

#[test]
#[ignore = "downloads the tiny.en Whisper model"]
fn whisper_runs_on_a_tone() {
    // Models are shared with the app's data folder; the user's config is never read or written.
    let path = model::ensure(&config::data_dir(), "tiny.en", |_, _| {}).expect("download tiny.en");
    let backend = SttBackend::load("whisper", &path).expect("load whisper");
    let samples: Vec<f32> = (0..24_000).map(|i| (2.0 * PI * 440.0 * i as f32 / 16_000.0).sin() * 0.2).collect();
    backend.transcribe(&samples, "en", false, &[]).expect("transcribe");
}
