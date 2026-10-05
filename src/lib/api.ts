// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Replacement {
  from: string;
  to: string;
}

export interface Config {
  /** Dictation key. */
  hotkey: string;
  /** Assistant key; must differ from `hotkey`. */
  assistant_hotkey: string;
  /** Whisper model, shared by dictation and the assistant. */
  model: string;
  language: string;
  translate: boolean;
  input_device: string | null;
  vocabulary: string[];
  replacements: Replacement[];
  remove_fillers: boolean;
  restore_clipboard: boolean;
  save_history: boolean;
  sounds: boolean;
  unload_after_minutes: number;
  setup_done: boolean;
  // ---- Assistant ----
  assistant_enabled: boolean;
  /** "builtin" (the AI bundled with Murmur) or "ollama". */
  brain: "builtin" | "ollama";
  /** Which built-in model: "8b" (default) or "4b" (lighter). */
  builtin_model: string;
  ollama_url: string;
  llm_model: string;
  keep_alive: string;
  system_prompt: string | null;
  assistant_name: string;
  voice: string;
  speech_rate: number;
  /** "system" (macOS voices) or "natural" (Kokoro, an AI voice that runs on the Mac). */
  voice_engine: "system" | "natural";
  /** Natural voice's speaker: "af_heart", "bm_george", … */
  natural_voice: string;
  speak_replies: boolean;
  forget_after_minutes: number;
  history_turns: number;
  pause_seconds: number;
  conversation_timeout_seconds: number;
  speech_threshold: number;
  /** Act on the Mac: apps, websites, timers, reminders, calendar, volume, files… */
  actions: boolean;
  /** Web search, reading pages and weather. Only search words and addresses leave the Mac. */
  web_access: boolean;
  /** City for the weather when none is named; empty = guessed from the connection. */
  location: string;
}

export interface BrainInfo {
  id: string;
  label: string;
  size_mb: number;
  min_ram_gb: number;
  note: string;
  downloaded: boolean;
}

export interface Voice {
  name: string;
  locale: string;
}

/** The assistant's side of the app state (null where there is no assistant). */
export interface AssistantState {
  /** Why it can't answer right now (turned off, AI not downloaded), or null. */
  unavailable: string | null;
  brain_ready: boolean;
  brain_label: string;
  brain_size_mb: number;
  ollama: { running: boolean; models: string[]; error: string | null };
  voices: Voice[];
  natural_ready: boolean;
  natural_size_mb: number;
  natural_voices: { id: string; label: string }[];
  brains: BrainInfo[];
  ram_gb: number;
}

export interface ModelInfo {
  id: string;
  label: string;
  size_mb: number;
  note: string;
  downloaded: boolean;
}

export type DownloadKey = "speech" | "brain" | "natural";

export interface AppState {
  config: Config;
  status: string;
  model_loaded: boolean;
  /** 0–1 while the speech model downloads, otherwise null. */
  model_progress: number | null;
  /** Downloads in progress (0–1), keyed "speech" / "brain" / "voice" (the natural voice). */
  downloads: Partial<Record<DownloadKey, number>>;
  permissions: { accessibility: boolean; microphone: "granted" | "denied" | "not_asked" | "unknown" };
  models: ModelInfo[];
  devices: string[];
  hotkeys: { id: string; label: string }[];
  login_enabled: boolean;
  version: string;
  data_dir: string;
  assistant: AssistantState | null;
}

/** A dictation. */
export interface Entry {
  time: string;
  text: string;
  raw: string;
  audio_seconds: number;
  transcribe_ms: number;
}

/** A question to the assistant and its answer. */
export interface Exchange {
  time: string;
  question: string;
  answer: string;
  mode: "quick" | "conversation" | "typed" | "";
  audio_seconds: number;
  heard_ms: number;
  first_word_ms: number;
  total_ms: number;
  model: string;
  /** What was done to answer, e.g. "Opening Slack". */
  actions?: string[];
}

/** Live answer as it streams in: "thinking" (reply = what it is doing, if anything) → "speaking" (growing text) → "done". */
export interface Reply {
  state: "thinking" | "speaking" | "done";
  question: string;
  reply: string;
}

/** One page of history and how many entries match in all. */
export interface HistoryPage<T> {
  entries: T[];
  total: number;
}

export interface Stats {
  dictations: number;
  words: number;
  words_today: number;
  audio_seconds: number;
  minutes_saved: number;
  avg_transcribe_ms: number;
}

export const api = {
  state: () => invoke<AppState>("get_state"),
  saveConfig: (config: Config) =>
    invoke<{ restarting: boolean; reloading_model: boolean }>("save_config", { config }),
  history: (query = "", limit?: number) => invoke<Entry[]>("history_list", { query, limit }),
  /** `page` counts from 0. */
  historyPage: (query: string, page: number, pageSize: number) => invoke<HistoryPage<Entry>>("history_page", { query, page, pageSize }),
  deleteEntry: (time: string) => invoke<void>("history_delete", { time }),
  clearHistory: () => invoke<void>("history_clear"),
  stats: () => invoke<Stats>("get_stats"),
  copy: (text: string) => invoke<void>("copy_text", { text }),
  setLogin: (enabled: boolean) => invoke<boolean>("set_login", { enabled }),
  openPrivacy: (pane: "accessibility" | "microphone") => invoke<void>("open_privacy", { pane }),
  openDataFolder: () => invoke<void>("open_data_folder"),
  notes: () => invoke<string>("get_notes"),
  saveNotes: (text: string) => invoke<void>("save_notes", { text }),
  requestMicrophone: () => invoke<void>("request_microphone"),
  downloadModel: (model: string) => invoke<void>("download_model", { model }),
  finishSetup: () => invoke<void>("finish_setup"),
  // Assistant
  exchanges: (query = "", limit?: number) => invoke<Exchange[]>("assistant_history_list", { query, limit }),
  exchangePage: (query: string, page: number, pageSize: number) => invoke<HistoryPage<Exchange>>("assistant_history_page", { query, page, pageSize }),
  deleteExchange: (time: string) => invoke<void>("assistant_history_delete", { time }),
  clearExchanges: () => invoke<void>("assistant_history_clear"),
  downloadBrain: () => invoke<void>("download_brain"),
  downloadVoice: () => invoke<void>("download_voice"),
  previewVoice: (engine: Config["voice_engine"], voice: string, rate: number) => invoke<void>("preview_voice", { engine, voice, rate }),
  stop: () => invoke<void>("stop_speaking"),
  ask: (question: string) => invoke<void>("ask_text", { question }),
  newConversation: () => invoke<void>("new_conversation"),
  openVoiceSettings: () => invoke<void>("open_voice_settings"),
  confirm: (yes: boolean) => invoke<void>("confirm_answer", { yes }),
};

export const events = {
  status: (cb: (s: string) => void): Promise<UnlistenFn> => listen<string>("status", (e) => cb(e.payload)),
  historyUpdated: (cb: () => void) => listen("history-updated", () => cb()),
  navigate: (cb: (tab: string) => void) => listen<string>("navigate", (e) => cb(e.payload)),
  dictation: (cb: (text: string) => void) => listen<string>("dictation", (e) => cb(e.payload)),
  downloadProgress: (cb: (p: { model: DownloadKey; progress: number | null }) => void) =>
    listen<{ model: DownloadKey; progress: number | null }>("download-progress", (e) => cb(e.payload)),
  downloadError: (cb: (message: string) => void) => listen<string>("download-error", (e) => cb(e.payload)),
  // Assistant
  reply: (cb: (r: Reply) => void) => listen<Reply>("reply", (e) => cb(e.payload)),
  replyError: (cb: (message: string) => void) => listen<string>("reply-error", (e) => cb(e.payload)),
  replyStopped: (cb: () => void) => listen("reply-stopped", cb),
  /** The assistant asks before a risky action ("Move report.pdf to the Trash?"); "" once answered. */
  confirm: (cb: (question: string) => void) => listen<string>("confirm", (e) => cb(e.payload)),
};

/** Short label for a key, e.g. "fn" or "⌥". */
export const keyLabel = (state: AppState, id: string) => {
  const label = state.hotkeys.find((h) => h.id === id)?.label ?? id;
  return id === "fn" ? label.split(" ")[0] : label.split(" ").at(-1)!;
};

/** Time from finishing speaking to hearing the first words of the answer. */
export const responseMs = (e: Exchange) => e.heard_ms + e.first_word_ms;

export const formatSeconds = (ms: number) => (ms >= 10_000 ? `${Math.round(ms / 1000)} s` : `${(ms / 1000).toFixed(1)} s`);

export const wordCount = (s: string) => (s.trim() ? s.trim().split(/\s+/).length : 0);
