# Murmur

Private, on-device voice dictation for macOS (Windows support in progress).
Hold **Fn**, speak, release: your words are typed into whatever app has focus.
Speech is transcribed locally with Whisper (whisper.cpp + Metal). Nothing leaves your machine.

See [PLAN.md](PLAN.md) for architecture and roadmap.

## Install (macOS)

```sh
pnpm install
pnpm tauri build --bundles app
cp -R src-tauri/target/release/bundle/macos/Murmur.app /Applications/
open /Applications/Murmur.app
```

First launch downloads the speech model (~550 MB, one time) to
`~/Library/Application Support/Murmur/models/`.

### One-time macOS setup
1. Grant Murmur these in **System Settings → Privacy & Security**:
   - **Accessibility** (to own the Fn key and to paste text)
   - **Microphone** (prompted the first time you dictate)
2. Quit and reopen Murmur after granting permissions.

With Accessibility granted, Murmur intercepts the Fn key itself, so macOS's own Fn action
(emoji picker, input switch, dictation) does not trigger and no keyboard settings need changing.
Without it, Murmur falls back to listening only (needs **Input Monitoring**); in that mode set
**System Settings → Keyboard → "Press 🌐 key to" → Do Nothing**.

> Rebuilding the app changes its ad-hoc signature, so macOS may ask for permissions again.
> If dictation stops working after a rebuild, remove Murmur from those lists and add it back.

## Use
- **Hold Fn** → speak → **release**: text is pasted into the focused app.
- **Double-tap Fn** → hands-free mode (lock icon) → speak as long as you like → **tap Fn** to finish.
- **Esc** cancels a recording; nothing is pasted.
- A quick single tap of Fn does nothing (no emoji picker, no widget).
- Pressing another key while holding Fn (e.g. Fn+F5, Fn+←) cancels, so normal Fn shortcuts still work.
- Your clipboard (text, images, files) is restored right after each paste.
- Menu bar mic icon: status, **Copy Last Dictation** (if a paste missed), history, settings,
  **Start at Login**, quit.

## The app window
Open it from the menu bar icon (**Open Murmur…**), or launch Murmur again from Spotlight/Finder.
- **Home** — status, missing-permission fixes, stats (words today, total, time saved, speed), recent dictations
- **History** — search, copy or delete any dictation; clear all
- **Settings** — everything below, saved automatically

## Settings
Changes apply immediately (a hotkey change restarts Murmur; a model change downloads/loads it in
the background). They are stored in `~/Library/Application Support/Murmur/config.json`.

| Setting | Default | Notes |
|---|---|---|
| Dictation key (`hotkey`) | `fn` | macOS: `fn`, `right_option`, `right_command`, `right_control`, `right_shift`. Windows: `right_ctrl`, `right_alt`, `caps_lock` |
| Model (`model`) | `large-v3-turbo-q5_0` | Also `large-v3-turbo`, `large-v3-q5_0`, `small`, `small.en`, `base.en`, `tiny.en` |
| Language (`language`) | `en` | `auto` or a code like `hi` |
| Translate to English (`translate`) | off | Needs a model with translate support (Large v3, Small multilingual) |
| Input device (`input_device`) | system default | Falls back to the default if the mic is disconnected |
| Remove filler words (`remove_fillers`) | on | um/uh, stutters, set-off "you know"/"I mean"; also writes times as `10:30` |
| Vocabulary (`vocabulary`) | — | Names/jargon to spell correctly |
| Replacements (`replacements`) | — | `[{"from": "acme corp", "to": "Acme Corp"}]`, whole words, any case |
| Sounds (`sounds`) | on | Soft start/stop tones |
| Restore clipboard (`restore_clipboard`) | on | Text, images and files are put back after pasting |
| Save history (`save_history`) | on | `history.jsonl`, local only |
| Free memory when idle (`unload_after_minutes`) | `0` (never) | Unloads the ~700 MB model; reloads as you start speaking |

## Development
The app window is React + Vite + Tailwind v4 + shadcn/ui (`src/`); charts use Recharts via
shadcn's chart components. The floating widget is a static page (`public/widget.html`).
The Rust core lives in `src-tauri/`.

```sh
pnpm tauri dev                     # run the app with hot reload
pnpm tauri build --bundles app     # build Murmur.app
pnpm ui:dev                        # preview the UI in a browser with mock data (no Tauri)
pnpm typecheck                     # TypeScript check
./src-tauri/target/release/murmur --transcribe clip.wav   # test the speech pipeline headlessly
```

Debug aids: `open --env MURMUR_TAB=insights /Applications/Murmur.app` opens straight onto a page;
`MURMUR_DEMO=1` cycles the widget states. When run from a terminal, macOS attributes permissions
to the terminal app instead of Murmur. Logs: `~/Library/Application Support/Murmur/murmur.log`.
