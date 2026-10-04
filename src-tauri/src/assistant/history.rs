//! Local log of questions and answers: one JSON object per line in `assistant-history.jsonl`.
//! Never leaves the machine.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// Serialise append and read/modify/replace so deleting an entry cannot lose a new answer.
static STORE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// RFC 3339 local time; also used as the entry's id.
    pub time: String,
    pub question: String,
    pub answer: String,
    /// "quick" (held the key), "conversation" (tapped) or "typed" (from the app window).
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub audio_seconds: f32,
    /// Speech recognition time.
    #[serde(default)]
    pub heard_ms: u64,
    /// From sending the question to the model's first words.
    #[serde(default)]
    pub first_word_ms: u64,
    /// From sending the question to the end of the written answer.
    #[serde(default)]
    pub total_ms: u64,
    #[serde(default)]
    pub model: String,
    /// What was done to answer: "Opening Slack", "Searching the web for …".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
}

pub fn path() -> PathBuf {
    crate::config::data_dir().join("assistant-history.jsonl")
}

pub fn append(entry: &Entry) {
    let _guard = STORE.lock().unwrap();
    let Ok(line) = serde_json::to_string(entry) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path()) {
        let _ = writeln!(f, "{line}");
    }
}

/// All entries, oldest first. Unreadable lines are skipped.
pub fn load() -> Vec<Entry> {
    let _guard = STORE.lock().unwrap();
    read_entries(&path()).unwrap_or_default()
}

fn read_entries(path: &Path) -> anyhow::Result<Vec<Entry>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    Ok(content.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
}

fn write_all(path: &Path, entries: &[Entry]) -> anyhow::Result<()> {
    let mut out = String::new();
    for e in entries {
        out.push_str(&serde_json::to_string(e)?);
        out.push('\n');
    }
    let pending = path.with_extension("jsonl.tmp");
    let mut file = std::fs::File::create(&pending)?;
    file.write_all(out.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(pending, path)?;
    Ok(())
}

/// Newest first, filtered by a case-insensitive substring of the question or answer.
pub fn search(query: &str, limit: usize) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    load()
        .into_iter()
        .rev()
        .filter(|e| q.is_empty() || e.question.to_lowercase().contains(&q) || e.answer.to_lowercase().contains(&q))
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
    let _guard = STORE.lock().unwrap();
    let path = path();
    let entries: Vec<Entry> = read_entries(&path)?.into_iter().filter(|e| e.time != time).collect();
    write_all(&path, &entries)
}

pub fn clear() -> anyhow::Result<()> {
    let _guard = STORE.lock().unwrap();
    write_all(&path(), &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_preserves_valid_entries_and_leaves_no_partial_file() {
        let dir = std::env::temp_dir().join(format!("murmur-history-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        let entry: Entry = serde_json::from_value(serde_json::json!({"time": "test", "question": "hello", "answer": "hi"})).unwrap();
        write_all(&path, &[entry]).unwrap();
        assert_eq!(read_entries(&path).unwrap()[0].answer, "hi");
        assert!(!path.with_extension("jsonl.tmp").exists());
        write_all(&path, &[]).unwrap();
        assert!(read_entries(&path).unwrap().is_empty());
        assert!(read_entries(&dir).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
