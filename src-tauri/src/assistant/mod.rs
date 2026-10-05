//! The assistant: hold the key to ask, tap it for a hands-free conversation. Answers are spoken
//! and can act on the Mac (see `actions`).

pub mod actions;
pub mod brain;
mod duck;
pub mod history;
pub mod kokoro;
pub mod llm;
mod player;
pub mod speech;
mod voice;

use crate::hotkey::HotkeyEvent;
use crate::{cleanup, config, gesture, key_label, model, play_sound, Mode, Shared, MIN_SECONDS, SILENCE_RMS};
use brain::Brain;
use duck::Ducker;
use config::Config;
use llm::{Message, SentenceSplitter};
use serde::Serialize;
use speech::Speaker;
use voice::VoiceDetector;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Don't ask the AI to load its model more often than this.
const WARM_UP_EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Idle,
    Recording,
    /// Transcribing, thinking or speaking. A hotkey press interrupts it.
    Responding,
}

#[derive(Serialize, Clone)]
struct WidgetState<'a> {
    mode: &'static str,
    state: &'a str,
    message: Option<String>,
    progress: Option<f32>,
    question: Option<&'a str>,
    reply: Option<&'a str>,
}

/// Hotkey timing state used to tell holds from taps.
#[derive(Default)]
struct Gesture {
    pressed_at: Option<Instant>,
    /// This press cut off an answer; if it's only a tap, stopping was the whole point.
    interrupted: bool,
    /// This press was handled on the way down (e.g. it ended a conversation); skip its release.
    ignore_release: bool,
}

/// Voice activity detection for conversation mode: notices when you've finished speaking.
struct Vad {
    started: Instant,
    last_tick: Instant,
    last_voice: Instant,
    /// Accumulated talking time; speech counts as started once it passes `MIN_SPEECH`.
    voice: Duration,
    /// How long the level has been over the threshold without a break.
    loud_run: Duration,
    heard_speech: bool,
    /// When speech first began, for the log.
    first_voice: Option<Instant>,
    /// Recent levels; the quietest of them is the room's background noise (a fan, AC or
    /// traffic sets it, while the gaps between words don't fool it).
    recent: std::collections::VecDeque<(Instant, f32)>,
    /// The threshold that started speech, measured on the room before you spoke. Kept from then
    /// on, so talking can't move it.
    start_threshold: f32,
    /// Typical level of your voice (average of loud moments); 0 until you speak.
    speech_level: f32,
    /// Loudest level heard, and the last threshold used, for the log.
    peak: f32,
    threshold: f32,
    /// Short sounds ignored because they didn't last (clicks, breaths), for the log.
    blips: u32,
    /// A decision (send / give up) has been made for this recording.
    decided: bool,
    /// The speech detector judges this recording; the level meter then only learns the room.
    by_detector: bool,
    /// Recorded audio (16 kHz samples) the speech detector has judged so far.
    judged_until: Option<usize>,
    /// Loud sound the speech detector found wasn't speech (typing, clicks), for the log.
    ignored: Duration,
    /// The mic has delivered audio for this recording.
    got_audio: bool,
}

impl Vad {
    fn new() -> Self {
        Self::after(None)
    }

    /// Starts listening again, remembering the room's background from just before, so speaking
    /// the moment the mic opens isn't mistaken for the background. Only a recent background
    /// counts: one from minutes ago may predate a song that's playing now, and would make that
    /// song pass for speech.
    fn after(previous: Option<&Vad>) -> Self {
        let now = Instant::now();
        let mut recent = std::collections::VecDeque::new();
        let previous = previous.filter(|v| now - v.last_tick < RECENT_BACKGROUND);
        if let Some(floor) = previous.map(Vad::noise_floor).filter(|f| *f < 1.0) {
            recent.push_back((now, floor));
        }
        Self {
            started: now,
            last_tick: now,
            last_voice: now,
            voice: Duration::ZERO,
            loud_run: Duration::ZERO,
            heard_speech: false,
            first_voice: None,
            recent,
            start_threshold: 0.0,
            speech_level: 0.0,
            peak: 0.0,
            threshold: 0.0,
            blips: 0,
            decided: false,
            by_detector: false,
            judged_until: None,
            ignored: Duration::ZERO,
            got_audio: false,
        }
    }

    fn noise_floor(&self) -> f32 {
        self.recent.iter().map(|&(_, l)| l).fold(f32::MAX, f32::min).min(1.0)
    }

    /// Takes the next mic level and decides whether you've finished speaking. `sensitivity`
    /// is the Mic sensitivity setting (lower = picks up quieter voices).
    fn step(&mut self, now: Instant, level: f32, sensitivity: f32, pause: Duration, timeout: Duration) -> VadDecision {
        let dt = now - self.last_tick;
        self.last_tick = now;
        self.got_audio = true;
        self.peak = self.peak.max(level);
        let threshold = if !self.heard_speech {
            // Before you speak: learn the room, and call it speech once it's clearly louder than
            // the room (judged against the room itself, so a quiet voice in a quiet room counts
            // and a fan doesn't).
            self.recent.push_back((now, level));
            while self.recent.front().is_some_and(|&(t, _)| now - t > NOISE_WINDOW) {
                self.recent.pop_front();
            }
            let scale = sensitivity / NORMAL_SENSITIVITY;
            (self.noise_floor() * SPEECH_OVER_NOISE * scale).max(MIN_SPEECH_LEVEL * scale)
        } else {
            // While you speak: only your voice keeps it open. Never lower than what started
            // speech, and at least a fair part of your speaking level, so room noise, typing
            // and breathing between sentences count as the pause they are.
            self.start_threshold.max(self.speech_level * VOICE_SHARE)
        };
        self.threshold = threshold;
        if self.by_detector {
            return VadDecision::Continue;
        }
        if level > threshold {
            self.loud_run += dt;
        } else {
            if self.loud_run > Duration::ZERO && self.loud_run < SUSTAIN {
                self.blips += 1;
            }
            self.loud_run = Duration::ZERO;
            if !self.heard_speech {
                // Isolated sounds fade instead of adding up to "speech".
                self.voice = self.voice.saturating_sub(dt / 2);
            }
        }
        if self.loud_run >= SUSTAIN {
            // Only sound about as loud as your voice may update its level. Quieter sound still
            // keeps listening open, but if it could pull the level down, the threshold would follow
            // it, and background sound (a song, a video) would hold listening open until you
            // out-shouted it.
            if self.speech_level == 0.0 {
                self.speech_level = level;
            } else if !self.heard_speech || level >= self.speech_level * VOICE_LIKE {
                self.speech_level = self.speech_level * 0.9 + level * 0.1;
            }
            self.voice += dt;
            self.last_voice = now;
            if self.voice >= MIN_SPEECH && !self.heard_speech {
                self.heard_speech = true;
                self.start_threshold = threshold;
                self.first_voice = Some(now - self.voice);
            }
        }
        self.decide(now, pause, timeout)
    }

    /// Takes the speech detector's verdict on the newest 32 ms windows (speech?, level), oldest
    /// first, the last one ending `now`. Only speech that is also clearly louder than the room
    /// counts as you talking, so a far-off voice doesn't either.
    fn hear(&mut self, now: Instant, windows: &[(bool, f32)], pause: Duration, timeout: Duration) -> VadDecision {
        let n = windows.len() as u32;
        for (i, &(speech, level)) in windows.iter().enumerate() {
            let at = now - voice::WINDOW_TIME * (n - 1 - i as u32);
            let gate = if self.heard_speech { self.start_threshold } else { self.threshold };
            if speech && level > gate {
                self.speech_level = if self.speech_level == 0.0 { level } else { self.speech_level * 0.9 + level * 0.1 };
                self.voice += voice::WINDOW_TIME;
                self.last_voice = at;
                if self.voice >= MIN_SPEECH && !self.heard_speech {
                    self.heard_speech = true;
                    self.start_threshold = gate;
                    self.first_voice = Some(at - self.voice);
                }
            } else {
                if level > gate {
                    self.ignored += voice::WINDOW_TIME;
                }
                if !self.heard_speech {
                    self.voice = self.voice.saturating_sub(voice::WINDOW_TIME / 2);
                }
            }
        }
        self.decide(now, pause, timeout)
    }

    fn decide(&self, now: Instant, pause: Duration, timeout: Duration) -> VadDecision {
        if self.heard_speech && (now - self.last_voice >= pause || now - self.started >= MAX_UTTERANCE) {
            VadDecision::Send
        } else if !self.heard_speech && now - self.started >= timeout {
            VadDecision::GiveUp
        } else {
            VadDecision::Continue
        }
    }

    /// Logs a send / give-up decision.
    fn log_decision(&self, now: Instant, decision: &VadDecision) {
        let since = |t: Instant| (t - self.started).as_secs_f32();
        let ignored = if self.by_detector {
            format!("{:.1}s of loud non-speech ignored (speech detector)", self.ignored.as_secs_f32())
        } else {
            format!("{} short sounds ignored (loudness only)", self.blips)
        };
        mlog!(
            "vad: {} after {:.1}s (speech {:.1}s–{:.1}s, then {:.1}s quiet) — background {:.4}, threshold {:.4}, voice {:.4}, loudest {:.4}, {ignored}",
            match decision {
                VadDecision::Send if now - self.started >= MAX_UTTERANCE => "sent at the length limit",
                VadDecision::Send => "pause detected",
                _ => "no speech",
            },
            since(now),
            self.first_voice.map(since).unwrap_or_default(),
            since(self.last_voice),
            (now - self.last_voice).as_secs_f32(),
            self.noise_floor(),
            self.threshold,
            self.speech_level,
            self.peak,
        );
    }
}

/// What the level meter decided about the current conversation-mode recording.
enum VadDecision {
    Continue,
    /// You stopped talking: send it.
    Send,
    /// Nothing said for a while: end the conversation.
    GiveUp,
}

/// Once you're speaking, sound only counts as more speech if it reaches this share of your
/// speaking level.
const VOICE_SHARE: f32 = 0.35;
/// Once you're speaking, only sound reaching this share of your speaking level updates that level.
const VOICE_LIKE: f32 = 0.6;
/// The background heard on the last listen is reused only if that ended this recently.
const RECENT_BACKGROUND: Duration = Duration::from_secs(30);
/// Loud time needed before a recording counts as speech (filters out clicks and coughs).
const MIN_SPEECH: Duration = Duration::from_millis(250);
/// Longest single utterance in conversation mode before it is sent anyway.
const MAX_UTTERANCE: Duration = Duration::from_secs(30);
/// How far back the background-noise estimate looks.
const NOISE_WINDOW: Duration = Duration::from_secs(3);
/// Speech must be this many times louder than the background (at "Normal" mic sensitivity).
const SPEECH_OVER_NOISE: f32 = 2.5;
/// In a near-silent room, speech still has to reach this level (at "Normal" sensitivity).
const MIN_SPEECH_LEVEL: f32 = 0.005;
/// The "Normal" mic sensitivity setting; others scale the thresholds from it.
const NORMAL_SENSITIVITY: f32 = 0.012;
/// Only sound that lasts this long counts as talking: clicks, taps and breaths are shorter, and
/// must neither start an utterance nor keep one open.
const SUSTAIN: Duration = Duration::from_millis(100);
/// No audio from the mic for this long while listening: it has stopped, so stop waiting.
const MIC_SILENT: Duration = Duration::from_secs(2);
/// Same, before the mic has delivered anything at all: opening it can be slow.
const MIC_START: Duration = Duration::from_secs(8);
/// Pause after the assistant stops speaking before the mic reopens, so it doesn't hear its own echo.
const ECHO_GUARD: Duration = Duration::from_millis(300);

/// Recent questions and answers, sent back to the model so follow-ups make sense.
#[derive(Default)]
struct Conversation {
    messages: Vec<Message>,
    last_at: Option<Instant>,
}

pub struct Assistant {
    app: AppHandle,
    shared: Arc<Shared>,
    pub brain: Brain,
    pub speaker: Speaker,
    /// Turns the Mac's sound down while the mic listens.
    ducker: Ducker,
    /// Tells your voice from typing, clicks and other sound in conversation mode.
    voice: VoiceDetector,
    conversation: Mutex<Conversation>,
    phase: Mutex<Phase>,
    /// Bumped on every interruption; a response thread whose turn is stale goes quiet.
    turn: AtomicU64,
    gesture: Mutex<Gesture>,
    /// Conversation mode: listen, answer, listen again, until ended.
    in_conversation: AtomicBool,
    vad: Mutex<Vad>,
    last_warm_up: Mutex<Option<Instant>>,
    /// Running timers: id, label, when it ends.
    timers: Mutex<Vec<(u64, String, Instant)>>,
    /// Ids for timers and the hidden windows pages are read in.
    next_id: AtomicU64,
    /// While the assistant waits for a yes or no: where the widget's buttons send the answer.
    pub pending_confirm: Mutex<Option<std::sync::mpsc::Sender<bool>>>,
}

impl Assistant {
    pub fn new(shared: Arc<Shared>) -> Self {
        let cfg = shared.cfg();
        Self {
            app: shared.app.clone(),
            brain: Brain::new(&cfg),
            speaker: Speaker::new(&cfg),
            ducker: Ducker::new(),
            voice: VoiceDetector::new(),
            shared,
            conversation: Mutex::new(Conversation::default()),
            phase: Mutex::new(Phase::Idle),
            turn: AtomicU64::new(0),
            gesture: Mutex::new(Gesture::default()),
            in_conversation: AtomicBool::new(false),
            vad: Mutex::new(Vad::new()),
            last_warm_up: Mutex::new(None),
            timers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            pending_confirm: Mutex::new(None),
        }
    }

    fn cfg(&self) -> Config {
        self.shared.cfg()
    }

    fn set_status(&self, text: &str) {
        self.shared.set_status(text);
    }

    fn ready_status(&self) {
        self.shared.ready_status();
    }

    /// Why the assistant can't answer yet, if it can't (turned off, or its AI isn't downloaded).
    pub fn unavailable(&self) -> Option<String> {
        let cfg = self.cfg();
        if !cfg.assistant_enabled {
            return Some("The assistant is turned off in Settings".into());
        }
        if cfg.brain != "ollama" && !model::brain_path(&config::data_dir(), &cfg.builtin_model).exists() {
            return Some("Download the assistant's AI in Murmur → Assistant".into());
        }
        None
    }

    pub fn phase(&self) -> Phase {
        *self.phase.lock().unwrap()
    }

    fn set_phase(&self, p: Phase) {
        *self.phase.lock().unwrap() = p;
    }

    fn current(&self, turn: u64) -> bool {
        self.turn.load(Ordering::SeqCst) == turn
    }

    /// Stops whatever is being said or generated. Done under the phase lock so a response
    /// thread that is just finishing can't overwrite the new phase.
    fn interrupt(&self) {
        let mut phase = self.phase.lock().unwrap();
        self.turn.fetch_add(1, Ordering::SeqCst);
        self.speaker.stop();
        let _ = self.app.emit_to("main", "reply-stopped", ());
        if *phase == Phase::Responding {
            *phase = Phase::Idle;
        }
    }

    /// Ends a response, unless it was interrupted. Returns false if it was.
    fn finish_turn(&self, turn: u64) -> bool {
        let mut phase = self.phase.lock().unwrap();
        if self.current(turn) && *phase == Phase::Responding {
            *phase = Phase::Idle;
            true
        } else {
            false
        }
    }

    fn emit(&self, state: &str, message: Option<String>, progress: Option<f32>) {
        let _ = self.app.emit_to("widget", "widget-state", WidgetState { mode: "assistant", state, message, progress, question: None, reply: None });
    }

    /// Live answer for the widget and the app window's Home page.
    fn emit_reply(&self, turn: u64, state: &str, question: &str, reply: &str) {
        if self.current(turn) {
            let payload = WidgetState { mode: "assistant", state, message: None, progress: None, question: Some(question), reply: Some(reply) };
            let _ = self.app.emit_to("widget", "widget-state", &payload);
            let _ = self.app.emit_to("main", "reply", &payload);
        }
    }

    fn show_widget(&self) {
        self.shared.show_widget(Mode::Assistant);
    }

    fn hide_widget(&self) {
        self.shared.hide_widget();
    }

    fn play(&self, sound: &str) {
        self.shared.play(sound);
    }

    /// Hides after `delay`, unless something new started (or the widget was shown again) in
    /// the meantime.
    fn hide_widget_later(self: &Arc<Self>, delay: Duration) {
        let core = self.clone();
        let turn = self.turn.load(Ordering::SeqCst);
        let shown = self.shared.widget_shown();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if core.current(turn) && core.phase() == Phase::Idle {
                core.shared.hide_widget_since(shown);
            }
        });
    }

    pub fn conversing(&self) -> bool {
        self.in_conversation.load(Ordering::SeqCst)
    }

    /// Returns true when the event was consumed (only matters for Esc).
    pub fn on_hotkey(self: &Arc<Self>, event: HotkeyEvent) -> bool {
        let now = Instant::now();
        match event {
            HotkeyEvent::Pressed => {
                let phase = self.phase();
                if self.conversing() {
                    // Tap while it talks: stop it and listen. Tap after you've spoken: send it now
                    // (when people nearby talk as loud as you, no pause ever comes). Tap while it
                    // waits for you to speak: end.
                    self.gesture.lock().unwrap().ignore_release = true;
                    let send_now = phase == Phase::Recording && {
                        let mut v = self.vad.lock().unwrap();
                        let send = v.heard_speech && !v.decided;
                        v.decided |= send;
                        send
                    };
                    if phase == Phase::Responding {
                        mlog!("interrupted (conversation continues)");
                        self.interrupt();
                        self.listen(true);
                    } else if send_now {
                        mlog!("sent with the hotkey");
                        self.finish_recording();
                    } else {
                        self.end_conversation("ended with the hotkey");
                    }
                    return false;
                }
                if self.shared.loading() {
                    self.emit("error", Some("Still loading…".into()), None);
                    self.show_widget();
                    self.hide_widget_later(Duration::from_millis(1500));
                } else if let Some(why) = self.unavailable() {
                    self.emit("error", Some(why), None);
                    self.show_widget();
                    self.hide_widget_later(Duration::from_millis(4000));
                } else if matches!(phase, Phase::Idle | Phase::Responding) {
                    {
                        let mut g = self.gesture.lock().unwrap();
                        g.pressed_at = Some(now);
                        g.interrupted = phase == Phase::Responding;
                    }
                    if phase == Phase::Responding {
                        mlog!("interrupted");
                        self.interrupt();
                    }
                    // Not waiting for the mic here: see `open_mic`.
                    let opened = self.open_mic();
                    self.start_hold_widget();
                    let core = self.clone();
                    std::thread::spawn(move || {
                        core.mic_opened(opened.recv().unwrap_or_else(|e| Err(anyhow::anyhow!("{e}"))));
                    });
                }
                false
            }
            HotkeyEvent::Released => {
                let mut g = self.gesture.lock().unwrap();
                if std::mem::take(&mut g.ignore_release) || self.conversing() || self.phase() != Phase::Recording {
                    return false;
                }
                let held = g.pressed_at.map(|t| now - t).unwrap_or_default();
                let interrupted = g.interrupted;
                drop(g);
                if !gesture::is_tap(held) {
                    mlog!("hotkey held {} ms → quick question", held.as_millis());
                    self.finish_recording();
                } else if interrupted {
                    // The tap was only to stop the answer.
                    self.cancel_recording();
                } else {
                    mlog!("hotkey tapped → conversation");
                    self.start_conversation();
                }
                false
            }
            HotkeyEvent::Cancelled => {
                if !self.conversing() {
                    self.cancel_recording();
                }
                false
            }
            HotkeyEvent::Escape => {
                if self.conversing() {
                    self.end_conversation("ended with Esc");
                    return true;
                }
                match self.phase() {
                    Phase::Recording => {
                        mlog!("cancelled with Esc");
                        self.cancel_recording();
                        true
                    }
                    Phase::Responding => {
                        mlog!("stopped with Esc");
                        self.interrupt();
                        self.hide_widget();
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    /// Opens the mic and waits until it is. In conversation mode the level meter also watches
    /// for the end of speech.
    fn start_recording(self: &Arc<Self>) -> bool {
        let opened = self.open_mic();
        self.mic_opened(opened.recv().unwrap_or_else(|e| Err(anyhow::anyhow!("{e}"))))
    }

    /// Starts opening the mic and returns at once with where the outcome will arrive. The
    /// hotkey's thread uses this so it's free to see a quick tap's release when it happens.
    fn open_mic(self: &Arc<Self>) -> std::sync::mpsc::Receiver<anyhow::Result<()>> {
        self.set_phase(Phase::Recording);
        {
            let mut vad = self.vad.lock().unwrap();
            *vad = Vad::after(Some(&vad));
        }
        let core = self.clone();
        let on_level = Box::new(move |level: f32| {
            let _ = core.app.emit_to("widget", "level", level);
            if core.conversing() {
                core.on_level(level);
            }
        });
        let cfg = self.cfg();
        self.shared.preload_transcriber();
        self.ducker.duck();
        self.shared.recorder.start_async(cfg.input_device, on_level)
    }

    /// The mic has opened, or couldn't. True when recording.
    fn mic_opened(self: &Arc<Self>, result: anyhow::Result<()>) -> bool {
        if let Err(e) = result {
            self.ducker.restore();
            self.set_phase(Phase::Idle);
            self.in_conversation.store(false, Ordering::SeqCst);
            self.fail(&format!("Mic error: {e}"));
            return false;
        }
        self.warm_up_llm();
        true
    }

    /// Quick question: the recording runs while the key is held.
    fn start_hold_widget(self: &Arc<Self>) {
        self.emit("recording", None, None);
        // Show only if this turns out to be a hold, so quick taps never flash the widget.
        let core = self.clone();
        let pressed_at = self.gesture.lock().unwrap().pressed_at;
        std::thread::spawn(move || {
            std::thread::sleep(gesture::SHOW_DELAY);
            let same_press = core.gesture.lock().unwrap().pressed_at == pressed_at;
            if core.phase() == Phase::Recording && same_press && !core.conversing() {
                core.show_widget();
                core.play("Tink");
            }
        });
    }

    /// The tap's recording keeps running; from now on a pause sends it.
    fn start_conversation(self: &Arc<Self>) {
        self.in_conversation.store(true, Ordering::SeqCst);
        self.ducker.duck_now();
        // Count from now: the tap itself (and any sound before it was turned down) isn't speech.
        {
            let mut vad = self.vad.lock().unwrap();
            *vad = Vad::after(Some(&vad));
        }
        self.emit("listening", None, None);
        self.show_widget();
        self.play("Tink");
        self.watch_mic();
        self.set_status(&format!("In conversation — tap {} to send now, Esc to end", key_label(&self.cfg().assistant_hotkey)));
    }

    /// Conversation mode: runs the speech detector on the new audio a few times a second and
    /// decides when you've finished. Also, if the mic stops delivering audio, nothing would ever
    /// decide the utterance is over and it would sit on "Listening": notice that and move on.
    fn watch_mic(self: &Arc<Self>) {
        let core = self.clone();
        let turn = self.turn.load(Ordering::SeqCst);
        std::thread::spawn(move || loop {
            std::thread::sleep(voice::WINDOW_TIME * 8);
            if !core.conversing() || !core.current(turn) || core.phase() != Phase::Recording {
                return;
            }
            if core.detect_voice() {
                return;
            }
            let (decided, silent, heard) = {
                let v = core.vad.lock().unwrap();
                // The first open after launch can take a few seconds to deliver audio.
                let limit = if v.got_audio { MIC_SILENT } else { MIC_START };
                (v.decided, v.last_tick.elapsed() >= limit, v.heard_speech)
            };
            if decided {
                return;
            }
            if silent {
                core.vad.lock().unwrap().decided = true;
                mlog!("vad: the mic stopped sending audio; {}", if heard { "sending what was said" } else { "ending the conversation" });
                if heard {
                    core.finish_recording();
                } else {
                    core.end_conversation("the mic stopped");
                }
                return;
            }
        });
    }

    /// Judges the audio recorded since the last look. True once a decision has been made.
    fn detect_voice(self: &Arc<Self>) -> bool {
        let Some(tail) = self.shared.recorder.peek(voice::CONTEXT) else { return false };
        let judged = {
            let mut v = self.vad.lock().unwrap();
            if v.decided {
                return true;
            }
            // First look at this recording: judge everything since listening (re)started.
            let since_start = (v.started.elapsed().as_secs_f64() * crate::audio::TARGET_RATE as f64) as usize;
            *v.judged_until.get_or_insert(tail.total.saturating_sub(since_start))
        };
        let new = (tail.total.saturating_sub(judged) / voice::WINDOW).min(tail.samples.len() / voice::WINDOW);
        if new == 0 {
            return false;
        }
        // Whole windows, lined up so the last one ends with the newest audio.
        let samples = &tail.samples[tail.samples.len() % voice::WINDOW..];
        let Some(windows) = self.voice.windows(samples) else { return false };
        let now = Instant::now();
        let decision = {
            let cfg = self.shared.cfg.read().unwrap();
            let mut v = self.vad.lock().unwrap();
            if v.decided || v.judged_until.is_none() {
                return v.decided;
            }
            if !v.by_detector {
                // Take over from the level meter, from a clean slate.
                v.by_detector = true;
                v.heard_speech = false;
                v.voice = Duration::ZERO;
                v.first_voice = None;
                v.last_voice = v.started;
            }
            v.judged_until = Some(judged + new * voice::WINDOW);
            let pause = Duration::from_secs_f32(cfg.pause_seconds.max(0.4));
            let timeout = Duration::from_secs(cfg.conversation_timeout_seconds.max(3) as u64);
            let decision = v.hear(now, &windows[windows.len() - new..], pause, timeout);
            if !matches!(decision, VadDecision::Continue) {
                v.decided = true;
                v.log_decision(now, &decision);
            }
            decision
        };
        match decision {
            VadDecision::Send => self.finish_recording(),
            VadDecision::GiveUp => self.end_conversation("timed out"),
            VadDecision::Continue => return false,
        }
        true
    }

    /// Conversation mode: open the mic for the next thing the user says.
    fn listen(self: &Arc<Self>, immediately: bool) {
        if !immediately {
            std::thread::sleep(ECHO_GUARD);
        }
        if !self.conversing() || !matches!(self.phase(), Phase::Idle | Phase::Responding) {
            return;
        }
        self.ducker.duck_now();
        if self.start_recording() {
            self.emit("listening", None, None);
            self.show_widget();
            self.watch_mic();
        }
    }

    fn end_conversation(self: &Arc<Self>, why: &str) {
        if !self.in_conversation.swap(false, Ordering::SeqCst) {
            return;
        }
        mlog!("conversation {why}");
        self.interrupt();
        if self.phase() == Phase::Recording {
            let _ = self.stop_mic();
            self.set_phase(Phase::Idle);
        }
        self.play("Bottle");
        self.ready_status();
        self.hide_widget();
    }

    /// Called ~30 times a second with the mic level while conversing.
    fn on_level(self: &Arc<Self>, level: f32) {
        let decision = {
            let cfg = self.shared.cfg.read().unwrap();
            let mut v = self.vad.lock().unwrap();
            if v.decided {
                return;
            }
            let pause = Duration::from_secs_f32(cfg.pause_seconds.max(0.4));
            let timeout = Duration::from_secs(cfg.conversation_timeout_seconds.max(3) as u64);
            let now = Instant::now();
            let decision = v.step(now, level, cfg.speech_threshold, pause, timeout);
            if !matches!(decision, VadDecision::Continue) {
                v.decided = true;
                v.log_decision(now, &decision);
            }
            decision
        };
        // Stopping the mic from inside its own callback would deadlock: hand it off.
        let core = self.clone();
        match decision {
            VadDecision::Send => {
                std::thread::spawn(move || core.finish_recording());
            }
            VadDecision::GiveUp => {
                std::thread::spawn(move || core.end_conversation("timed out"));
            }
            VadDecision::Continue => {}
        }
    }

    /// Stops the mic and turns the sound back up.
    fn stop_mic(&self) -> anyhow::Result<crate::audio::Recording> {
        let rec = self.shared.recorder.stop();
        self.ducker.restore();
        rec
    }

    /// Puts the sound back if it was turned down (Murmur is quitting).
    pub fn restore_sound(&self) {
        self.ducker.restore();
    }

    fn cancel_recording(self: &Arc<Self>) {
        if self.phase() != Phase::Recording {
            return;
        }
        let _ = self.stop_mic();
        self.set_phase(Phase::Idle);
        self.hide_widget();
    }

    fn finish_recording(self: &Arc<Self>) {
        if self.phase() != Phase::Recording {
            return;
        }
        self.set_phase(Phase::Responding);
        self.play("Pop");
        let turn = self.turn.load(Ordering::SeqCst);
        let core = self.clone();
        std::thread::spawn(move || {
            let result = core.respond(turn);
            if !core.finish_turn(turn) {
                return;
            }
            match result {
                Ok(_) if core.conversing() => core.listen(false),
                Ok(hide_after) => core.hide_widget_later(hide_after),
                Err(e) => {
                    core.in_conversation.store(false, Ordering::SeqCst);
                    core.ready_status();
                    core.fail(&e.to_string());
                }
            }
        });
    }

    /// Recording → question → streamed answer, spoken sentence by sentence.
    /// Returns how long to keep the answer on screen afterwards.
    fn respond(&self, turn: u64) -> anyhow::Result<Duration> {
        let rec = self.stop_mic()?;
        if rec.seconds() < MIN_SECONDS || rec.peak_rms < SILENCE_RMS {
            mlog!("skipped clip ({:.2}s, peak rms {:.4})", rec.seconds(), rec.peak_rms);
            return Ok(Duration::ZERO);
        }
        self.emit("transcribing", None, None);
        let cfg = self.cfg();
        let transcriber = self.shared.ensure_transcriber()?;
        let t0 = Instant::now();
        let raw = transcriber.transcribe(&rec.samples, &cfg.language, false, &[])?;
        let heard_ms = t0.elapsed().as_millis();
        if cleanup::is_hallucination(&raw) {
            return Ok(Duration::ZERO);
        }
        let question = cleanup::clean_question(&raw);
        if question.is_empty() || !self.current(turn) {
            return Ok(Duration::ZERO);
        }
        mlog!("heard ({:.1}s audio, {heard_ms} ms): {question}", rec.seconds());
        let mode = if self.conversing() { "conversation" } else { "quick" };
        self.answer(turn, &question, mode, rec.seconds(), heard_ms as u64)
    }

    /// Streams the model's answer to `question`, speaking it sentence by sentence, and logs it.
    /// Returns how long to keep the answer on screen afterwards.
    fn answer(&self, turn: u64, question: &str, mode: &str, audio_seconds: f32, heard_ms: u64) -> anyhow::Result<Duration> {
        let cfg = self.cfg();
        self.emit_reply(turn, "thinking", question, "");
        let messages = self.messages_for(&cfg, question);
        let mut reply = String::new();
        // Shared by the two callbacks below: text before an action is spoken when it starts.
        let working = std::cell::Cell::new(false);
        let splitter = std::cell::RefCell::new(SentenceSplitter::default());
        // Sentences said so far: a closing offer ("Let me know if…") after the answer is dropped.
        let said = std::cell::Cell::new(0usize);
        let say = |sentence: &str| {
            if said.get() > 0 && llm::is_offer(sentence) {
                return;
            }
            said.set(said.get() + 1);
            self.speaker.say(&llm::speakable(sentence));
        };
        let steps = std::cell::RefCell::new(Vec::<String>::new());
        let mut guard = llm::LoopGuard::default();
        let mut loop_start = None;
        let mut first_token_ms = None;
        let t1 = Instant::now();
        let host = self.app.state::<Arc<Assistant>>().inner().clone();
        let result = actions::converse(&self.brain, &cfg, host.as_ref(), messages, || self.current(turn), |piece| {
            if !self.current(turn) {
                return false;
            }
            first_token_ms.get_or_insert(t1.elapsed().as_millis());
            if working.take() && !reply.is_empty() {
                // Words said before an action and after it are separate sentences.
                reply.push(' ');
                guard.push(" ");
            }
            reply.push_str(piece);
            // Checked before speaking, so the repeat that gives the loop away is never said.
            if let Some(start) = guard.push(piece) {
                loop_start = Some(start);
                return false;
            }
            if cfg.speak_replies {
                for sentence in splitter.borrow_mut().push(piece) {
                    say(&sentence);
                }
            }
            self.emit_reply(turn, "speaking", question, reply.trim());
            true
        }, |what| {
            if !self.current(turn) {
                return false;
            }
            if cfg.speak_replies {
                // Say what came before the action now, rather than after it's done.
                if let Some(rest) = splitter.borrow_mut().finish() {
                    say(&rest);
                }
            }
            working.set(true);
            // A task of several steps shows the last ones done, then the one under way.
            let mut steps = steps.borrow_mut();
            let shown: Vec<String> = steps.iter().rev().take(2).rev().map(|s: &String| format!("✓ {s}")).collect();
            steps.push(what.to_string());
            let line = if shown.is_empty() { format!("{what}…") } else { format!("{} · {what}…", shown.join(" · ")) };
            self.emit_reply(turn, "thinking", question, &line);
            true
        });
        if let Some(start) = loop_start {
            // Keep only what came before the loop, so it is neither shown, remembered (it would
            // pull the next answers into the same rut) nor saved; then own up instead.
            mlog!("reply went round in circles after {} chars; cut back to {start}", reply.len());
            reply.truncate(start);
            let sorry = "Sorry, I'm not sure about that one.";
            if cfg.speak_replies && self.current(turn) {
                self.speaker.stop();
                self.speaker.say(sorry);
            }
            reply = format!("{} {sorry}", reply.trim());
            self.emit_reply(turn, "speaking", question, reply.trim());
        } else if cfg.speak_replies && self.current(turn) {
            if let Some(rest) = splitter.borrow_mut().finish() {
                say(&rest);
            }
        }
        let mut reply = llm::without_offers(&reply);
        let (done, steps) = result.as_ref().map(|t| (t.done.clone(), t.steps.clone())).unwrap_or_default();
        if reply.trim().is_empty() && !done.is_empty() && self.current(turn) {
            // Acted, but said nothing about it.
            reply = "I couldn't verify that the task was completed.".into();
            if cfg.speak_replies {
                self.speaker.say(&reply);
            }
            self.emit_reply(turn, "speaking", question, &reply);
        }
        // Interrupted workers must not append stale answers to a newer conversation.
        let total_ms = t1.elapsed().as_millis() as u64;
        if !reply.trim().is_empty() && self.current(turn) {
            self.remember(turn, question, reply.trim(), steps);
            if cfg.save_history {
                history::append(&history::Entry {
                    time: chrono::Local::now().to_rfc3339(),
                    question: question.to_string(),
                    answer: reply.trim().to_string(),
                    mode: mode.to_string(),
                    audio_seconds,
                    heard_ms,
                    first_word_ms: first_token_ms.unwrap_or_default() as u64,
                    total_ms,
                    model: self.brain.model_name(),
                    actions: done,
                });
                let _ = self.app.emit_to("main", "history-updated", ());
            }
        }
        result?;
        if !self.current(turn) {
            return Ok(Duration::ZERO);
        }
        if reply.trim().is_empty() {
            return Err(anyhow::anyhow!("The model gave an empty answer"));
        }
        mlog!(
            "answered: first words after {} ms, full reply in {} ms ({} chars)",
            first_token_ms.unwrap_or_default(),
            total_ms,
            reply.trim().len()
        );
        self.emit_reply(turn, "done", question, reply.trim());
        if !self.conversing() {
            self.ready_status();
        }
        if cfg.speak_replies {
            self.speaker.wait(|| self.current(turn));
            Ok(Duration::from_millis(1500))
        } else {
            // Long enough to read it.
            let ms = (reply.len() as u64 * 60).clamp(3000, 15000);
            Ok(Duration::from_millis(ms))
        }
    }

    /// A question typed in the app window: answered (and spoken) like a spoken one.
    pub fn ask_typed(self: &Arc<Self>, question: &str) -> anyhow::Result<()> {
        if question.is_empty() {
            return Ok(());
        }
        if let Some(why) = self.unavailable() {
            return Err(anyhow::anyhow!(why));
        }
        match self.phase() {
            Phase::Recording => return Err(anyhow::anyhow!("{} is listening right now", self.cfg().assistant_name)),
            Phase::Responding => self.interrupt(),
            Phase::Idle => {}
        }
        self.set_phase(Phase::Responding);
        let turn = self.turn.load(Ordering::SeqCst);
        let core = self.clone();
        let question = question.to_string();
        std::thread::spawn(move || {
            let result = core.answer(turn, &question, "typed", 0.0, 0);
            if core.finish_turn(turn) {
                if let Err(e) = result {
                    let _ = core.app.emit_to("main", "reply-error", e.to_string());
                    mlog!("{e}");
                }
            }
        });
        Ok(())
    }

    /// Stop button in the app window: ends a conversation or cuts off the current answer.
    pub fn stop_all(self: &Arc<Self>) {
        if self.conversing() {
            self.end_conversation("ended from the app window");
        } else {
            self.cancel_recording();
            self.interrupt();
            self.hide_widget();
        }
    }

    /// System prompt + recent conversation + the new question. Starts over after a quiet spell.
    fn messages_for(&self, cfg: &Config, question: &str) -> Vec<Message> {
        let mut conv = self.conversation.lock().unwrap();
        let forget = Duration::from_secs(cfg.forget_after_minutes as u64 * 60);
        if conv.last_at.is_some_and(|t| t.elapsed() > forget) && !conv.messages.is_empty() {
            mlog!("starting a new conversation after {} quiet minutes", cfg.forget_after_minutes);
            conv.messages.clear();
        }
        let skip = turns_start(&conv.messages, cfg.history_turns);
        let mut messages = vec![Message::system(system_prompt(cfg))];
        messages.extend(conv.messages[skip..].iter().cloned());
        messages.push(Message::user(with_time(question)));
        messages
    }

    /// Keeps the exchange for follow-ups, with the actions taken and what they found, so "open
    /// that" knows what "that" is.
    fn remember(&self, turn: u64, question: &str, reply: &str, steps: Vec<Message>) {
        let mut conv = self.conversation.lock().unwrap();
        if !self.current(turn) {
            return;
        }
        conv.messages.push(Message::user(question));
        conv.messages.extend(steps);
        conv.messages.push(Message::assistant(reply));
        conv.last_at = Some(Instant::now());
        let start = turns_start(&conv.messages, self.cfg().history_turns);
        conv.messages.drain(..start);
    }

    pub fn new_conversation(self: &Arc<Self>) {
        self.stop_all();
        *self.conversation.lock().unwrap() = Conversation::default();
        mlog!("new conversation");
    }

    /// Asks the AI to load its model in the background, so it's ready by the time the
    /// question is transcribed. Cheap when it's already loaded.
    pub fn warm_up_llm(self: &Arc<Self>) {
        if self.unavailable().is_some() {
            return;
        }
        {
            let mut last = self.last_warm_up.lock().unwrap();
            if last.is_some_and(|t| t.elapsed() < WARM_UP_EVERY) {
                return;
            }
            *last = Some(Instant::now());
        }
        let core = self.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let cfg = core.cfg();
            // Some chat templates refuse a conversation without a question in it.
            let prefix = [Message::system(system_prompt(&cfg)), Message::user("Hi")];
            match core.brain.warm_up(&prefix) {
                Ok(()) => mlog!("AI ready ({} ms)", started.elapsed().as_millis()),
                Err(e) => {
                    mlog!("AI: {e}");
                    core.set_status(&e.to_string());
                    // Try again on the next press.
                    *core.last_warm_up.lock().unwrap() = None;
                }
            }
            // Then the AI voice, if one is chosen, so the first sentence doesn't wait for it.
            if cfg.speak_replies {
                if let Err(e) = core.speaker.warm_up() {
                    mlog!("voice: {e}");
                }
            }
        });
    }

    fn fail(self: &Arc<Self>, message: &str) {
        mlog!("{message}");
        self.emit("error", Some(message.to_string()), None);
        self.show_widget();
        self.hide_widget_later(Duration::from_millis(4000));
    }

    /// Downloads the built-in AI (setup's assistant step, or the Assistant page). Safe to call
    /// repeatedly. Progress arrives as `download-progress` events.
    pub fn download_brain(self: &Arc<Self>) {
        let dir = config::data_dir();
        let brain_id = self.cfg().builtin_model;
        if model::brain_path(&dir, &brain_id).exists() || self.shared.downloading("brain") {
            return;
        }
        self.shared.download_progress("brain", Some(0.0));
        let core = self.clone();
        std::thread::spawn(move || {
            let result = model::ensure_brain(&config::data_dir(), &brain_id, |done, total| {
                core.shared.download_progress("brain", Some(if total > 0 { done as f32 / total as f32 } else { 0.0 }));
            });
            core.shared.download_progress("brain", None);
            match result {
                Ok(_) => {
                    mlog!("AI model downloaded");
                    *core.last_warm_up.lock().unwrap() = None;
                    core.warm_up_llm();
                    core.ready_status();
                }
                Err(e) => {
                    mlog!("AI model download failed: {e}");
                    let _ = core.app.emit_to("main", "download-error", format!("AI model: {e}"));
                }
            }
        });
    }

    /// Downloads the natural voice (Settings → Voice). Safe to call repeatedly. Progress arrives
    /// as `download-progress` events for "natural".
    pub fn download_voice(self: &Arc<Self>) {
        if kokoro::Kokoro::ready() || self.shared.downloading("natural") {
            return;
        }
        self.shared.download_progress("natural", Some(0.0));
        let core = self.clone();
        std::thread::spawn(move || {
            let result = model::ensure_natural(&config::data_dir(), |done, total| {
                core.shared.download_progress("natural", Some(if total > 0 { done as f32 / total as f32 } else { 0.0 }));
            });
            core.shared.download_progress("natural", None);
            match result {
                Ok(()) => {
                    mlog!("natural voice downloaded");
                    *core.last_warm_up.lock().unwrap() = None;
                    core.warm_up_llm();
                }
                Err(e) => {
                    mlog!("natural voice download failed: {e}");
                    let _ = core.app.emit_to("main", "download-error", format!("Natural voice: {e}"));
                }
            }
        });
    }

    /// Settings changed: point the AI and the voice at the new choices.
    pub fn configure(self: &Arc<Self>, old: &Config, new: &Config) {
        self.brain.configure(new);
        let voice_changed = old.voice_engine != new.voice_engine || old.natural_voice != new.natural_voice;
        if voice_changed || old.voice != new.voice || old.speech_rate != new.speech_rate {
            self.speaker.configure(new);
        }
        let brain_changed = old.llm_model != new.llm_model || old.brain != new.brain || old.builtin_model != new.builtin_model || old.ollama_url != new.ollama_url;
        if brain_changed || (new.assistant_enabled && !old.assistant_enabled) {
            *self.last_warm_up.lock().unwrap() = None;
            self.warm_up_llm();
        }
        if !new.assistant_enabled && old.assistant_enabled {
            self.stop_all();
            self.brain.local.stop();
            self.speaker.unload();
        }
    }
}

/// Timers, and the hidden browser pages are read in, for `actions`.
impl Assistant {
    /// Listens (with the open mic, no key needed) for a yes or no, while also taking the
    /// widget's buttons. Gives up after a few quiet seconds.
    fn hear_yes_no(&self, turn: u64, buttons: &std::sync::mpsc::Receiver<bool>) -> bool {
        std::thread::sleep(ECHO_GUARD);
        let cfg = self.cfg();
        let (said_tx, said_rx) = std::sync::mpsc::channel::<bool>();
        let vad = Arc::new(Mutex::new(Vad::new()));
        let app = self.app.clone();
        let sensitivity = cfg.speech_threshold;
        let level_vad = vad.clone();
        let said_tx = Mutex::new(Some(said_tx));
        let on_level = Box::new(move |level: f32| {
            let _ = app.emit_to("widget", "level", level);
            let decision = level_vad.lock().unwrap().step(Instant::now(), level, sensitivity, Duration::from_millis(900), Duration::from_secs(8));
            let heard = match decision {
                VadDecision::Continue => return,
                VadDecision::Send => true,
                VadDecision::GiveUp => false,
            };
            if let Some(tx) = said_tx.lock().unwrap().take() {
                let _ = tx.send(heard);
            }
        });
        self.ducker.duck_now();
        let listening = self.shared.recorder.start(cfg.input_device.clone(), on_level).is_ok();
        if !listening {
            self.ducker.restore();
        }
        let started = Instant::now();
        let heard = loop {
            if let Ok(yes) = buttons.try_recv() {
                if listening {
                    let _ = self.stop_mic();
                }
                return yes;
            }
            if !self.current(turn) || started.elapsed() > Duration::from_secs(15) {
                if listening {
                    let _ = self.stop_mic();
                }
                return false;
            }
            if let Ok(heard) = said_rx.try_recv() {
                break heard;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        if !listening {
            return false;
        }
        let Ok(rec) = self.stop_mic() else { return false };
        if !heard {
            return false;
        }
        self.emit("transcribing", None, None);
        let Ok(transcriber) = self.shared.ensure_transcriber() else { return false };
        let said = transcriber.transcribe(&rec.samples, &cfg.language, false, &[]).map(|t| cleanup::clean_question(&t)).unwrap_or_default();
        mlog!("heard for the confirmation: {said}");
        self.current(turn) && is_yes(&said)
    }
}

/// "Yes", "yeah, do it", "go ahead", "sure"… but not "no" or "yes, wait, no".
fn is_yes(said: &str) -> bool {
    static YES: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\b(yes|yeah|yep|yup|sure|ok|okay|go ahead|do it|confirm|please do|correct|absolutely|of course|affirmative)\b").unwrap()
    });
    static NO: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\b(no|nope|don'?t|do not|stop|cancel|wait|never mind|nevermind)\b").unwrap()
    });
    YES.is_match(said) && !NO.is_match(said)
}

impl actions::Host for Assistant {
    fn start_timer(&self, seconds: u64, label: String) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let ends = Instant::now() + Duration::from_secs(seconds);
        self.timers.lock().unwrap().push((id, label.clone(), ends));
        let core = self.app.state::<Arc<Assistant>>().inner().clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(seconds));
            let mut timers = core.timers.lock().unwrap();
            let Some(i) = timers.iter().position(|t| t.0 == id) else { return }; // cancelled
            timers.remove(i);
            drop(timers);
            core.timer_done(&label);
        });
    }

    fn timers(&self) -> Vec<(String, u64)> {
        let now = Instant::now();
        self.timers.lock().unwrap().iter().map(|(_, label, ends)| (label.clone(), ends.saturating_duration_since(now).as_secs())).collect()
    }

    fn cancel_timers(&self) -> usize {
        std::mem::take(&mut *self.timers.lock().unwrap()).len()
    }

    /// Says the question, shows Yes / No in the widget, and listens once for the answer. Silence,
    /// "no", a stop (Esc or the hotkey) or anything unclear counts as no.
    fn confirm(&self, question: &str) -> bool {
        let turn = self.turn.load(Ordering::SeqCst);
        let cfg = self.cfg();
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        *self.pending_confirm.lock().unwrap() = Some(tx);
        let payload = WidgetState { mode: "assistant", state: "confirm", message: Some(question.to_string()), progress: None, question: None, reply: None };
        let _ = self.app.emit_to("widget", "widget-state", &payload);
        let _ = self.app.emit_to("main", "confirm", question);
        self.show_widget();
        mlog!("asking: {question}");
        if cfg.speak_replies {
            self.speaker.say(question);
            self.speaker.wait(|| self.current(turn));
        }
        let answer = self.hear_yes_no(turn, &rx);
        *self.pending_confirm.lock().unwrap() = None;
        mlog!("answer: {}", if answer { "yes" } else { "no" });
        let _ = self.app.emit_to("main", "confirm", "");
        answer
    }

    /// Opens the page in a hidden window (Safari's engine, built into macOS) so its JavaScript
    /// runs, then reads its text. The page hands the text back by navigating to a made-up
    /// address, which is caught and cancelled here; the window never gets Murmur's own APIs.
    fn render_page(&self, url: &str) -> anyhow::Result<String> {
        const SCRIPT: &str = r#"
            if (window.top === window) {
              const send = () => {
                const text = (document.body && document.body.innerText) || "";
                location.href = "https://murmur.invalid/#" + encodeURIComponent(document.title + "\n" + text.slice(0, 30000));
              };
              addEventListener("load", () => setTimeout(send, 1500));
            }
        "#;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let tx = Mutex::new(Some(tx));
        let label = format!("page-{}", self.next_id.fetch_add(1, Ordering::SeqCst));
        let window = WebviewWindowBuilder::new(&self.app, &label, WebviewUrl::External(url.parse()?))
            .visible(false)
            .focused(false)
            .user_agent(actions::USER_AGENT)
            .initialization_script(SCRIPT)
            .on_navigation(move |to| {
                if to.host_str() != Some("murmur.invalid") {
                    return true;
                }
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(actions::percent_decode(to.fragment().unwrap_or_default()));
                }
                false
            })
            .build()?;
        let text = rx.recv_timeout(Duration::from_secs(20));
        let _ = window.destroy();
        text.map_err(|_| anyhow::anyhow!("The page didn't finish loading"))
    }
}

impl Assistant {
    /// A timer ran out: say so even when spoken answers are off, and post a notification.
    fn timer_done(self: &Arc<Self>, label: &str) {
        let text = if label.is_empty() { "Your timer is done.".to_string() } else { format!("Your {label} timer is done.") };
        mlog!("timer done: {label}");
        play_sound("Glass");
        let _ = std::process::Command::new("osascript")
            .args(["-e", "on run argv\ndisplay notification (item 1 of argv) with title (item 2 of argv) sound name \"Glass\"\nend run", &text, &self.cfg().assistant_name])
            .status();
        if self.phase() == Phase::Idle {
            let turn = self.turn.load(Ordering::SeqCst);
            self.show_widget();
            self.emit_reply(turn, "done", "Timer", &text);
            self.speaker.say(&text);
            self.hide_widget_later(Duration::from_secs(6));
        }
    }
}

pub fn system_prompt(cfg: &Config) -> String {
    let base = cfg.system_prompt.clone().unwrap_or_else(|| {
        format!(
            "You are {name}, a helpful voice assistant running on the user's Mac. \
             Everything you write is read aloud by a text-to-speech voice, so answer the way a \
             person would speak: usually one to three short sentences, more only when the user \
             asks for detail or steps. Never use markdown, bullet points, headings, code blocks, \
             tables, emojis or links. Write numbers, times and units the way you would say them. \
             Don't end with offers or questions like \"Would you like me to…\" or \"Let me know if…\"; \
             just stop once you've answered. \
             The user's words come from speech recognition and may contain mistakes, so work out \
             what they most likely meant. If you don't know something, or are unsure of a \
             word in a language you don't know well, say so in one short sentence; never guess, \
             and never correct yourself over and over.",
            name = cfg.assistant_name
        )
    });
    // No clock here: this part and the tool list stay the same from question to question, so
    // the AI can keep them read in advance. The time goes with each question instead.
    format!("{base}\n\n{}", actions::instructions(cfg))
}

/// Where the last `turns` exchanges begin, so a question is never kept without its actions
/// and answer.
fn turns_start(messages: &[Message], turns: usize) -> usize {
    let questions: Vec<usize> = messages.iter().enumerate().filter(|(_, m)| m.role == "user").map(|(i, _)| i).collect();
    questions.len().checked_sub(turns).and_then(|skip| questions.get(skip)).copied().unwrap_or(if turns == 0 { messages.len() } else { 0 })
}

/// The question with the current date and time, which the AI needs for "tomorrow", "in an hour"
/// and reminders.
fn with_time(question: &str) -> String {
    let now = chrono::Local::now().format("%A, %-d %B %Y, %-I:%M %p (%Y-%m-%d %H:%M)");
    format!("{question}\n\n[It is now {now}.]")
}

/// Headless check of the voice set in Settings: `murmur --say "It's raining."`
/// Speaks sentence by sentence, as an answer would be, and logs how long each step takes.
/// `MURMUR_NATURAL=af_heart` tries a natural voice whatever Settings say.
pub fn cli_say(text: &str) {
    let mut cfg = config::load();
    if let Ok(voice) = std::env::var("MURMUR_NATURAL") {
        cfg.voice_engine = "natural".into();
        cfg.natural_voice = voice;
    }
    let speaker = Speaker::new(&cfg);
    let started = Instant::now();
    let mut splitter = SentenceSplitter::default();
    let mut sentences = splitter.push(&llm::speakable(text));
    sentences.extend(splitter.finish());
    for s in &sentences {
        speaker.say(s);
    }
    speaker.wait(|| true);
    eprintln!("{} sentences, {} ms in all", sentences.len(), started.elapsed().as_millis());
    speaker.unload();
}

/// Headless check of the brain, actions and voice: `jarvis --ask "what's the weather in Pune?"`.
pub fn cli_ask(question: &str) -> anyhow::Result<()> {
    use std::io::Write;
    /// Timers and hidden pages need the running app; from the command line they're stand-ins.
    struct Cli;
    impl actions::Host for Cli {
        fn start_timer(&self, seconds: u64, label: String) {
            eprintln!("(would start a {seconds}-second timer: {label})");
        }
        fn timers(&self) -> Vec<(String, u64)> {
            vec![]
        }
        fn cancel_timers(&self) -> usize {
            0
        }
        fn render_page(&self, _url: &str) -> anyhow::Result<String> {
            Err(anyhow::anyhow!("no hidden browser from the command line"))
        }
        fn confirm(&self, question: &str) -> bool {
            if std::env::var_os("MURMUR_YES").is_some() {
                eprintln!("({question} → yes, MURMUR_YES is set)");
                return true;
            }
            eprint!("{question} [y/N] ");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            line.trim().eq_ignore_ascii_case("y") || line.trim().eq_ignore_ascii_case("yes")
        }
    }
    let cfg = config::load();
    let brain = Brain::new(&cfg);
    let speaker = Speaker::new(&cfg);
    let messages = vec![Message::system(system_prompt(&cfg)), Message::user(with_time(question))];
    let splitter = std::cell::RefCell::new(SentenceSplitter::default());
    let started = Instant::now();
    let mut first = None;
    let speak = cfg.speak_replies && std::env::var_os("MURMUR_QUIET").is_none();
    let done = actions::converse(
        &brain,
        &cfg,
        &Cli,
        messages,
        || true,
        |piece| {
            first.get_or_insert(started.elapsed().as_millis());
            print!("{piece}");
            let _ = std::io::stdout().flush();
            if speak {
                for s in splitter.borrow_mut().push(piece) {
                    speaker.say(&llm::speakable(&s));
                }
            }
            true
        },
        |what| {
            eprintln!("[{what}…] ({} ms)", started.elapsed().as_millis());
            true
        },
    )?;
    if let Some(rest) = splitter.borrow_mut().finish() {
        if speak {
            speaker.say(&llm::speakable(&rest));
        }
    }
    println!();
    eprintln!(
        "model: {}  actions: {}  first words: {} ms  full reply: {} ms",
        brain.model_name(),
        done.done.len(),
        first.unwrap_or_default(),
        started.elapsed().as_millis()
    );
    speaker.wait(|| true);
    brain.local.stop();
    speaker.unload();
    Ok(())
}
