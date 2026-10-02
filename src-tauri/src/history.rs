//! Local dictation log: one JSON object per line in `history.jsonl`. Never leaves the machine.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// RFC 3339 local time; also used as the entry's id.
    pub time: String,
    pub text: String,
    #[serde(default)]
    pub raw: String,
    #[serde(default)]
    pub audio_seconds: f32,
    #[serde(default)]
    pub transcribe_ms: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct Stats {
    pub dictations: usize,
    pub words: usize,
    pub words_today: usize,
    pub audio_seconds: f32,
    /// Typing the same words at 40 wpm, minus the time spent speaking.
    pub minutes_saved: f32,
    pub avg_transcribe_ms: u64,
}

const TYPING_WPM: f32 = 40.0;

pub fn path() -> PathBuf {
    crate::config::data_dir().join("history.jsonl")
}

/// Appends one entry.
pub fn append(text: &str, raw: &str, audio_seconds: f32, transcribe_ms: u128) {
    let entry = Entry {
        time: chrono::Local::now().to_rfc3339(),
        text: text.to_string(),
        raw: raw.to_string(),
        audio_seconds,
        transcribe_ms: transcribe_ms as u64,
    };
    let Ok(line) = serde_json::to_string(&entry) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path()) {
        let _ = writeln!(f, "{line}");
    }
}

/// All entries, oldest first. Unreadable lines are skipped.
pub fn load() -> Vec<Entry> {
    let Ok(content) = std::fs::read_to_string(path()) else { return vec![] };
    content.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn write_all(entries: &[Entry]) -> anyhow::Result<()> {
    let mut out = String::new();
    for e in entries {
        out.push_str(&serde_json::to_string(e)?);
        out.push('\n');
    }
    std::fs::write(path(), out)?;
    Ok(())
}

/// Newest first, filtered by a case-insensitive substring.
pub fn search(query: &str, limit: usize) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    load()
        .into_iter()
        .rev()
        .filter(|e| q.is_empty() || e.text.to_lowercase().contains(&q))
        .take(limit)
        .collect()
}

pub fn delete(time: &str) -> anyhow::Result<()> {
    let entries: Vec<Entry> = load().into_iter().filter(|e| e.time != time).collect();
    write_all(&entries)
}

pub fn clear() -> anyhow::Result<()> {
    write_all(&[])
}

/// Text of the most recent entry, if any.
pub fn last_text() -> Option<String> {
    load().pop().map(|e| e.text)
}

pub fn stats() -> Stats {
    stats_for(&load(), &chrono::Local::now().format("%Y-%m-%d").to_string())
}

fn stats_for(entries: &[Entry], today: &str) -> Stats {
    let count_words = |e: &Entry| e.text.split_whitespace().count();
    let words: usize = entries.iter().map(count_words).sum();
    let audio_seconds: f32 = entries.iter().map(|e| e.audio_seconds).sum();
    let typing_minutes = words as f32 / TYPING_WPM;
    Stats {
        dictations: entries.len(),
        words,
        words_today: entries.iter().filter(|e| e.time.starts_with(today)).map(count_words).sum(),
        audio_seconds,
        minutes_saved: (typing_minutes - audio_seconds / 60.0).max(0.0),
        avg_transcribe_ms: if entries.is_empty() {
            0
        } else {
            entries.iter().map(|e| e.transcribe_ms).sum::<u64>() / entries.len() as u64
        },
    }
}
