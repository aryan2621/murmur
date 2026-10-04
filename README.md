# Murmur

Private, on-device voice dictation and voice assistant for macOS (Windows support, dictation
only, in progress).
- **Dictation:** hold **Fn**, speak, release: your words are typed into whatever app has focus.
- **Assistant** (called Jarvis by default): hold **Right Option**, ask something, release: it
  answers out loud, or does it. Tap the key for a hands-free conversation.

Speech is transcribed locally with Whisper (whisper.cpp + Metal), shared by both. The assistant
thinks with a built-in AI (Qwen3 8B on llama.cpp, Metal) and speaks with the macOS voice. Your
voice and conversation never leave the Mac; with web access on, only search words and page
addresses do.



## Install (macOS)

```sh
pnpm install
pnpm tauri build --bundles app
cp -R src-tauri/target/release/bundle/macos/Murmur.app /Applications/
open /Applications/Murmur.app
```

`pnpm tauri build` also builds llama.cpp's server for the assistant (needs `cmake`).

First launch opens a short setup: allow Accessibility and Microphone, download the speech model
(~550 MB by default, one time, to `~/Library/Application Support/Murmur/models/`), and try a first
dictation. Then, optionally, set up the assistant: download its AI (4.8 GB, or skip and do it
later from its page), pick a voice and ask a first question. Run setup again any time from
**Settings → About → Run setup again**.

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

## Use: dictation
- **Hold Fn** → speak → **release**: text is pasted into the focused app.
- **Double-tap Fn** → hands-free mode (lock icon) → speak as long as you like → **tap Fn** to finish.
- **Esc** cancels a recording; nothing is pasted.
- A quick single tap of Fn does nothing (no emoji picker, no widget).
- Pressing another key while holding Fn (e.g. Fn+F5, Fn+←) cancels, so normal Fn shortcuts still work.
- Your clipboard (text, images, files) is restored right after each paste.
- Menu bar mic icon: status, **Copy Last Dictation** (if a paste missed), **New Conversation**,
  history, settings, **Open Log**, **Start at Login**, quit.

## Use: the assistant
**Quick question:** hold **Right Option**, ask, release. You get one answer and it's done.

**Conversation:** tap **Right Option** once. Talk; when you pause (~1.2 s) it answers, then
listens again by itself for your follow-up.
- **Tap** while it's talking → it stops and listens to you.
- **Tap** while it's listening → the conversation ends. So does **Esc**, or ~20 s of silence.
- The mic is off while it speaks, so it never hears (and answers) its own voice.

Anywhere: **Esc** stops an answer. The last few exchanges are remembered for 5 minutes, so
follow-ups work in either mode; **New Conversation** in the menu bar starts over.

Only one of the two uses the mic at a time. Pressing **Fn** while the assistant is talking or
waiting in a conversation stops it and starts a dictation; pressing the assistant's key while
dictating does nothing. Turn the assistant off in Settings if you only want dictation: its key
is then left alone and no AI is loaded.

## What the assistant can do
Ask in your own words; the AI picks the action (tool calling) and confirms in a few words.

| | Examples |
|---|---|
| **Web** | "Who won the last F1 race?", "What's Bitcoin at?", "Read this article and summarise it" |
| **Weather** | "Will it rain tomorrow?", "Weather in London" (set **Your city** in Settings) |
| **Sums** | "10 tomatoes at 40 rupees each, plus 100 for detergent and 100 for milk: what's the total?", "What's 18% of 2340?" |
| **Browser** | "Play the Kesariya song" (top YouTube result, starts by itself), "Open github.com", "Open Amazon and search for umbrella", "New tab", "Which tabs are open?", "Switch to the LinkedIn tab", "Close this tab", "Go back" |
| **Apps** | "Open Slack", "Quit Spotify", "What apps are open?" |
| **Screen** | "What's on my screen?", "Read this to me", "What does this error say?" (reads the text in the window in front, in any app, on the Mac; needs Screen Recording permission once) |
| **Files** | "Find files named invoice", "Open my resume", "Open the latest download", "What's on my Desktop?" |
| **Mac** | "Volume to 30", "Turn it down", "Dark mode", "Lock the screen", "Take a screenshot", "How much battery?" |
| **Music** | "Pause", "Next song" (Spotify or Music) |
| **Timers** | "10 minute pasta timer", "How long is left?", "Cancel the timer" |
| **Reminders** | "Remind me to call Mum tomorrow at 6" |
| **Calendar** | "What's on today?", "Anything on Friday?" (includes repeating events) |

The assistant deliberately doesn't click or type inside apps or web pages, and doesn't move, rename or
delete files: a small local model gets those multi-step tasks wrong too often. It says so when asked.

How it works (`src-tauri/src/assistant/actions/`): every question takes three steps.
1. **Decide** (`mod.rs`): the AI picks one action from a fixed list (`chat`, `calculate`,
   `web_search`, `play_youtube`, `open_website`, `open_app`, `list_folder`, `timer`…). Its answer is
   JSON that llama.cpp forces, token by token, to match that list's schema, so it can't skip a
   field, garble the format, or talk instead of choosing.
2. **Do**: plain code runs that one action.
3. **Say**: actions that just do something get a fixed confirmation ("Opening Slack.",
   "Volume is now 30%."). Actions that find something out (web search, a folder's contents, the
   calendar, a sum) and plain conversation get a short reply written by the AI from the result,
   streamed so speech starts at once. Closing offers ("Let me know if…") are dropped.

The decision and a short form of its result stay in the conversation, so "open that in Brave"
knows what "that" is. Sums are never worked out by the AI: it writes the expression
(`10*40 + 100`, `18% * 2340`) and `calculate.rs` evaluates it.
- **Web search** (`web.rs`) reads Brave's results (DuckDuckGo if Brave refuses) and the top two
  pages at once, so most answers take one round. Results are kept for 10 minutes. Pages that need
  JavaScript open in a hidden WebKit window. Weather comes from wttr.in. No account or key.
- **Browser** (`browser.rs`) drives the browser you use, so you stay signed in, with AppleScript
  for tabs. "Play …" finds the top video on YouTube's results page and opens it, where it plays
  by itself. Common sites can be named without their address ("youtube", "gmail"), and a site can
  be searched directly (YouTube, Amazon, GitHub, Wikipedia…). Firefox gets keyboard shortcuts.

**Asking first:** opening an archive, installer or script (`.zip`, `.dmg`, `.pkg`, `.sh`…) is
asked about out loud. Say yes or no (or use the widget's buttons); silence or a stop counts as no.

The first time an action needs another app, macOS asks once (Automation, Calendars).
**Act on this Mac** and **Look things up online** can each be turned off in Settings → Actions & web.

**Models:** the built-in AI is Qwen3 8B (4.8 GB download, run without its "thinking" so answers
start at once). Settings → Assistant → Model can switch to the lighter Qwen3 4B (2.4 GB) on Macs with
less than 16 GB of memory.


## The app window
Open it from the menu bar icon (**Open Murmur…**), or launch Murmur again from Spotlight/Finder.
- **Home** — status, missing-permission fixes, stats (words today, streak, time saved), a scratchpad, recent dictations
- **Assistant** — today's questions, response time, a box to **type a question**, and recent
  answers. Live answers stream in here too.
- **Insights** — dictation (words per day, speed, busiest hours) or the assistant (questions per day, where response time goes)
- **History** — every dictation, or every question and answer: search, copy, delete
- **Settings** — everything below, saved automatically (voice picker has a ▶ preview)

## Settings
Changes apply immediately (a key change restarts Murmur; a model change downloads/loads it in
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
| Free memory when idle (`unload_after_minutes`) | `0` (never) | Unloads the ~700 MB speech model; reloads as you start speaking |

Assistant (macOS):

| Setting | Default | Notes |
|---|---|---|
| `assistant_enabled` | on | Off = its key does nothing and no AI is loaded |
| `assistant_hotkey` | `right_option` | `right_command`, `right_control`, `right_shift`, `fn`; must differ from the dictation key |
| `assistant_name` | `Jarvis` | What it calls itself |
| `brain` | `builtin` | `builtin` (bundled AI) or `ollama` to use your own Ollama instead |
| `builtin_model` | `8b` | Built-in model: `8b`, or `4b` for a lighter one |
| `llm_model` | `qwen3:8b` | With `brain: ollama`: any model from `ollama list` |
| `keep_alive` | `30m` | How long the AI stays in memory after a question (`-1` = always) |
| `voice` | `Daniel` | Any name from `say -v '?'`. Premium voices sound much better: System Settings → Accessibility → Spoken Content → Manage Voices |
| `speech_rate` | `195` | Words per minute |
| `pause_seconds` | `1.2` | Conversation mode: silence that sends what you said. Raise it if it cuts you off mid-thought |
| `conversation_timeout_seconds` | `20` | Conversation ends after this long without hearing you |
| `speech_threshold` | `0.012` | Mic level that counts as speech. Raise in a noisy room, lower if it misses a quiet voice |
| `speak_replies` | on | Off = answers only appear on screen |
| `system_prompt` | built-in | Your own personality / instructions |
| `forget_after_minutes` | `5` | Fresh conversation after this long without a question |
| `actions` | on | Act on the Mac (apps, browser, timers, reminders, calendar, volume, files…) |
| `web_access` | on | Web search, reading pages, weather. Off = nothing leaves the Mac |
| `location` | empty | City for the weather; empty = guessed from your connection |

History is kept in `history.jsonl` (dictations) and `assistant-history.jsonl` (questions and answers).

## Development
The app window is React + Vite + Tailwind v4 + shadcn/ui (`src/`, with the assistant's pages in
`src/assistant/`), styled after Claude (warm paper tones, a clay accent, Inter and Source Serif);
charts use Recharts via shadcn's chart components. The floating widget is a static page
(`public/widget.html`) shared by both modes.

The Rust core lives in `src-tauri/src/`: `lib.rs` holds what both modes share (settings, the mic,
the speech model, one key tap for both keys, the widget, the menu bar), `dictation.rs` the
dictation flow, and `assistant/` the assistant: `brain.rs` (runs the bundled llama.cpp server, or
Ollama), `llm.rs` (streaming, sentence splitting), `actions/` (the tools) and `speech.rs`
(interruptible `say` queue). `scripts/build-llama-server.sh` builds the server as one static
binary that Tauri bundles as a sidecar (macOS only, via `tauri.macos.conf.json`).

```sh
pnpm tauri dev                     # run the app with hot reload
pnpm tauri build --bundles app     # build Murmur.app
pnpm ui:dev                        # preview the UI in a browser with mock data (no Tauri)
pnpm typecheck                     # TypeScript check
./src-tauri/target/release/murmur --transcribe clip.wav   # test the speech pipeline headlessly
./src-tauri/target/release/murmur --ask "what's the weather in Pune?"   # test the AI, actions + voice headlessly
MURMUR_QUIET=1 ./src-tauri/target/release/murmur --ask "…"               # same, without speaking
MURMUR_DEBUG=1 MURMUR_QUIET=1 ./src-tauri/target/release/murmur --ask "…" # also print each tool result
MURMUR_YES=1 …                                                           # answer yes to confirmations
open -n --env MURMUR_ASK="set a 1 minute timer" /Applications/Murmur.app # ask inside the app
```

Debug aids: `open --env MURMUR_TAB=insights /Applications/Murmur.app` opens straight onto a page;
`MURMUR_DEMO=1` cycles the widget states. When run from a terminal, macOS attributes permissions
to the terminal app instead of Murmur. Logs: `~/Library/Application Support/Murmur/murmur.log`
(menu bar → **Open Log**); it records how long each answer took to start and finish.
