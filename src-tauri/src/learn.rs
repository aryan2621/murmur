//! Suggest vocabulary / replacements when the user fixes a dictation in History.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Learning {
    pub kind: String,
    pub from: String,
    pub to: String,
}

/// Conservative diff: one whole-word substitution, or one new vocabulary token.
pub fn suggest_learnings(old_text: &str, new_text: &str, raw: &str) -> Vec<Learning> {
    let old = old_text.trim();
    let new = new_text.trim();
    if old.is_empty() || new.is_empty() || old == new {
        return vec![];
    }

    let old_words: Vec<&str> = old.split_whitespace().collect();
    let new_words: Vec<&str> = new.split_whitespace().collect();
    if old_words.len() == new_words.len() && !old_words.is_empty() {
        let mut diffs = Vec::new();
        for (o, n) in old_words.iter().zip(new_words.iter()) {
            if o != n {
                diffs.push(((*o).to_string(), (*n).to_string()));
            }
        }
        if diffs.len() == 1 {
            return vec![Learning { kind: "replacement".into(), from: diffs[0].0.clone(), to: diffs[0].1.clone() }];
        }
    }

    let raw_lower = raw.to_lowercase();
    for word in new_words {
        let plain = word.trim_matches(|c: char| !c.is_alphanumeric());
        if plain.len() < 2 {
            continue;
        }
        if old_words.iter().any(|w| w.eq_ignore_ascii_case(plain)) {
            continue;
        }
        if raw_lower.contains(&plain.to_lowercase()) {
            continue;
        }
        return vec![Learning { kind: "vocabulary".into(), from: String::new(), to: plain.to_string() }];
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_replacement() {
        let s = suggest_learnings("Open slack", "Open Slack", "open slack");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].kind, "replacement");
        assert_eq!(s[0].from, "slack");
        assert_eq!(s[0].to, "Slack");
    }
}
