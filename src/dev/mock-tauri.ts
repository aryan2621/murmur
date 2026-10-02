// Browser-only stand-in for the Tauri backend, so the UI can be previewed and screenshotted
// with `pnpm ui:dev` outside the app. Never loaded inside Murmur (see main.tsx).
import type { AppState, Config, Entry } from "@/lib/api";

const SAMPLE = [
  "Can we move the standup to 10:30 tomorrow?",
  "Please send the quarterly report to the team before Friday.",
  "I think the new dictation feature is ready to ship.",
  "Remind me to call the bank about the card.",
  "The deployment failed because the environment variable was missing, so I added it and reran the pipeline.",
  "Let's sync after lunch to review the design.",
  "Thanks for the update, looks good to me.",
  "Draft an email to Priya about the offsite agenda and the travel budget for next month.",
];

function makeHistory(): Entry[] {
  const out: Entry[] = [];
  const now = Date.now();
  let seed = 7;
  const rand = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  for (let i = 0; i < 160; i++) {
    const t = now - rand() * 21 * 86_400_000;
    const d = new Date(t);
    d.setHours(9 + Math.floor(rand() * 10), Math.floor(rand() * 60));
    const text = SAMPLE[Math.floor(rand() * SAMPLE.length)];
    const secs = Math.max(1, text.split(" ").length / 2.6 + rand() * 6);
    out.push({ time: d.toISOString(), text, raw: text, audio_seconds: secs, transcribe_ms: Math.round(780 + secs * 28 + rand() * 120) });
  }
  return out.sort((a, b) => b.time.localeCompare(a.time));
}

const history = makeHistory();
let notes = "Ideas for the offsite:\n- team dinner on Thursday\n- one hour for demos";
let config: Config = {
  hotkey: "fn", model: "large-v3-turbo-q5_0", language: "en", translate: false, input_device: null,
  vocabulary: ["Acme Corp", "Kubernetes"], replacements: [{ from: "acme corp", to: "Acme Corp" }], remove_fillers: true,
  restore_clipboard: true, save_history: true, sounds: true, unload_after_minutes: 0,
};

const state = (): AppState => ({
  config, status: "Ready — hold Fn to dictate", model_loaded: true,
  permissions: { accessibility: true, microphone: "granted" },
  models: [
    { id: "large-v3-turbo-q5_0", label: "Large v3 Turbo", size_mb: 547, note: "Recommended: best accuracy for its speed.", downloaded: true },
    { id: "small", label: "Small (multilingual)", size_mb: 466, note: "Fast; good for Hindi and translate.", downloaded: false },
  ],
  devices: ["MacBook Pro Microphone", "AirPods Pro"],
  hotkeys: [{ id: "fn", label: "Fn / 🌐" }, { id: "right_option", label: "Right Option ⌥" }],
  login_enabled: true, version: "0.1.0", data_dir: "~/Library/Application Support/Murmur",
});

function stats() {
  const words = history.reduce((s, e) => s + e.text.split(/\s+/).length, 0);
  const today = new Date().toDateString();
  return {
    dictations: history.length,
    words,
    words_today: history.filter((e) => new Date(e.time).toDateString() === today).reduce((s, e) => s + e.text.split(/\s+/).length, 0),
    audio_seconds: history.reduce((s, e) => s + e.audio_seconds, 0),
    minutes_saved: words / 40 - history.reduce((s, e) => s + e.audio_seconds, 0) / 60,
    avg_transcribe_ms: Math.round(history.reduce((s, e) => s + e.transcribe_ms, 0) / history.length),
  };
}

const handlers: Record<string, (a: any) => unknown> = {
  get_state: state,
  save_config: (a) => ((config = a.config), { restarting: false, reloading_model: false }),
  history_list: (a) => history.filter((e) => !a.query || e.text.toLowerCase().includes(a.query.toLowerCase())).slice(0, a.limit ?? 500),
  get_stats: stats,
  get_notes: () => notes,
  save_notes: (a) => void (notes = a.text),
  set_login: (a) => a.enabled,
  "plugin:event|listen": () => Math.floor(Math.random() * 1e6),
  "plugin:event|unlisten": () => null,
};

let cbId = 0;
(window as any).__TAURI_INTERNALS__ = {
  transformCallback: () => ++cbId,
  invoke: async (cmd: string, args: any) => {
    const h = handlers[cmd];
    if (!h) return null;
    return h(args ?? {});
  },
};
(window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
