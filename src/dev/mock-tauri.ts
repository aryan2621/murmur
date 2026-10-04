// Browser-only stand-in for the Tauri backend, so the UI can be previewed and screenshotted
// with `pnpm ui:dev` outside the app. Never loaded inside Murmur (see main.tsx).
import type { AppState, Config, Entry, Exchange } from "@/lib/api";

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

const QUESTIONS: [string, string][] = [
  ["What's a good way to stay focused while working?", "Try working in short, timed blocks with a five minute break after each, and keep your phone out of reach."],
  ["How many litres of water should I drink in a day?", "Most adults do well with around two to three litres a day, a bit more if it's hot or you're exercising."],
  ["Give me a quick dinner idea with paneer.", "Make a quick paneer bhurji: crumble paneer into onions, tomatoes and spices, and serve it with roti."],
  ["What's the difference between a virus and bacteria?", "Bacteria are living cells that can grow on their own, while a virus needs to get inside your cells to multiply."],
  ["Explain what an API is in simple words.", "An API is a menu that lets one program ask another for something, like a waiter taking your order to the kitchen."],
  ["Suggest a name for a cat.", "How about Mochi? It's short, cute and easy to call out."],
];

function makeExchanges(): Exchange[] {
  const out: Exchange[] = [];
  const now = Date.now();
  let seed = 11;
  const rand = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  for (let i = 0; i < 90; i++) {
    const d = new Date(now - rand() * 14 * 86_400_000);
    d.setHours(9 + Math.floor(rand() * 12), Math.floor(rand() * 60));
    const [question, answer] = QUESTIONS[Math.floor(rand() * QUESTIONS.length)];
    const mode = (["quick", "quick", "conversation", "typed"] as const)[Math.floor(rand() * 4)];
    const audio = mode === "typed" ? 0 : 2 + rand() * 6;
    out.push({
      time: d.toISOString(), question, answer, mode, audio_seconds: audio,
      heard_ms: mode === "typed" ? 0 : Math.round(780 + rand() * 120),
      first_word_ms: Math.round(220 + rand() * 200), total_ms: Math.round(900 + rand() * 900), model: "Qwen3 8B",
    });
  }
  return out.sort((a, b) => b.time.localeCompare(a.time));
}

const history = makeHistory();
const exchanges = makeExchanges();
let notes = "Ideas for the offsite:\n- team dinner on Thursday\n- one hour for demos";
let config: Config = {
  hotkey: "fn", model: "large-v3-turbo-q5_0", language: "en", translate: false, input_device: null,
  vocabulary: ["Acme Corp", "Kubernetes"], replacements: [{ from: "acme corp", to: "Acme Corp" }], remove_fillers: true,
  restore_clipboard: true, save_history: true, sounds: true, unload_after_minutes: 0,
  // `?setup` in the URL previews the first-run setup (`?setup=2` opens a step).
  setup_done: !new URLSearchParams(location.search).has("setup"),
  assistant_hotkey: "right_option", assistant_enabled: true,
  brain: "builtin", builtin_model: "8b", ollama_url: "http://localhost:11434", llm_model: "qwen3:8b", keep_alive: "30m", system_prompt: null,
  assistant_name: "Jarvis", voice: "Tara", speech_rate: 195, speak_replies: true,
  forget_after_minutes: 5, history_turns: 8, pause_seconds: 1.2, conversation_timeout_seconds: 20,
  speech_threshold: 0.012, actions: true, web_access: true, location: "",
};
const mockStep = new URLSearchParams(location.search).get("setup");
if (mockStep) localStorage.setItem("murmur.setupStep", mockStep);
// `?setup&fresh` shows the speech model step before anything is downloaded.
const fresh = new URLSearchParams(location.search).has("fresh");

const state = (): AppState => ({
  config, status: fresh ? "Finish setup to download the speech model" : "Ready — hold Fn to dictate, Right ⌥ to ask", model_loaded: !fresh,
  // `?setup=2&fresh&dl=0.4` previews the download progress bar.
  model_progress: new URLSearchParams(location.search).has("dl") ? Number(new URLSearchParams(location.search).get("dl")) : null,
  downloads: new URLSearchParams(location.search).has("dl") ? { brain: Number(new URLSearchParams(location.search).get("dl")) } : {},
  assistant: {
    unavailable: fresh ? "Download the assistant's AI in Murmur → Assistant" : null,
    brain_ready: !fresh, brain_label: "Qwen3 8B", brain_size_mb: 4795,
    ollama: { running: false, models: [], error: null },
    voices: [
      { name: "Daniel", locale: "en_GB" }, { name: "Samantha", locale: "en_US" }, { name: "Aman", locale: "en_IN" },
      { name: "Rishi", locale: "en_IN" }, { name: "Tara", locale: "en_IN" }, { name: "Lekha", locale: "hi_IN" },
      { name: "Karen", locale: "en_AU" }, { name: "Moira", locale: "en_IE" },
    ],
    brains: [
      { id: "8b", label: "Qwen3 8B", size_mb: 4795, min_ram_gb: 16, note: "The built-in AI. Accurate with multi-step tasks like clicking through pages and apps.", downloaded: !fresh },
      { id: "4b", label: "Qwen3 4B", size_mb: 2382, min_ram_gb: 8, note: "Lighter and quicker, for Macs with less than 16 GB of memory. Weaker at multi-step tasks.", downloaded: false },
    ],
    ram_gb: 16,
  },
  permissions: fresh ? { accessibility: true, microphone: "not_asked" } : { accessibility: true, microphone: "granted" },
  models: [
    { id: "large-v3-turbo-q5_0", label: "Large v3 Turbo", size_mb: 547, note: "Recommended: best accuracy for its speed.", downloaded: !fresh },
    { id: "small", label: "Small (multilingual)", size_mb: 466, note: "Fast; good for Hindi and translate.", downloaded: false },
    { id: "base.en", label: "Base (English)", size_mb: 142, note: "Very fast, less accurate.", downloaded: false },
  ],
  devices: ["MacBook Pro Microphone", "AirPods Pro"],
  hotkeys: [{ id: "fn", label: "Fn / 🌐" }, { id: "right_option", label: "Right Option ⌥" }, { id: "right_command", label: "Right Command ⌘" }],
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
  history_page: (a) => {
    const all = history.filter((e) => !a.query || e.text.toLowerCase().includes(a.query.toLowerCase()));
    return { entries: all.slice(a.page * a.pageSize, (a.page + 1) * a.pageSize), total: all.length };
  },
  assistant_history_page: (a) => {
    const all = exchanges.filter((e) => !a.query || e.question.toLowerCase().includes(a.query.toLowerCase()));
    return { entries: all.slice(a.page * a.pageSize, (a.page + 1) * a.pageSize), total: all.length };
  },
  assistant_history_list: (a) => exchanges.filter((e) => !a.query || e.question.toLowerCase().includes(a.query.toLowerCase())).slice(0, a.limit ?? 500),
  get_notes: () => notes,
  save_notes: (a) => void (notes = a.text),
  set_login: (a) => a.enabled,
  finish_setup: () => void (config = { ...config, setup_done: true }),
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
