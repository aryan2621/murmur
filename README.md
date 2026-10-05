# Murmur

**Talk instead of type. Private voice dictation and a voice assistant for your Mac.**

<!-- DEMO VIDEO: paste the https://github.com/user-attachments/assets/... link on the next line -->

- **Dictation:** hold **Fn**, speak, let go. Your words are typed wherever your cursor is.
- **Assistant (Jarvis):** hold **Right Option** and ask. It answers out loud, or does it for you:
  searches the web, reads your screen, opens apps, sets timers, plays music and more.

Everything runs on your Mac (Whisper for speech, a built-in Qwen3 AI, the macOS voice). Your
voice and conversations never leave it; with web access on, only search words and page
addresses go online.

**[⬇ Download for macOS](https://github.com/aryan2621/desktop-apps/releases/latest/download/Murmur_0.1.0_aarch64.dmg)**
· macOS 11+, Apple silicon recommended · [Windows beta and all downloads](https://github.com/aryan2621/desktop-apps/releases/latest)

📖 **[User guide](docs/user.md)** — every feature, setting and fix
🛠 **[Developer guide](docs/dev.md)** — build from source, code layout, how the assistant works

## Install

1. Open the `.dmg` and drag **Murmur** into **Applications**, then open it.
2. If macOS says **"Apple could not verify Murmur is free of malware"**: click **Done**, go to
   **System Settings → Privacy & Security**, scroll down and click **Open Anyway**. (Murmur isn't
   signed with a paid Apple certificate; that's the only reason for the warning.)
3. If it says Murmur **"is damaged"**, run this once in Terminal and open it again:
   ```bash
   xattr -dr com.apple.quarantine /Applications/Murmur.app
   ```
4. The setup asks for **Accessibility** and **Microphone**, downloads the speech model
   (~550 MB, once), and optionally the assistant's AI (4.8 GB, or 2.4 GB for smaller Macs).

## Quick start

| Do this | What happens |
|---|---|
| **Hold Fn**, speak, release | Your words are typed into the app you're in |
| **Double-tap Fn** | Hands-free dictation; tap Fn to finish |
| **Hold Right Option**, ask, release | Jarvis answers one question |
| **Tap Right Option** | A conversation: talk, pause, it answers and listens again |
| **Esc** | Cancel or stop, any time |

Try: *"What's the weather tomorrow?"*, *"Play the Kesariya song"*, *"What's on my screen?"*,
*"10 minute pasta timer"*, *"Open my resume"*. More in the [user guide](docs/user.md#what-the-assistant-can-do).

## Build from source

```bash
cd murmur && pnpm install && pnpm tauri build --bundles app
```

Needs Node, pnpm, Rust and `cmake`. Details in the [developer guide](docs/dev.md).
