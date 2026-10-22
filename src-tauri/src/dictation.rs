//! Dictation: hold the key, speak, release, and the words are typed where the cursor is.
//! Double-tap for hands-free.

use crate::gesture::{self, ReleaseAction};
use crate::hotkey::HotkeyEvent;
use crate::{cleanup, focus, history, paste, snippets, Mode, Shared, MIN_SECONDS, SILENCE_RMS};
use crate::audio::TARGET_RATE;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Idle,
    Recording,
    Transcribing,
}

#[derive(Serialize, Clone)]
struct WidgetState<'a> {
    mode: &'static str,
    state: &'a str,
    message: Option<String>,
    progress: Option<f32>,
}

/// Hotkey timing state used to tell holds, taps and double-taps apart.
#[derive(Default)]
struct Gesture {
    pressed_at: Option<Instant>,
    last_tap_end: Option<Instant>,
    gap_before_press: Option<Duration>,
    /// Hands-free recording: runs until the next press.
    locked: bool,
    /// The press that stopped a hands-free recording; its release is not a gesture.
    ignore_release: bool,
}

#[derive(Serialize, Clone)]
struct PartialTranscript {
    text: String,
}

pub struct Dictation {
    app: AppHandle,
    shared: Arc<Shared>,
    phase: Mutex<Phase>,
    gesture: Mutex<Gesture>,
    pub last_text: Mutex<Option<String>>,
    /// Bumped when a recording starts; partial workers exit when this changes.
    recording_id: AtomicU64,
}

impl Dictation {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            app: shared.app.clone(),
            shared,
            phase: Mutex::new(Phase::Idle),
            gesture: Mutex::new(Gesture::default()),
            last_text: Mutex::new(None),
            recording_id: AtomicU64::new(0),
        }
    }

    pub fn phase(&self) -> Phase {
        *self.phase.lock().unwrap()
    }

    fn set_phase(&self, p: Phase) {
        *self.phase.lock().unwrap() = p;
    }

    /// Murmur's app window is open and has keyboard focus, so its page can take dictation.
    fn main_window_focused(&self) -> bool {
        self.app
            .get_webview_window("main")
            .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
    }

    fn emit(&self, state: &str, message: Option<String>, progress: Option<f32>) {
        let _ = self.app.emit_to("widget", "widget-state", WidgetState { mode: "dictation", state, message, progress });
    }

    fn show_widget(&self) {
        let before = focus::frontmost_bundle();
        self.shared.show_widget(Mode::Dictation);
        // Diagnostics: showing the widget must never change which app (or Space) is in front.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let after = focus::frontmost_bundle();
            if after != before {
                mlog!("widget: front app changed from {before} to {after} while showing");
            }
        });
    }

    /// Hides after `delay`, unless a new recording started (or the widget was shown again) in
    /// the meantime.
    fn hide_widget_later(self: &Arc<Self>, delay: Duration) {
        let core = self.clone();
        let shown = self.shared.widget_shown();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if core.phase() == Phase::Idle {
                core.shared.hide_widget_since(shown);
            }
        });
    }

    /// Returns true when the event was consumed (only matters for Esc).
    pub fn on_hotkey(self: &Arc<Self>, event: HotkeyEvent) -> bool {
        let now = Instant::now();
        match event {
            HotkeyEvent::Pressed => {
                let mut g = self.gesture.lock().unwrap();
                if self.phase() == Phase::Recording && g.locked {
                    g.locked = false;
                    g.ignore_release = true;
                    drop(g);
                    mlog!("hands-free stopped");
                    self.finish_recording();
                } else if self.shared.loading() {
                    drop(g);
                    self.emit("error", Some("Model still loading…".into()), None);
                    self.show_widget();
                    self.hide_widget_later(Duration::from_millis(1500));
                } else if self.phase() == Phase::Idle {
                    g.pressed_at = Some(now);
                    g.gap_before_press = g.last_tap_end.map(|t| now - t);
                    drop(g);
                    self.start_recording();
                }
                false
            }
            HotkeyEvent::Released => {
                let mut g = self.gesture.lock().unwrap();
                if std::mem::take(&mut g.ignore_release) || g.locked || self.phase() != Phase::Recording {
                    return false;
                }
                let held = g.pressed_at.map(|t| now - t).unwrap_or_default();
                let action = gesture::on_release(held, g.gap_before_press);
                mlog!("hotkey released after {} ms → {action:?}", held.as_millis());
                match action {
                    ReleaseAction::Finish => {
                        g.last_tap_end = None;
                        drop(g);
                        self.finish_recording();
                    }
                    ReleaseAction::DiscardTap => {
                        g.last_tap_end = Some(now);
                        drop(g);
                        self.cancel_recording();
                    }
                    ReleaseAction::LockHandsFree => {
                        g.last_tap_end = None;
                        g.locked = true;
                        drop(g);
                        self.emit("locked", None, None);
                        self.show_widget();
                        self.shared.play("Tink");
                    }
                }
                false
            }
            HotkeyEvent::Cancelled => {
                if !self.gesture.lock().unwrap().locked {
                    self.cancel_recording();
                }
                false
            }
            HotkeyEvent::Escape => {
                if self.phase() != Phase::Recording {
                    return false;
                }
                mlog!("cancelled with Esc");
                self.gesture.lock().unwrap().locked = false;
                self.cancel_recording();
                true
            }
        }
    }

    /// Opens the mic for this press. Returns at once (the hotkey's thread must stay free to see
    /// the release when it happens); the rest runs once the mic is open.
    fn start_recording(self: &Arc<Self>) {
        self.set_phase(Phase::Recording);
        let app = self.app.clone();
        let on_level = Box::new(move |level: f32| {
            let _ = app.emit_to("widget", "level", level);
        });
        let cfg = self.shared.cfg();
        let opened = self.shared.recorder.start_async(cfg.input_device.clone(), cfg.mic_processing(), on_level);
        let core = self.clone();
        std::thread::spawn(move || match opened.recv().unwrap_or_else(|e| Err(anyhow::anyhow!("{e}"))) {
            Ok(()) => core.recording_started(),
            Err(e) => {
                core.set_phase(Phase::Idle);
                core.fail(&format!("Mic error: {e}"));
            }
        });
    }

    fn recording_started(self: &Arc<Self>) {
        if self.phase() != Phase::Recording {
            // Released and handled while the mic was still opening.
            return;
        }
        self.emit("recording", None, None);
        let _ = self.app.emit_to("widget", "partial-transcript", PartialTranscript { text: String::new() });
        self.shared.preload_transcriber();
        self.spawn_partial_worker();
        // Show only if this turns out to be a hold, so quick taps never flash the widget.
        let core = self.clone();
        let pressed_at = self.gesture.lock().unwrap().pressed_at;
        std::thread::spawn(move || {
            std::thread::sleep(gesture::SHOW_DELAY);
            let g = core.gesture.lock().unwrap();
            let same_press = g.pressed_at == pressed_at;
            let locked = g.locked;
            drop(g);
            if core.phase() == Phase::Recording && same_press && !locked {
                core.show_widget();
                core.shared.play("Tink");
            }
        });
    }

    fn spawn_partial_worker(self: &Arc<Self>) {
        let id = self.recording_id.fetch_add(1, Ordering::SeqCst) + 1;
        let core = self.clone();
        std::thread::spawn(move || core.partial_loop(id));
    }

    fn partial_loop(self: &Arc<Self>, id: u64) {
        let min_samples = TARGET_RATE as usize * 6 / 5;
        let mut last_hint = String::new();
        loop {
            let cfg = self.shared.cfg();
            if !cfg.live_partials {
                std::thread::sleep(Duration::from_millis(cfg.partial_interval_ms.max(400) as u64));
                if self.recording_id.load(Ordering::SeqCst) != id || self.phase() != Phase::Recording {
                    break;
                }
                continue;
            }
            std::thread::sleep(Duration::from_millis(cfg.partial_interval_ms.max(400) as u64));
            if self.recording_id.load(Ordering::SeqCst) != id || self.phase() != Phase::Recording {
                break;
            }
            let Some(tail) = self.shared.recorder.snapshot() else { break };
            if tail.samples.len() < min_samples {
                continue;
            }
            let lang = cfg.language.as_str();
            let hint = if last_hint.is_empty() { None } else { Some(last_hint.as_str()) };
            let Ok(raw) = self.shared.transcribe_partial(&tail.samples, lang, cfg.translate, &cfg.vocabulary, hint)
            else {
                continue;
            };
            if raw.is_empty() || cleanup::is_hallucination(&raw) {
                continue;
            }
            let text = cleanup::clean_for_style(&raw, &cfg.dictation_style, cfg.remove_fillers);
            if text.is_empty() {
                continue;
            }
            last_hint = raw;
            let _ = self.app.emit_to("widget", "partial-transcript", PartialTranscript { text });
        }
    }

    pub fn cancel_recording(self: &Arc<Self>) {
        if self.phase() != Phase::Recording {
            return;
        }
        let _ = self.shared.recorder.stop();
        self.set_phase(Phase::Idle);
        self.hide_widget_later(Duration::ZERO);
    }

    fn finish_recording(self: &Arc<Self>) {
        if self.phase() != Phase::Recording {
            return;
        }
        self.set_phase(Phase::Transcribing);
        self.shared.play("Pop");
        let core = self.clone();
        std::thread::spawn(move || {
            let result = core.process();
            core.set_phase(Phase::Idle);
            match result {
                Ok(hide_after) => core.hide_widget_later(hide_after),
                Err(e) => core.fail(&e.to_string()),
            }
        });
    }

    /// Records → text → inserted where it makes sense. Returns how long to keep the widget up.
    fn process(&self) -> anyhow::Result<Duration> {
        let quick = Duration::from_millis(350);
        let mut rec = self.shared.recorder.stop()?;
        if rec.seconds() < MIN_SECONDS || rec.peak_rms < SILENCE_RMS {
            mlog!("skipped clip ({:.2}s, peak rms {:.4})", rec.seconds(), rec.peak_rms);
            return Ok(Duration::ZERO);
        }
        if self.shared.cfg().noise_suppression && !crate::voice::keep_speech(&mut rec.samples) {
            mlog!("skipped clip ({:.2}s): no speech in it", rec.seconds());
            return Ok(Duration::ZERO);
        }
        self.emit("transcribing", None, None);
        let _ = self.app.emit_to("widget", "partial-transcript", PartialTranscript { text: String::new() });
        let started = Instant::now();
        let cfg = self.shared.cfg();
        let raw = self.shared.transcribe(&rec.samples, &cfg.language, cfg.translate, &cfg.vocabulary)?;
        let elapsed = started.elapsed().as_millis();
        mlog!("transcribed {:.1}s audio in {elapsed} ms", rec.seconds());
        if cleanup::is_hallucination(&raw) {
            return Ok(Duration::ZERO);
        }
        let mut text = cleanup::apply_replacements(
            &cleanup::clean_for_style(&raw, &cfg.dictation_style, cfg.remove_fillers),
            &cfg.replacements,
        );
        text = snippets::apply_snippets(&text, &cfg.snippets);
        if text.is_empty() {
            return Ok(Duration::ZERO);
        }
        let copied = Duration::from_millis(2200);
        let hide_after = match focus::detect() {
            // Murmur's own window: the page puts it in the focused field or the scratchpad.
            focus::Target::Murmur if self.main_window_focused() => {
                let _ = self.app.emit_to("main", "dictation", &text);
                self.emit("done", None, None);
                quick
            }
            // Murmur is in front with no window of its own (a menu-bar app stays in front after
            // its window closes), or nothing can take text: keep it on the clipboard and say so
            // instead of a false ✓.
            focus::Target::Murmur | focus::Target::NotEditable { .. } => {
                paste::copy(&text)?;
                self.emit("copied", None, None);
                copied
            }
            // A text field: type it there (trailing space so dictations flow together).
            focus::Target::Editable => {
                paste::insert(&format!("{text} "), cfg.restore_clipboard)?;
                self.emit("done", None, None);
                quick
            }
            // Can't tell if anything will take the text: paste anyway, but keep it on the
            // clipboard so it is never lost.
            focus::Target::Unknown => {
                paste::insert(&format!("{text} "), false)?;
                self.emit("pasted", None, None);
                copied
            }
        };
        *self.last_text.lock().unwrap() = Some(text.clone());
        self.shared.touch();
        if cfg.save_history {
            history::append(&text, &raw, rec.seconds(), elapsed);
            let _ = self.app.emit_to("main", "history-updated", ());
        }
        Ok(hide_after)
    }

    fn fail(self: &Arc<Self>, message: &str) {
        mlog!("{message}");
        self.emit("error", Some(message.to_string()), None);
        self.show_widget();
        self.hide_widget_later(Duration::from_millis(2500));
    }

    /// Debug aid: `MURMUR_DEMO=1` cycles the widget through recording → transcribing → done.
    pub fn demo(&self, delay: u64) {
        std::thread::sleep(Duration::from_secs(delay));
        self.emit("recording", None, None);
        self.show_widget();
        for i in 0..90 {
            let _ = self.app.emit_to("widget", "level", 0.02 + 0.05 * ((i as f32) * 0.4).sin().abs());
            std::thread::sleep(Duration::from_millis(33));
        }
        self.emit("transcribing", None, None);
        std::thread::sleep(Duration::from_secs(3));
        self.emit("done", None, None);
        std::thread::sleep(Duration::from_millis(1500));
        self.shared.hide_widget();
    }
}
