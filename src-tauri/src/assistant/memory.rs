//! Long-term facts about the user, one per line in `memory.txt` (editable in Settings).
//! Local only. Each question gets the few facts most related to it (meaning, via `embedder`,
//! plus shared words for names), and after each answer the AI notes any new lasting facts:
//! the Mem0 idea, without a server or a vector database (a few thousand facts search in a
//! millisecond as a plain list).

use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use super::brain::Brain;
use super::embedder::{cosine, embed};
use super::llm::Message;

// Serialise read/modify/write so two writers in a row can't lose a fact.
static STORE: Mutex<()> = Mutex::new(());
/// Embeddings of facts by text, so each fact is embedded once (kept in `memory-vectors.json`).
static VECTORS: Mutex<Option<HashMap<String, Vec<f32>>>> = Mutex::new(None);
const MAX_FACTS: usize = 2000;
const MAX_FACT_LEN: usize = 200;
/// Facts sent with each question.
const TOP_K: usize = 6;
/// A new fact this close in meaning to a kept one replaces it ("lives in Pune" → "lives in Delhi").
const SAME_FACT: f32 = 0.9;

fn path() -> PathBuf {
    crate::config::data_dir().join("memory.txt")
}

fn vectors_path() -> PathBuf {
    crate::config::data_dir().join("memory-vectors.json")
}

pub fn load() -> Vec<String> {
    std::fs::read_to_string(path()).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

fn save(facts: &[String]) -> Result<()> {
    std::fs::write(path(), facts.join("\n") + "\n")?;
    Ok(())
}

/// The fact's embedding, from the cache or worked out now. `None` while the embedder isn't ready.
fn vector(fact: &str) -> Option<Vec<f32>> {
    let mut cache = VECTORS.lock().unwrap();
    let map = cache.get_or_insert_with(|| std::fs::read(vectors_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    if let Some(v) = map.get(fact) {
        return Some(v.clone());
    }
    let v = embed(fact)?;
    map.insert(fact.to_string(), v.clone());
    if let Ok(bytes) = serde_json::to_vec(map) {
        let _ = std::fs::write(vectors_path(), bytes);
    }
    Some(v)
}

fn clean(fact: &str) -> String {
    fact.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_FACT_LEN).collect()
}

pub fn remember(fact: &str) -> Result<String> {
    let fact = clean(fact);
    if fact.is_empty() {
        bail!("Nothing to remember");
    }
    let _guard = STORE.lock().unwrap();
    let mut facts = load();
    if facts.iter().any(|f| f.eq_ignore_ascii_case(&fact)) {
        return Ok(format!("Already remembered: {fact}"));
    }
    if let Some(new) = vector(&fact) {
        facts.retain(|f| vector(f).map_or(true, |v| cosine(&v, &new) < SAME_FACT));
    }
    facts.push(fact.clone());
    facts.drain(..facts.len().saturating_sub(MAX_FACTS));
    save(&facts)?;
    Ok(format!("Remembered: {fact}"))
}

/// Replaces all facts with `text`, one per line (the Settings editor).
pub fn replace(text: &str) -> Result<()> {
    let _guard = STORE.lock().unwrap();
    let facts: Vec<String> = text.lines().map(clean).filter(|l| !l.is_empty()).collect();
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

fn words(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 3).map(String::from).collect()
}

/// The facts most related to `text`, best first: all of them while there are only a few.
fn relevant(text: &str, k: usize) -> Vec<String> {
    let facts = load();
    if facts.len() <= k {
        return facts;
    }
    let query = embed(text);
    let query_words = words(text);
    let mut scored: Vec<(f32, String)> = facts
        .into_iter()
        .enumerate()
        .map(|(i, f)| {
            let meaning = query.as_ref().zip(vector(&f)).map_or(0.0, |(q, v)| cosine(q, &v));
            // Names and places match by spelling better than by meaning.
            let shared = words(&f).iter().filter(|w| query_words.contains(w)).count() as f32;
            // Newer facts win ties.
            (meaning + 0.1 * shared + i as f32 * 1e-6, f)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(k).map(|(_, f)| f).collect()
}

/// `question` with the facts that may help answer it, for the AI.
pub fn with_facts(question: &str) -> String {
    let facts = relevant(question, TOP_K);
    if facts.is_empty() {
        return question.to_string();
    }
    format!("{question}\n\n[What you remember about the user, use only if it helps: {}]", facts.join("; "))
}

/// Notes lasting facts from one exchange (Mem0's add/update/delete step). Runs after the answer,
/// off the answer's path.
pub fn learn(brain: &Brain, question: &str, reply: &str) {
    if question.split_whitespace().count() < 3 {
        return;
    }
    let known = relevant(question, TOP_K);
    let messages = [
        Message::system(
            "You keep a short list of lasting facts about the user for a voice assistant. From the exchange, \
             note only durable personal facts the user stated: names, family, friends, where they live or work, \
             preferences, routines, projects, important dates. Never note questions, one-off requests, the \
             assistant's own answers, or general knowledge. Write each fact as one short English sentence \
             starting with \"The user\". In remove, copy any known fact the user just said is wrong or out of \
             date. Usually both lists are empty.",
        ),
        Message::user(format!(
            "Known facts:\n{}\n\nUser: {question}\nAssistant: {reply}",
            if known.is_empty() { "(none)".to_string() } else { known.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n") }
        )),
    ];
    let list = json!({ "type": "array", "items": { "type": "string" }, "maxItems": 3 });
    let schema = json!({ "type": "object", "properties": { "add": list, "remove": list }, "required": ["add", "remove"] });
    let decision = match brain.decide(&messages, &schema) {
        Ok(d) => d,
        Err(e) => return mlog!("memory: couldn't learn from the exchange: {e}"),
    };
    let strings = |key: &str| decision[key].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect::<Vec<_>>();
    for old in strings("remove") {
        let _guard = STORE.lock().unwrap();
        let facts = load();
        let kept: Vec<String> = facts.iter().filter(|f| !f.eq_ignore_ascii_case(old.trim())).cloned().collect();
        if kept.len() < facts.len() && save(&kept).is_ok() {
            mlog!("memory: dropped “{old}”");
        }
    }
    for fact in strings("add") {
        if let Ok(done) = remember(&fact) {
            mlog!("memory: {done}");
        }
    }
}
