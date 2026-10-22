//! Text snippet expansion after transcription.

use crate::config::Snippet;
use regex::RegexBuilder;

/// Expands configured snippets on the final dictation text.
pub fn apply_snippets(text: &str, snippets: &[Snippet]) -> String {
    let mut out = text.to_string();
    for sn in snippets {
        let trigger = sn.trigger.trim();
        if trigger.is_empty() {
            continue;
        }
        let expansion = sn.text.trim();
        if expansion.is_empty() {
            continue;
        }
        if sn.mode == "anywhere" {
            let pattern = format!(r"(^|\W){}(\W|$)", regex::escape(trigger));
            if let Ok(re) = RegexBuilder::new(&pattern).case_insensitive(true).build() {
                out = re
                    .replace_all(&out, |c: &regex::Captures| format!("{}{}{}", &c[1], expansion, &c[2]))
                    .into_owned();
            }
        } else if out.ends_with(trigger) {
            let prefix = out[..out.len() - trigger.len()].trim_end();
            out = if prefix.is_empty() {
                expansion.to_string()
            } else {
                format!("{prefix} {expansion}")
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Snippet;

    #[test]
    fn suffix_snippet() {
        let sn = vec![Snippet { trigger: ";;sig".into(), text: "Best,\nRishabh".into(), mode: "suffix".into() }];
        assert_eq!(apply_snippets("Thanks ;;sig", &sn), "Thanks Best,\nRishabh");
    }

    #[test]
    fn anywhere_snippet() {
        let sn = vec![Snippet { trigger: "github".into(), text: "GitHub".into(), mode: "anywhere".into() }];
        assert_eq!(apply_snippets("Push to github now", &sn), "Push to GitHub now");
    }
}
