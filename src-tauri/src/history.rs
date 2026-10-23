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
    /// Bundle id of the app dictated into ("" if unknown, or a file transcription).
    #[serde(default)]
    pub app: String,
    /// File name in `audio_dir()` when the clip was kept ("" otherwise).
    #[serde(default)]
    pub audio: String,
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

pub fn audio_dir() -> PathBuf {
    crate::config::data_dir().join("audio")
}

/// Appends one entry, keeping its 16 kHz audio as a WAV when `samples` is given.
pub fn append(text: &str, raw: &str, audio_seconds: f32, transcribe_ms: u128, app: &str, samples: Option<&[f32]>) {
    let now = chrono::Local::now();
    let audio = samples
        .and_then(|s| {
            let name = format!("{}.wav", now.format("%Y%m%d-%H%M%S%3f"));
            let saved = std::fs::create_dir_all(audio_dir()).map_err(anyhow::Error::from).and_then(|_| crate::audio::write_wav(&audio_dir().join(&name), s));
            if let Err(e) = &saved {
                mlog!("couldn't keep the dictation audio: {e}");
            }
            saved.ok().map(|_| name)
        })
        .unwrap_or_default();
    let entry = Entry {
        time: now.to_rfc3339(),
        text: text.to_string(),
        raw: raw.to_string(),
        audio_seconds,
        transcribe_ms: transcribe_ms as u64,
        app: app.to_string(),
        audio,
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

/// One page of matches, newest first (`page` counts from 0), and how many match in all.
pub fn page(query: &str, page: usize, size: usize) -> (Vec<Entry>, usize) {
    let all = search(query, usize::MAX);
    let total = all.len();
    (all.into_iter().skip(page * size).take(size).collect(), total)
}

pub fn delete(time: &str) -> anyhow::Result<()> {
    let (gone, entries): (Vec<Entry>, Vec<Entry>) = load().into_iter().partition(|e| e.time == time);
    write_all(&entries)?;
    for e in gone.iter().filter(|e| !e.audio.is_empty()) {
        let _ = std::fs::remove_file(audio_dir().join(&e.audio));
    }
    Ok(())
}

/// Updates the final text of one entry, and its transcript when `raw` is given.
pub fn update_text(time: &str, text: &str, raw: Option<&str>) -> anyhow::Result<Option<Entry>> {
    let mut found = None;
    let entries: Vec<Entry> = load()
        .into_iter()
        .map(|mut e| {
            if e.time == time {
                e.text = text.to_string();
                if let Some(raw) = raw {
                    e.raw = raw.to_string();
                }
                found = Some(e.clone());
            }
            e
        })
        .collect();
    if found.is_none() {
        return Ok(None);
    }
    write_all(&entries)?;
    Ok(found)
}

pub fn entry_by_time(time: &str) -> Option<Entry> {
    load().into_iter().find(|e| e.time == time)
}

pub fn clear() -> anyhow::Result<()> {
    write_all(&[])?;
    let _ = std::fs::remove_dir_all(audio_dir());
    Ok(())
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
