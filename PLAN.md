# Murmur — private, local voice dictation

Hold a key, speak, release → clean text is typed into whatever app is focused.
Everything runs on-device. No audio or text ever leaves the machine.

## Goals
- 100% offline after a one-time model download
- Hold-to-talk: **Fn** on macOS, **Right Ctrl** on Windows
- Floating widget with live waveform while recording
- < ~1.5 s from key release to pasted text on Apple Silicon
- One codebase for macOS + Windows (Tauri v2)

## Architecture

```
 ┌────────────┐  press/release  ┌────────────┐  16 kHz mono  ┌──────────────┐
 │ hotkey.rs  │ ──────────────▶ │  audio.rs  │ ────────────▶ │transcribe.rs │
 │ Fn / RCtrl │                 │   (cpal)   │               │ (whisper.cpp)│
 └────────────┘                 └─────┬──────┘               └──────┬───────┘
                                      │ mic level                    │ raw text
                                      ▼                              ▼
                               ┌────────────┐                ┌──────────────┐
                               │ widget UI  │                │  cleanup.rs  │
                               │ (webview)  │                │ fillers/rules│
                               └────────────┘                └──────┬───────┘
                                                                     ▼
                                                  ┌──────────────┐  ┌──────────────┐
                                                  │  paste.rs    │  │ history.rs   │
                                                  │ clipboard+⌘V │  │ local JSONL  │
                                                  └──────────────┘  └──────────────┘
```

| Module | Responsibility | Mac | Windows |
|---|---|---|---|
| `hotkey.rs` | Global hold-to-talk key | CGEventTap, Fn flag | Low-level hook (rdev), Right Ctrl |
| `audio.rs` | Mic capture, mono + resample to 16 kHz, RMS level | cpal (CoreAudio) | cpal (WASAPI) |
| `transcribe.rs` | Load model once, run Whisper | whisper-rs + Metal GPU | whisper-rs (CPU) |
| `model.rs` | First-run model download with progress | shared | shared |
| `cleanup.rs` | Remove "um/uh", stutters, fix spacing/capitalisation | shared | shared |
| `paste.rs` | Clipboard + simulated paste, restore old clipboard | ⌘V | Ctrl+V |
| `history.rs` | Append every dictation to a local file | shared | shared |
| `lib.rs` | App state machine, tray menu, widget window | shared | shared |

**State machine:** `Idle → Recording → Transcribing → Idle` (key presses ignored while transcribing).

**Model:** `ggml-large-v3-turbo-q5_0` (~550 MB): near large-v3 accuracy, fast on Metal.
Smaller `base`/`small` models can be selected for CPU-only Windows laptops.

## Phases

### Phase 1 — Core ✅
Tray app, Fn hold detection, mic capture, floating non-focus-stealing widget (NSPanel, works over
full-screen apps), model auto-download, local Whisper (Metal), rule-based cleanup, paste + clipboard
restore, local history, stable code signing, file logging.

### Phase 1.5 — Core polish ✅
- Fn tap no longer opens the emoji picker (swallow the synthetic Globe key, keycode 179)
- Flowing-wave transcribing animation, compact ✓ on done
- Widget only appears for holds (no flash on taps); soft start/stop sounds
- Double-tap Fn = hands-free mode; Esc cancels
- Full clipboard preservation (images, files, rich text) restored in the background
- Tray: Copy Last Dictation, Start at Login

### Phase 2a — App window & settings ✅
- Main window (Home / History / Settings) from the tray or by relaunching; Dock icon while open
- Home: status, permission checks with fix buttons, stats, recent dictations
- History: search, copy, delete, clear
- Settings applied live: hotkey (auto-restart), model (background download/load), language,
  translate to English, microphone choice, fillers, vocabulary, replacements, sounds, clipboard,
  start at login, history, free memory when idle
- Cleanup v2: set-off "you know"/"I mean", spoken times → `10:30`, user replacements

### Phase 2b — Smart features (needs a local LLM; deferred)
- **Voice edit:** select text, hold Fn, say "make this formal" → rewritten in place
- **App-aware style:** casual in Slack, formal in Mail, code-aware in VS Code/Terminal
- Optional LLM polish of dictations; learned personal dictionary; voice snippets
- Live streaming preview

### Phase 3 — Windows, packaging, quality
- Compile and test on Windows, GPU (CUDA/Vulkan) option, installer, autostart
- Developer ID signing + notarization for sharing
- CI build for macOS and Windows

## macOS setup (one time)
- Grant **Microphone** and **Accessibility** permission when prompted
- Murmur swallows Fn events (and the synthetic Globe key 179 macOS sends after a bare Fn tap)
  with an active HID event tap, so the emoji picker never opens. It also sets
  "Press 🌐 key to" → Do Nothing at startup as a fallback and restores it if the hotkey changes.

## Privacy
- No network calls except the one-time model download from Hugging Face
- History stored only at the app data dir (`~/Library/Application Support/Murmur/`)
