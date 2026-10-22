//! Rule-based tidy-up of raw Whisper output. Deliberately conservative: it should never
//! change the meaning of what was said.

use crate::config::Replacement;
use regex::{Regex, RegexBuilder};
use std::sync::LazyLock;

static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]|\*[^*]*\*").unwrap());
static FILLERS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:,\s*)?\b(?:u+m+|u+h+m*|e+r+m+|hm+|mm+)\b,?").unwrap());
static SPACE_BEFORE_PUNCT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+([,.!?;:])").unwrap());
static MULTI_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());
/// "you know" / "I mean" only when set off by commas or opening a sentence before a comma,
/// so "I mean it" and "do you know him" survive.
static SOFT_FILLERS_MID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i),\s*(?:you know|i mean)\s*,").unwrap());
static SOFT_FILLERS_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|[.!?]\s+)(?:you know|i mean|so,? like)\s*,\s*(\w)").unwrap());
/// "at 10.30" / "10.30 pm" → "10:30" (Whisper writes spoken times with a dot).
static TIME_AFTER_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(at|by|until|till|from|around|before|after|to)\s+(\d{1,2})\.([0-5]\d)\b").unwrap()
});
static TIME_BEFORE_AMPM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,2})\.([0-5]\d)(\s*)(am|pm|a\.m\.|p\.m\.)").unwrap());
static DOUBLE_COMMA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",\s*([,.!?])").unwrap());

/// Phrases Whisper tends to invent on near-silent audio.
const HALLUCINATIONS: &[&str] = &[
    "you", "thank you", "thanks for watching", "thank you for watching", "bye", "okay", "silence",
];

pub fn clean(raw: &str, remove_fillers: bool) -> String {
    clean_for_style(raw, "clean", remove_fillers)
}

/// Tidies raw Whisper output for dictation, according to the user's style.
pub fn clean_for_style(raw: &str, style: &str, remove_fillers: bool) -> String {
    match style {
        "verbatim" => {
            let s = MULTI_SPACE.replace_all(raw.trim(), " ").into_owned();
            s.trim().to_string()
        }
        "message" => {
            let mut s = tidy(raw, remove_fillers, true, true);
            s = s.replace('\n', " ");
            s = MULTI_SPACE.replace_all(&s, " ").into_owned();
            let s = s.trim();
            if s.is_empty() {
                return String::new();
            }
            if !s.ends_with(['.', '!', '?']) {
                format!("{s}.")
            } else {
                s.to_string()
            }
        }
        "email" => {
            let s = tidy(raw, remove_fillers, true, true);
            capitalize_sentences(&s)
        }
        "code" => tidy(raw, false, true, false).trim().to_string(),
        _ => tidy(raw, remove_fillers, true, true),
    }
}

/// A spoken question for the assistant. Only an explicit am/pm suffix identifies a time there:
/// "at 10.30" can be a price.
pub fn clean_question(raw: &str) -> String {
    tidy(raw, true, false, true)
}

fn tidy(raw: &str, remove_fillers: bool, times_after_words: bool, capitalize: bool) -> String {
    let mut s = TAGS.replace_all(raw, " ").into_owned();
    if remove_fillers {
        s = FILLERS.replace_all(&s, "").into_owned();
        s = SOFT_FILLERS_MID.replace_all(&s, "").into_owned();
        s = SOFT_FILLERS_START
            .replace_all(&s, |c: &regex::Captures| format!("{}{}", &c[1], c[2].to_uppercase()))
            .into_owned();
        s = dedupe_words(&s);
    }
    if times_after_words {
        s = TIME_AFTER_WORD.replace_all(&s, "$1 $2:$3").into_owned();
    }
    s = TIME_BEFORE_AMPM.replace_all(&s, "$1:$2$3$4").into_owned();
    s = SPACE_BEFORE_PUNCT.replace_all(&s, "$1").into_owned();
    s = DOUBLE_COMMA.replace_all(&s, "$1").into_owned();
    s = MULTI_SPACE.replace_all(&s, " ").into_owned();
    let s = s.trim().trim_start_matches([',', '.', ';', ':']).trim();
    if capitalize { capitalize_first(s) } else { s.to_string() }
}

/// Applies the user's fix-ups, e.g. a name Whisper keeps mishearing.
/// Whole words only, case-insensitive; the replacement is inserted as written.
pub fn apply_replacements(text: &str, replacements: &[Replacement]) -> String {
    let mut out = text.to_string();
    for r in replacements {
        let from = r.from.trim();
        if from.is_empty() {
            continue;
        }
        let pattern = format!(r"(^|\W){}(\W|$)", regex::escape(from));
        if let Ok(re) = RegexBuilder::new(&pattern).case_insensitive(true).build() {
            let to = r.to.as_str();
            out = re.replace_all(&out, |c: &regex::Captures| format!("{}{}{}", &c[1], to, &c[2])).into_owned();
        }
    }
    out
}

/// True for output that is almost certainly a hallucination on quiet audio.
pub fn is_hallucination(text: &str) -> bool {
    let t = text.trim().trim_matches(|c: char| !c.is_alphanumeric() && c != ' ').to_lowercase();
    t.is_empty() || HALLUCINATIONS.contains(&t.as_str())
}

/// Collapses accidental stutters like "the the" or "I I".
fn dedupe_words(s: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for word in s.split_whitespace() {
        if let Some(prev) = out.last() {
            let prev_plain = prev.trim_end_matches(|c: char| !c.is_alphanumeric());
            let has_punct = prev_plain.len() != prev.len();
            if !has_punct && prev_plain.eq_ignore_ascii_case(word.trim_end_matches(|c: char| !c.is_alphanumeric())) {
                out.pop();
            }
        }
        out.push(word);
    }
    out.join(" ")
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn capitalize_sentences(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap_next = true;
    for c in s.chars() {
        if cap_next && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            cap_next = false;
        } else {
            out.push(c);
            if ".!?".contains(c) {
                cap_next = true;
            }
        }
    }
    let t = out.trim();
    if t.is_empty() {
        return String::new();
    }
    if !t.ends_with(['.', '!', '?']) {
        format!("{t}.")
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;

    #[test]
    fn verbatim_keeps_fillers() {
        let s = clean_for_style("um hello there", "verbatim", true);
        assert!(s.contains("um"));
    }

    #[test]
    fn message_one_paragraph() {
        let s = clean_for_style("hello\nworld", "message", false);
        assert_eq!(s, "Hello world.");
    }
}
