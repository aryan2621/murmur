//! Shared handlers for the localhost REST API and MCP proxy.

use crate::{history, Murmur, Shared};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
pub struct StatusResponse {
    pub version: &'static str,
    pub model_loaded: bool,
    pub assistant_enabled: bool,
    pub stt_engine: String,
    pub stt_model: String,
}

#[derive(Deserialize)]
pub struct AskRequest {
    pub question: String,
}

#[derive(Deserialize)]
pub struct SpeakRequest {
    pub text: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Serialize)]
pub struct DictateResult {
    pub text: Option<String>,
}

#[derive(Serialize)]
pub struct StopResponse {
    pub ok: bool,
}

pub fn status(shared: &Shared) -> StatusResponse {
    let cfg = shared.cfg();
    StatusResponse {
        version: env!("CARGO_PKG_VERSION"),
        model_loaded: shared.model_loaded(),
        assistant_enabled: cfg.assistant_enabled,
        stt_engine: cfg.stt_engine.clone(),
        stt_model: cfg.effective_stt_model().into(),
    }
}

pub fn last_dictation(murmur: &Murmur) -> DictateResult {
    let text = murmur
        .dictation
        .last_text
        .lock()
        .unwrap()
        .clone()
        .or_else(history::last_text);
    DictateResult { text }
}

pub fn ask(murmur: &Murmur, question: &str) -> Result<(), String> {
    murmur.assistant.ask_typed(question.trim()).map_err(|e| e.to_string())
}

pub fn speak(murmur: &Murmur, text: &str, force: bool) -> Result<(), String> {
    let cfg = murmur.shared.cfg();
    if !force && !cfg.speak_replies {
        return Ok(());
    }
    murmur.assistant.speaker.say(text);
    Ok(())
}

pub fn stop(murmur: &Murmur) -> StopResponse {
    murmur.assistant.stop_all();
    StopResponse { ok: true }
}

/// An empty token never means "no auth": Settings generates one when the API is switched on.
pub fn check_auth(shared: &Arc<Shared>, auth: Option<&str>) -> Result<(), axum::http::StatusCode> {
    let token = shared.cfg().http_token;
    let given = auth.and_then(|h| h.strip_prefix("Bearer ")).unwrap_or("");
    // Constant-time compare, so response timing doesn't leak the token.
    let same = token.len() == given.len() && token.bytes().zip(given.bytes()).fold(0, |d, (a, b)| d | (a ^ b)) == 0;
    if token.is_empty() || !same {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }
    Ok(())
}
