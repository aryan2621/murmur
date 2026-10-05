# Murmur: developer guide

[← Back to README](../README.md) · [User guide](user.md)

## Build and install

Needs Node 22, pnpm, Rust (stable) and `cmake` (for llama.cpp's server).

```bash
cd murmur
pnpm install
pnpm tauri build --bundles app
cp -R src-tauri/target/release/bundle/macos/Murmur.app /Applications/
open /Applications/Murmur.app
```

`pnpm tauri build` also builds llama.cpp's server for the assistant
(`scripts/build-llama-server.sh`). Rebuilding changes the app's ad-hoc signature, so macOS may
ask for Accessibility and Microphone again: remove Murmur from those lists and add it back.
When run from a terminal (`pnpm tauri dev`), macOS gives the permissions to the terminal app
instead of Murmur.

CI (`.github/workflows/build.yml`) builds the macOS `.dmg` and the Windows installer on every
push; tagging `v*` publishes a release.

## Code layout

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

## Commands

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

## How the assistant works

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
