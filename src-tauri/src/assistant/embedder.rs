//! Sentence embeddings for memory search: bge-small-en-v1.5 (int8 ONNX, ~34 MB) through the ONNX
//! Runtime Murmur already ships for the natural voice. A few milliseconds per sentence on the CPU.

use anyhow::{anyhow, Result};
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::{config, model};

/// bge-small's vector width.
const DIM: usize = 384;
/// Facts and questions are short; longer text is cut here.
const MAX_TOKENS: usize = 128;

struct Loaded {
    session: Session,
    vocab: HashMap<String, i64>,
}

static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

/// The text as a unit-length vector, or `None` while the model isn't downloaded (the download
/// starts in the background on first use) or couldn't load.
pub fn embed(text: &str) -> Option<Vec<f32>> {
    let mut slot = LOADED.lock().unwrap();
    if slot.is_none() {
        *slot = load().map_err(|e| mlog!("memory search model: {e}")).ok().flatten();
    }
    let loaded = slot.as_mut()?;
    run(loaded, text).map_err(|e| mlog!("embedding failed: {e}")).ok()
}

fn load() -> Result<Option<Loaded>> {
    let dir = config::data_dir();
    if !model::embedder_ready(&dir) {
        static DOWNLOADING: AtomicBool = AtomicBool::new(false);
        if !DOWNLOADING.swap(true, Ordering::SeqCst) {
            std::thread::spawn(move || {
                if let Err(e) = model::ensure_embedder(&dir) {
                    mlog!("memory search model download failed: {e}");
                    DOWNLOADING.store(false, Ordering::SeqCst);
                }
            });
        }
        return Ok(None);
    }
    let (model_path, vocab_path) = model::embedder_paths(&dir);
    let vocab = std::fs::read_to_string(vocab_path)?.lines().enumerate().map(|(i, t)| (t.to_string(), i as i64)).collect();
    // The builder's errors carry the builder back, which isn't `Send`.
    let mut builder = Session::builder()?.with_intra_threads(2).map_err(|e| anyhow!("{e}"))?;
    let session = builder.commit_from_file(model_path)?;
    Ok(Some(Loaded { session, vocab }))
}

fn run(loaded: &mut Loaded, text: &str) -> Result<Vec<f32>> {
    let ids = tokenize(&loaded.vocab, text)?;
    let n = ids.len();
    let outputs = loaded.session.run(ort::inputs![
        "input_ids" => Tensor::from_array(([1usize, n], ids))?,
        "attention_mask" => Tensor::from_array(([1usize, n], vec![1i64; n]))?,
        "token_type_ids" => Tensor::from_array(([1usize, n], vec![0i64; n]))?,
    ])?;
    let (_, hidden) = outputs["last_hidden_state"].try_extract_tensor::<f32>()?;
    // bge pools with the [CLS] token, the first row.
    let mut v = hidden[..DIM].to_vec();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= norm);
    Ok(v)
}

/// BERT's uncased WordPiece: lower-case, split off punctuation, then longest known pieces.
// shortcut: no accent stripping or CJK splitting; fine for the English facts the AI writes, use the tokenizers crate if non-English recall matters.
fn tokenize(vocab: &HashMap<String, i64>, text: &str) -> Result<Vec<i64>> {
    let id = |t: &str| vocab.get(t).copied().ok_or_else(|| anyhow!("vocabulary has no {t}"));
    let (cls, sep, unk) = (id("[CLS]")?, id("[SEP]")?, id("[UNK]")?);
    let mut ids = vec![cls];
    let lower = text.to_lowercase();
    let words = lower.split_whitespace().flat_map(|w| {
        let mut parts = Vec::new();
        let mut start = 0;
        for (i, c) in w.char_indices() {
            if c.is_ascii_punctuation() {
                if start < i {
                    parts.push(&w[start..i]);
                }
                parts.push(&w[i..i + c.len_utf8()]);
                start = i + c.len_utf8();
            }
        }
        if start < w.len() {
            parts.push(&w[start..]);
        }
        parts
    });
    'words: for word in words {
        let mut pieces = Vec::new();
        let mut start = 0;
        while start < word.len() {
            let mut end = word.len();
            loop {
                let piece = if start == 0 { word[..end].to_string() } else { format!("##{}", &word[start..end]) };
                if let Some(&i) = vocab.get(&piece) {
                    pieces.push(i);
                    start = end;
                    break;
                }
                end = word[..end].char_indices().last().map(|(i, _)| i).unwrap_or(0);
                if end <= start {
                    ids.push(unk);
                    continue 'words;
                }
            }
        }
        ids.extend(pieces);
    }
    ids.truncate(MAX_TOKENS - 1);
    ids.push(sep);
    Ok(ids)
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
