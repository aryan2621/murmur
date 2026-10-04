//! Talks to a local Ollama server. Replies are streamed token by token so speech can start
//! before the whole answer is written; decisions come back whole, shaped by a JSON schema.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::RwLock;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

impl Message {
    fn new(role: &'static str, content: impl Into<String>) -> Self {
        Self { role, content: content.into() }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }

    /// The chat format both llama.cpp's server and Ollama take.
    pub fn to_json(&self) -> Value {
        json!({ "role": self.role, "content": self.content })
    }
}

#[derive(Clone)]
struct Settings {
    base: String,
    model: String,
    keep_alive: String,
}

pub struct Ollama {
    settings: RwLock<Settings>,
    client: reqwest::blocking::Client,
}

impl Ollama {
    pub fn new(base: &str, model: &str, keep_alive: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            // Streams can run long; a cold model load alone can take ~10 s.
            .timeout(None)
            .build()
            .expect("http client");
        let ollama = Self { settings: RwLock::new(Settings { base: String::new(), model: String::new(), keep_alive: String::new() }), client };
        ollama.configure(base, model, keep_alive);
        ollama
    }

    /// Applies changed settings; the next request uses them.
    pub fn configure(&self, base: &str, model: &str, keep_alive: &str) {
        *self.settings.write().unwrap() =
            Settings { base: base.trim_end_matches('/').to_string(), model: model.to_string(), keep_alive: keep_alive.to_string() };
    }

    fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    /// Names of the installed models (`ollama list`). Errors when Ollama isn't running.
    pub fn models(&self) -> Result<Vec<String>> {
        let base = self.settings().base;
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(2)).build()?;
        let tags: Value = client.get(format!("{base}/api/tags")).send().map_err(|e| self.explain(e))?.json()?;
        let mut names: Vec<String> =
            tags["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str().map(str::to_string)).collect();
        names.sort();
        Ok(names)
    }

    /// Streams a reply. `on_text` gets each new piece and returns false to stop early
    /// (dropping the connection makes Ollama stop generating).
    pub fn chat(&self, messages: &[Message], temperature: f32, mut on_text: impl FnMut(&str) -> bool) -> Result<()> {
        let resp = self.post_chat(messages, json!({ "temperature": temperature }), None, true)?;
        for line in BufReader::new(resp).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let chunk: Value = serde_json::from_str(&line)?;
            if let Some(err) = chunk["error"].as_str() {
                return Err(anyhow!("Ollama: {err}"));
            }
            if let Some(text) = chunk["message"]["content"].as_str() {
                if !text.is_empty() && !on_text(text) {
                    return Ok(());
                }
            }
            if chunk["done"].as_bool() == Some(true) {
                break;
            }
        }
        Ok(())
    }

    /// A reply forced to match the JSON `schema`.
    pub fn decide(&self, messages: &[Message], schema: &Value) -> Result<Value> {
        let resp: Value = self.post_chat(messages, json!({ "temperature": 0 }), Some(schema), false)?.json()?;
        let text = resp["message"]["content"].as_str().unwrap_or_default();
        Ok(serde_json::from_str(text)?)
    }

    fn post_chat(&self, messages: &[Message], options: Value, format: Option<&Value>, stream: bool) -> Result<reqwest::blocking::Response> {
        let Settings { base, model, keep_alive } = self.settings();
        let mut body = json!({
            "model": model,
            "messages": messages.iter().map(Message::to_json).collect::<Vec<_>>(),
            "stream": stream,
            "keep_alive": keep_alive,
            "options": options,
            // Thinking models (Qwen 3) would otherwise reason silently for seconds first.
            "think": false,
        });
        if let Some(schema) = format {
            body["format"] = schema.clone();
        }
        let mut resp = self.client.post(format!("{base}/api/chat")).json(&body).send().map_err(|e| self.explain(e))?;
        if !resp.status().is_success() && resp.status().as_u16() == 400 {
            // Older models reject `think`.
            body.as_object_mut().unwrap().remove("think");
            resp = self.client.post(format!("{base}/api/chat")).json(&body).send().map_err(|e| self.explain(e))?;
        }
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or(text);
        if status.as_u16() == 404 && detail.contains("not found") {
            return Err(anyhow!("Model '{model}' isn't installed. Run: ollama pull {model}"));
        }
        Err(anyhow!("Ollama error ({status}): {detail}"))
    }

    /// Loads the model into memory ahead of the first question (an empty chat does just that).
    pub fn warm_up(&self) -> Result<()> {
        let Settings { base, model, keep_alive } = self.settings();
        let body = json!({ "model": model, "messages": [], "keep_alive": keep_alive });
        let resp = self.client.post(format!("{base}/api/chat")).json(&body).send().map_err(|e| self.explain(e))?;
        if resp.status().as_u16() == 404 {
            return Err(anyhow!("Model '{model}' isn't installed. Run: ollama pull {model}"));
        }
        resp.error_for_status()?;
        Ok(())
    }

    pub fn model(&self) -> String {
        self.settings().model
    }

    fn explain(&self, e: reqwest::Error) -> anyhow::Error {
        if e.is_connect() || e.is_timeout() {
            anyhow!("Ollama isn't running. Open the Ollama app (or run: ollama serve)")
        } else {
            anyhow!("Could not reach Ollama: {e}")
        }
    }
}

/// Splits streamed text into speakable sentences as soon as each one is complete.
#[derive(Default)]
pub struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    /// Adds streamed text and returns any sentences it completed.
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        while let Some(end) = sentence_end(&self.buf) {
            let sentence: String = self.buf.drain(..end).collect();
            let sentence = sentence.trim();
            if !sentence.is_empty() {
                out.push(sentence.to_string());
            }
        }
        out
    }

    /// Whatever is left once the stream ends.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let rest = rest.trim();
        (!rest.is_empty()).then(|| rest.to_string())
    }
}

/// Small local models sometimes get stuck going round in circles ("… — no. Actually, it's …
/// — no."). Watches the streamed reply and reports when any sentence comes round a third time.
#[derive(Default)]
pub struct LoopGuard {
    buf: String,
    /// Bytes of the reply already split off into `seen`.
    consumed: usize,
    /// Each normalised sentence: (times seen, byte offset in the reply where it first appeared).
    seen: std::collections::HashMap<String, (u32, usize)>,
}

impl LoopGuard {
    const REPEATS: u32 = 3;
    /// Fragments shorter than this ("no", "1", "e.g") repeat naturally.
    const MIN_CHARS: usize = 8;

    /// Adds streamed text. Returns the byte offset in the reply where the loop began once the
    /// reply is repeating itself; everything from there on is noise.
    pub fn push(&mut self, text: &str) -> Option<usize> {
        self.buf.push_str(text);
        while let Some(i) = self.buf.find(['.', '!', '?', '\n', '—']) {
            let end = i + self.buf[i..].chars().next().map_or(1, char::len_utf8);
            let start = self.consumed;
            let piece: String = self.buf.drain(..end).collect();
            self.consumed += end;
            let key = piece
                .chars()
                .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                .collect::<String>()
                .to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if key.chars().count() < Self::MIN_CHARS {
                continue;
            }
            let entry = self.seen.entry(key).or_insert((0, start));
            entry.0 += 1;
            if entry.0 >= Self::REPEATS {
                return Some(entry.1);
            }
        }
        None
    }
}

/// Byte index just past the first sentence boundary: `.`/`!`/`?`/`:` followed by whitespace,
/// or a newline. Very short fragments ("e.g.", "Dr.") are not treated as sentences.
fn sentence_end(s: &str) -> Option<usize> {
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let boundary = match c {
            '\n' => true,
            '.' | '!' | '?' | ':' | ';' => next.is_some_and(char::is_whitespace),
            _ => false,
        };
        if boundary && (c == '\n' || s[..i].trim().len() >= 12) {
            return Some(i + c.len_utf8());
        }
    }
    None
}

/// A closing offer that adds nothing when heard: "Let me know if you need anything else!",
/// "Would you like to adjust the volume?". Only dropped after the answer, never as the answer.
pub fn is_offer(sentence: &str) -> bool {
    static OFFER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)^\W*(let me know (if|whether|what|how)|is there anything (else|more)|anything else (i can|you)|feel free to|if you (need|want|have|'d like) (anything|any|more|further|help)|hope (this|that) helps|happy to help|would you like (me )?to|do you want me to|shall i|should i (also )?(help|play|open|search|adjust))",
        )
        .unwrap()
    });
    OFFER.is_match(sentence)
}

/// The reply without closing offers after its first sentence.
pub fn without_offers(reply: &str) -> String {
    let mut splitter = SentenceSplitter::default();
    let mut sentences = splitter.push(reply);
    sentences.extend(splitter.finish());
    let mut kept: Vec<String> = Vec::new();
    for sentence in sentences {
        if kept.is_empty() || !is_offer(&sentence) {
            kept.push(sentence);
        }
    }
    kept.join(" ")
}

/// Strips markdown and symbols the voice would read out literally ("asterisk asterisk").
pub fn speakable(text: &str) -> String {
    let mut s: String = text.chars().filter(|c| !matches!(c, '*' | '#' | '`' | '_' | '|' | '>')).collect();
    // List markers at the start: "- item", "1. item".
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix("- ") {
        s = rest.to_string();
    }
    s.trim().to_string()
}
