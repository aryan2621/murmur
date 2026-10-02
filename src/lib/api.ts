// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Replacement {
  from: string;
  to: string;
}

export interface Config {
  hotkey: string;
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
}

export interface ModelInfo {
  id: string;
  label: string;
  size_mb: number;
  note: string;
  downloaded: boolean;
}

export interface AppState {
  config: Config;
  status: string;
  model_loaded: boolean;
  permissions: { accessibility: boolean; microphone: "granted" | "denied" | "not_asked" | "unknown" };
  models: ModelInfo[];
  devices: string[];
  hotkeys: { id: string; label: string }[];
  login_enabled: boolean;
  version: string;
  data_dir: string;
}

export interface Entry {
  time: string;
  text: string;
  raw: string;
  audio_seconds: number;
  transcribe_ms: number;
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
  deleteEntry: (time: string) => invoke<void>("history_delete", { time }),
  clearHistory: () => invoke<void>("history_clear"),
  stats: () => invoke<Stats>("get_stats"),
  copy: (text: string) => invoke<void>("copy_text", { text }),
  setLogin: (enabled: boolean) => invoke<boolean>("set_login", { enabled }),
  openPrivacy: (pane: "accessibility" | "microphone") => invoke<void>("open_privacy", { pane }),
  openDataFolder: () => invoke<void>("open_data_folder"),
  notes: () => invoke<string>("get_notes"),
  saveNotes: (text: string) => invoke<void>("save_notes", { text }),
};

export const events = {
  status: (cb: (s: string) => void): Promise<UnlistenFn> => listen<string>("status", (e) => cb(e.payload)),
  historyUpdated: (cb: () => void) => listen("history-updated", () => cb()),
  navigate: (cb: (tab: string) => void) => listen<string>("navigate", (e) => cb(e.payload)),
  dictation: (cb: (text: string) => void) => listen<string>("dictation", (e) => cb(e.payload)),
};

export const wordCount = (s: string) => (s.trim() ? s.trim().split(/\s+/).length : 0);
