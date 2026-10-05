//! The natural voice: Kokoro, an 82M-parameter speech model that sounds natural and speaks
//! several times faster than real time on the CPU, so answers start about as quickly as with the
//! macOS voices.
//!
//! Text becomes phonemes with misaki (Kokoro's own pronunciation rules and dictionaries), with
//! eSpeak NG for words those don't know (names, jargon), then one ONNX run turns a sentence's
//! phonemes into 24 kHz sound for the shared `Player`. Hindi written in Devanagari is read by
//! eSpeak NG's Hindi rules in one of Kokoro's Hindi voices, so a reply can mix both.

use crate::assistant::player::Player;
use crate::{config, model};
use anyhow::{anyhow, Result};
use misaki_rs::fallback::FallbackError;
use misaki_rs::{Fallback, Language, G2P};
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Kokoro reads at most this many phonemes at once (512 with the start and end markers).
const MAX_PHONEMES: usize = 510;
/// Floats in one style vector; a voice file holds one per phoneme count.
const STYLE: usize = 256;
/// Long sentences are spoken in pieces split at commas, so the first words come out sooner.
const PIECE_CHARS: usize = 140;

pub struct Kokoro {
    player: Arc<Player>,
    loaded: Mutex<Option<Loaded>>,
    last_used: Mutex<Instant>,
}

struct Loaded {
    session: Session,
    american: G2P,
    british: G2P,
    hindi: espeak_ng::Translator,
    /// Style tables by voice id, read on first use.
    voices: HashMap<String, Vec<f32>>,
}

impl Kokoro {
    pub fn new(player: Arc<Player>) -> Self {
        Self { player, loaded: Mutex::new(None), last_used: Mutex::new(Instant::now()) }
    }

    pub fn ready() -> bool {
        model::natural_ready(&config::data_dir())
    }

    /// Loads the model ahead of the first sentence, and says a word into the void: the first run
    /// after loading is a few times slower than the rest.
    pub fn warm_up(&self) -> Result<()> {
        let first = self.loaded.lock().unwrap().is_none();
        self.with_loaded(|k| if first { k.synthesize("Hi.", model::NATURAL_VOICES[0].0, 1.0).map(drop) } else { Ok(()) })
    }

    /// Gives the memory back.
    pub fn unload(&self) {
        if self.loaded.lock().unwrap().take().is_some() {
            mlog!("natural voice unloaded");
        }
    }

    /// Unloads after `minutes` without speaking (0 = never).
    pub fn unload_if_idle(&self, minutes: u64) {
        if minutes > 0 && self.last_used.lock().unwrap().elapsed() >= Duration::from_secs(minutes * 60) {
            self.unload();
        }
    }

    /// Speaks `text` as `voice` (a `model::NATURAL_VOICES` id) at `speed` (1 = normal): queues
    /// each piece to play as soon as it's made. Returns once all of it is queued, or soon after
    /// `cancelled` turns true.
    pub fn speak(&self, text: &str, voice: &str, speed: f32, cancelled: impl Fn() -> bool) -> Result<()> {
        let voice = if model::NATURAL_VOICES.iter().any(|(id, _)| *id == voice) { voice } else { model::NATURAL_VOICES[0].0 };
        for piece in pieces(text) {
            if cancelled() {
                return Ok(());
            }
            let started = Instant::now();
            let audio = self.with_loaded(|k| k.synthesize(&piece, voice, speed))?;
            if cancelled() {
                return Ok(());
            }
            mlog!("natural voice: {:.1} s of speech in {} ms", audio.len() as f32 / 24_000.0, started.elapsed().as_millis());
            self.player.add(audio);
        }
        Ok(())
    }

    fn with_loaded<T>(&self, f: impl FnOnce(&mut Loaded) -> Result<T>) -> Result<T> {
        *self.last_used.lock().unwrap() = Instant::now();
        let mut loaded = self.loaded.lock().unwrap();
        if loaded.is_none() {
            let started = Instant::now();
            *loaded = Some(Loaded::load()?);
            mlog!("natural voice ready in {} ms", started.elapsed().as_millis());
        }
        f(loaded.as_mut().unwrap())
    }
}

impl Loaded {
    fn load() -> Result<Self> {
        let dir = config::data_dir();
        if !model::natural_ready(&dir) {
            return Err(anyhow!("The natural voice isn't downloaded yet. Download it in Settings → Voice."));
        }
        // The builder's errors carry the builder back, which isn't `Send`.
        let mut builder = Session::builder()?.with_intra_threads(6).map_err(|e| anyhow!("{e}"))?;
        let session = builder.commit_from_file(model::natural_model_path(&dir))?;
        // eSpeak NG's English data is built into Murmur; it reads it from a folder.
        let espeak = dir.join("models").join("espeak-ng-data");
        if !espeak.join("hi_dict").exists() {
            espeak_ng::install_bundled_languages(&espeak, &["en", "hi"])?;
        }
        let fallback = |lang: &str| -> Result<Option<Box<dyn Fallback>>> {
            Ok(Some(Box::new(Espeak(Mutex::new(espeak_ng::Translator::new(lang, Some(&espeak))?)))))
        };
        Ok(Self {
            session,
            american: G2P::with_fallback(Language::EnglishUS, fallback("en-us")?),
            british: G2P::with_fallback(Language::EnglishGB, fallback("en")?),
            hindi: espeak_ng::Translator::new("hi", Some(&espeak))?,
            voices: HashMap::new(),
        })
    }

    /// Sound for `text`; runs of Hindi in it are read in the Hindi voice matching `voice`.
    fn synthesize(&mut self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>> {
        let mut audio = Vec::new();
        for (hindi, run) in script_runs(text) {
            let (phonemes, voice) = if hindi {
                let hindi_voice = if voice.chars().nth(1) == Some('m') { model::HINDI_VOICES[1] } else { model::HINDI_VOICES[0] };
                (self.hindi.text_to_ipa(&run)?, hindi_voice)
            } else {
                let g2p = if voice.starts_with('b') { &self.british } else { &self.american };
                (g2p.g2p(&run).map_err(|e| anyhow!("pronouncing \"{run}\": {e}"))?.0, voice)
            };
            audio.extend(self.speak_phonemes(&phonemes, voice, speed)?);
        }
        Ok(audio)
    }

    fn speak_phonemes(&mut self, phonemes: &str, voice: &str, speed: f32) -> Result<Vec<f32>> {
        let tokens = tokens(phonemes);
        if !self.voices.contains_key(voice) {
            let dir = config::data_dir();
            model::ensure_natural_voice(&dir, voice)?;
            let bytes = std::fs::read(model::natural_voice_path(&dir, voice))?;
            let table: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            self.voices.insert(voice.to_string(), table);
        }
        let table = &self.voices[voice];
        let mut audio = Vec::new();
        for chunk in tokens.chunks(MAX_PHONEMES) {
            // The voice's style for a passage this long.
            let row = chunk.len().min(table.len() / STYLE - 1);
            let style = table[row * STYLE..(row + 1) * STYLE].to_vec();
            let mut ids = Vec::with_capacity(chunk.len() + 2);
            ids.push(0);
            ids.extend_from_slice(chunk);
            ids.push(0);
            let n = ids.len();
            let outputs = self.session.run(ort::inputs![
                "input_ids" => Tensor::from_array(([1usize, n], ids))?,
                "style" => Tensor::from_array(([1usize, STYLE], style))?,
                "speed" => Tensor::from_array(([1usize], vec![speed]))?,
            ])?;
            let (_, samples) = outputs["waveform"].try_extract_tensor::<f32>()?;
            audio.extend_from_slice(samples);
        }
        Ok(audio)
    }
}

/// eSpeak NG for words misaki doesn't know.
struct Espeak(Mutex<espeak_ng::Translator>);

impl Fallback for Espeak {
    fn phonemize(&self, word: &str) -> Result<String, FallbackError> {
        self.0.lock().unwrap().text_to_ipa(word).map_err(|e| FallbackError::Espeak { word: word.to_string(), error: e.to_string() })
    }
}

/// `text` split into runs of Hindi (Devanagari) and everything else, in order. Words without
/// letters (numbers, punctuation) stay with the run they're in.
fn script_runs(text: &str) -> Vec<(bool, String)> {
    let devanagari = |c: char| ('\u{0900}'..='\u{097F}').contains(&c);
    let mut runs: Vec<(bool, String)> = Vec::new();
    for word in text.split_whitespace() {
        let hindi = if word.chars().any(devanagari) {
            true
        } else if word.chars().any(char::is_alphabetic) {
            false
        } else {
            runs.last().map_or(false, |r| r.0)
        };
        match runs.last_mut() {
            Some((h, run)) if *h == hindi => {
                run.push(' ');
                run.push_str(word);
            }
            _ => runs.push((hindi, word.to_string())),
        }
    }
    runs
}

/// `text` in pieces of up to about `PIECE_CHARS`, split after commas, semicolons and dashes.
fn pieces(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.chars().count() <= PIECE_CHARS {
        return if text.is_empty() { vec![] } else { vec![text.to_string()] };
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
        let breaks = word.ends_with([',', ';', ':']) || word == "—" || word == "-";
        if (breaks && current.chars().count() >= 40) || current.chars().count() >= PIECE_CHARS * 2 {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Kokoro's phoneme ids for misaki's output.
fn tokens(phonemes: &str) -> Vec<i64> {
    // Kokoro writes diphthongs and affricates as one symbol; misaki joins two with U+200D (and
    // eSpeak, for words misaki doesn't know, writes them as two).
    let mut p = phonemes.to_string();
    for (from, to) in [
        ("e\u{200d}ɪ", "A"), ("a\u{200d}ɪ", "I"), ("a\u{200d}ʊ", "W"), ("ɔ\u{200d}ɪ", "Y"), ("o\u{200d}ʊ", "O"), ("ə\u{200d}ʊ", "Q"),
        ("d\u{200d}ʒ", "ʤ"), ("t\u{200d}ʃ", "ʧ"), ("eɪ", "A"), ("aɪ", "I"), ("aʊ", "W"), ("ɔɪ", "Y"), ("oʊ", "O"), ("əʊ", "Q"),
        ("dʒ", "ʤ"), ("tʃ", "ʧ"), ("ɝ", "ɜɹ"), ("\u{200d}", ""),
    ] {
        p = p.replace(from, to);
    }
    // misaki puts spaces around punctuation and between spelled-out letters.
    let p = p.split_whitespace().collect::<Vec<_>>().join(" ");
    let p = [" .", " ,", " !", " ?", " ;", " :"].iter().fold(p, |p, s| p.replace(s, &s[1..]));
    p.chars().filter_map(|c| VOCAB.get(&c).copied()).collect()
}

static VOCAB: LazyLock<HashMap<char, i64>> = LazyLock::new(|| VOCAB_TABLE.iter().copied().collect());

/// Kokoro's phoneme vocabulary (its tokenizer.json).
const VOCAB_TABLE: [(char, i64); 115] = [
    ('$', 0), (';', 1), (':', 2), (',', 3), ('.', 4), ('!', 5), ('?', 6), ('\u{2014}', 9),
    ('\u{2026}', 10), ('"', 11), ('(', 12), (')', 13), ('\u{201c}', 14), ('\u{201d}', 15),
    (' ', 16), ('\u{303}', 17), ('\u{2a3}', 18), ('\u{2a5}', 19), ('\u{2a6}', 20), ('\u{2a8}', 21),
    ('\u{1d5d}', 22), ('\u{ab67}', 23), ('A', 24), ('I', 25), ('O', 31), ('Q', 33), ('S', 35),
    ('T', 36), ('W', 39), ('Y', 41), ('\u{1d4a}', 42), ('a', 43), ('b', 44), ('c', 45), ('d', 46),
    ('e', 47), ('f', 48), ('h', 50), ('i', 51), ('j', 52), ('k', 53), ('l', 54), ('m', 55),
    ('n', 56), ('o', 57), ('p', 58), ('q', 59), ('r', 60), ('s', 61), ('t', 62), ('u', 63),
    ('v', 64), ('w', 65), ('x', 66), ('y', 67), ('z', 68), ('\u{251}', 69), ('\u{250}', 70),
    ('\u{252}', 71), ('\u{e6}', 72), ('\u{3b2}', 75), ('\u{254}', 76), ('\u{255}', 77),
    ('\u{e7}', 78), ('\u{256}', 80), ('\u{f0}', 81), ('\u{2a4}', 82), ('\u{259}', 83),
    ('\u{25a}', 85), ('\u{25b}', 86), ('\u{25c}', 87), ('\u{25f}', 90), ('\u{261}', 92),
    ('\u{265}', 99), ('\u{268}', 101), ('\u{26a}', 102), ('\u{29d}', 103), ('\u{26f}', 110),
    ('\u{270}', 111), ('\u{14b}', 112), ('\u{273}', 113), ('\u{272}', 114), ('\u{274}', 115),
    ('\u{f8}', 116), ('\u{278}', 118), ('\u{3b8}', 119), ('\u{153}', 120), ('\u{279}', 123),
    ('\u{27e}', 125), ('\u{27b}', 126), ('\u{281}', 128), ('\u{27d}', 129), ('\u{282}', 130),
    ('\u{283}', 131), ('\u{288}', 132), ('\u{2a7}', 133), ('\u{28a}', 135), ('\u{28b}', 136),
    ('\u{28c}', 138), ('\u{263}', 139), ('\u{264}', 140), ('\u{3c7}', 142), ('\u{28e}', 143),
    ('\u{292}', 147), ('\u{294}', 148), ('\u{2c8}', 156), ('\u{2cc}', 157), ('\u{2d0}', 158),
    ('\u{2b0}', 162), ('\u{2b2}', 164), ('\u{2193}', 169), ('\u{2192}', 171), ('\u{2197}', 172),
    ('\u{2198}', 173), ('\u{1d7b}', 177),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phoneme_tokens() {
        // "hey, friday!" as misaki writes it.
        assert_eq!(tokens("hˈe\u{200d}ɪ , fɹˈa\u{200d}ɪdˌe\u{200d}ɪ !"), tokens("hˈA, fɹˈIdˌA!"));
        assert!(tokens("tʃˈæt").starts_with(&[VOCAB[&'ʧ']]));
    }

    #[test]
    fn hindi_runs() {
        let runs = script_runs("Here's one: क्यों बच्चा बाहर नहीं जाता? Nice one");
        assert_eq!(runs, vec![(false, "Here's one:".into()), (true, "क्यों बच्चा बाहर नहीं जाता?".into()), (false, "Nice one".into())]);
    }

    #[test]
    fn long_sentences_split_at_commas() {
        let long = "First of all, the meeting moved to Thursday afternoon at three, because the room was taken, and everyone on the team agreed that it would be easier for the people travelling in from Pune.";
        let parts = pieces(long);
        assert!(parts.len() > 1, "{parts:?}");
        assert_eq!(parts.join(" "), long);
        assert_eq!(pieces("Short one."), vec!["Short one."]);
    }
}
