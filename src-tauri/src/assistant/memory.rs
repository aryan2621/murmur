//! Long-term facts the user asked Jarvis to remember: one per line in `memory.txt`.
//! Local only; read into the system prompt so they apply to every conversation.

use anyhow::{bail, Result};
use std::path::PathBuf;
use std::sync::Mutex;

// Serialise read/modify/write so two actions in a row can't lose a fact.
static STORE: Mutex<()> = Mutex::new(());
/// Facts kept in the prompt; the oldest are dropped past this (16k context window).
const MAX_FACTS: usize = 30;
const MAX_FACT_LEN: usize = 200;

fn path() -> PathBuf {
    crate::config::data_dir().join("memory.txt")
}

pub fn load() -> Vec<String> {
    std::fs::read_to_string(path()).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

fn save(facts: &[String]) -> Result<()> {
    std::fs::write(path(), facts.join("\n") + "\n")?;
    Ok(())
}

pub fn remember(fact: &str) -> Result<String> {
    let fact = fact.split_whitespace().collect::<Vec<_>>().join(" ");
    if fact.is_empty() {
        bail!("Nothing to remember");
    }
    let fact: String = fact.chars().take(MAX_FACT_LEN).collect();
    let _guard = STORE.lock().unwrap();
    let mut facts = load();
    if !facts.iter().any(|f| f.eq_ignore_ascii_case(&fact)) {
        facts.push(fact.clone());
        facts.drain(..facts.len().saturating_sub(MAX_FACTS));
        save(&facts)?;
    }
    Ok(format!("Remembered: {fact}"))
}

/// Replaces all facts with `text`, one per line (the Settings editor).
pub fn replace(text: &str) -> Result<()> {
    let _guard = STORE.lock().unwrap();
    let facts: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.chars().take(MAX_FACT_LEN).collect()).collect();
    save(&facts[facts.len().saturating_sub(MAX_FACTS)..])
}

/// Removes every fact containing `words` (case-insensitive).
pub fn forget(words: &str) -> Result<String> {
    let words = words.trim().to_lowercase();
    if words.is_empty() {
        bail!("Say what to forget");
    }
    let _guard = STORE.lock().unwrap();
    let facts = load();
    let kept: Vec<String> = facts.iter().filter(|f| !f.to_lowercase().contains(&words)).cloned().collect();
    if kept.len() == facts.len() {
        bail!("I don't have anything about that");
    }
    save(&kept)?;
    Ok(format!("Forgot {} thing(s)", facts.len() - kept.len()))
}

/// Prompt section listing the facts, empty when there are none.
pub fn prompt_section() -> String {
    let facts = load();
    if facts.is_empty() {
        return String::new();
    }
    format!("\n\nThings the user asked you to remember (use them when relevant):\n{}", facts.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n"))
}
