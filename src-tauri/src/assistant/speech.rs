//! Text to speech with the built-in macOS voices (`say`), so it works offline with no download.
//! Sentences are queued and spoken in order on one thread; `stop()` cuts off the current one
//! and drops the rest, which is what makes interrupting feel instant.

use serde::Serialize;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

struct Utterance {
    epoch: u64,
    text: String,
    /// Full `say` voice name; `None` = system voice.
    voice: Option<String>,
    rate: u32,
}

pub struct Speaker {
    tx: mpsc::Sender<Utterance>,
    /// Bumped by `stop()`; queued sentences from an older epoch are skipped.
    epoch: Arc<AtomicU64>,
    /// Sentences queued or being spoken.
    pending: Arc<AtomicUsize>,
    voice: Mutex<(Option<String>, u32)>,
}

impl Speaker {
    pub fn new(voice: &str, rate: u32) -> Self {
        let (tx, rx) = mpsc::channel::<Utterance>();
        let epoch = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(AtomicUsize::new(0));
        let (e, p) = (epoch.clone(), pending.clone());
        std::thread::Builder::new()
            .name("murmur-speech".into())
            .spawn(move || {
                for u in rx {
                    if u.epoch == e.load(Ordering::SeqCst) {
                        speak_one(&u, &e);
                    }
                    p.fetch_sub(1, Ordering::SeqCst);
                }
            })
            .expect("spawn speech thread");
        Self { tx, epoch, pending, voice: Mutex::new((resolve(voice), rate)) }
    }

    /// Switches voice / speed for everything said from now on.
    pub fn configure(&self, voice: &str, rate: u32) {
        *self.voice.lock().unwrap() = (resolve(voice), rate);
    }

    pub fn say(&self, text: &str) {
        let (voice, rate) = self.voice.lock().unwrap().clone();
        self.enqueue(text, voice, rate);
    }

    /// Stops whatever is playing and says `text` in the given voice (Settings' ▶ button).
    pub fn preview(&self, voice: &str, rate: u32, text: &str) {
        self.stop();
        self.enqueue(text, resolve(voice), rate);
    }

    fn enqueue(&self, text: &str, voice: Option<String>, rate: u32) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        let u = Utterance { epoch: self.epoch.load(Ordering::SeqCst), text: text.to_string(), voice, rate };
        if self.tx.send(u).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Stops speaking now and forgets anything queued.
    pub fn stop(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    pub fn is_speaking(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0
    }

    /// Blocks until everything queued has been spoken, `keep_waiting` returns false, or a
    /// safety timeout passes.
    pub fn wait(&self, keep_waiting: impl Fn() -> bool) {
        let started = Instant::now();
        while self.is_speaking() && keep_waiting() && started.elapsed() < Duration::from_secs(300) {
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}

fn speak_one(u: &Utterance, epoch: &AtomicU64) {
    let mut cmd = Command::new("say");
    if let Some(v) = &u.voice {
        cmd.args(["-v", v]);
    }
    // Text goes in on stdin so a sentence starting with "-" isn't read as a flag.
    cmd.args(["-r", &u.rate.to_string()]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            mlog!("speech: could not start `say`: {e}");
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(u.text.as_bytes());
    }
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {}
        }
        if epoch.load(Ordering::SeqCst) != u.epoch {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Voice {
    /// What goes in config ("Tara"); also how it's shown.
    pub name: String,
    /// e.g. "en_IN".
    pub locale: String,
}

/// Novelty voices that sound like sound effects rather than an assistant.
const NOVELTY: &[&str] = &[
    "Albert", "Bad News", "Bahh", "Bells", "Boing", "Bubbles", "Cellos", "Fred", "Good News", "Jester", "Junior",
    "Kathy", "Organ", "Ralph", "Superstar", "Trinoids", "Whisper", "Wobble", "Zarvox",
];

/// Installed voices as (full `say` name, short name, locale).
fn installed() -> Vec<(String, String, String)> {
    let Ok(out) = Command::new("say").args(["-v", "?"]).output() else { return vec![] };
    // Lines look like "Daniel              en_GB    # Hello! My name is Daniel." or
    // "Tara (English (India)) en_IN    # ...": the name is everything before the locale column.
    let locale = regex::Regex::new(r"\s+([a-z]{2,3}_[A-Za-z0-9]+)\s+#").unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let caps = locale.captures(l)?;
            let name = l[..caps.get(0)?.start()].trim().to_string();
            let short = name.split(" (").next().unwrap_or(&name).to_string();
            Some((name, short, caps[1].to_string()))
        })
        .collect()
}

/// Voices worth offering in Settings, one per name, sorted by language then name.
/// Names that exist in several accents (e.g. "Eddy (English (UK))") are kept in full.
pub fn voices() -> Vec<Voice> {
    let all = installed();
    let mut out: Vec<Voice> = Vec::new();
    for (name, short, locale) in &all {
        if NOVELTY.contains(&short.as_str()) {
            continue;
        }
        let ambiguous = all.iter().any(|(_, s, l)| s == short && l != locale);
        let shown = if ambiguous { name.clone() } else { short.clone() };
        if !out.iter().any(|v| v.name == shown) {
            out.push(Voice { name: shown, locale: locale.clone() });
        }
    }
    out.sort_by(|a, b| a.locale.cmp(&b.locale).then(a.name.cmp(&b.name)));
    out
}

/// The full `say` name for a configured voice ("Tara" → "Tara (English (India))"), or `None`
/// (system voice) if it isn't installed.
fn resolve(wanted: &str) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    let found = installed()
        .into_iter()
        .find(|(name, short, _)| name.eq_ignore_ascii_case(wanted) || short.eq_ignore_ascii_case(wanted))
        .map(|(name, _, _)| name);
    if found.is_none() {
        mlog!("voice '{wanted}' is not installed; using the system voice");
    }
    found
}
