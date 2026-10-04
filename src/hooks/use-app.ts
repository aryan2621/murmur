import { useCallback, useEffect, useState } from "react";
import { api, events, type AppState, type Entry, type Stats } from "@/lib/api";

/** Live app state (config, permissions, status) refreshed on focus and status changes. */
export function useAppState() {
  const [state, setState] = useState<AppState | null>(null);
  const refresh = useCallback(() => api.state().then(setState), []);

  useEffect(() => {
    refresh();
    const unStatus = events.status((status) => {
      setState((s) => (s ? { ...s, status } : s));
      if (status.startsWith("Ready")) refresh();
    });
    // Download progress (speech and AI models), so every page shows it live.
    const unProgress = events.downloadProgress(({ model, progress }) => {
      setState((s) => {
        if (!s) return s;
        const downloads = { ...s.downloads };
        if (progress === null) delete downloads[model];
        else downloads[model] = progress;
        return { ...s, downloads, model_progress: model === "speech" ? progress : s.model_progress };
      });
      if (progress === null) refresh();
    });
    window.addEventListener("focus", refresh);
    return () => {
      unStatus.then((u) => u());
      unProgress.then((u) => u());
      window.removeEventListener("focus", refresh);
    };
  }, [refresh]);

  return { state, setState, refresh };
}

/** History entries + stats, refreshed whenever a dictation is saved. */
export function useHistory(query = "", limit?: number) {
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const refresh = useCallback(() => {
    api.history(query, limit).then(setEntries);
    api.stats().then(setStats);
  }, [query, limit]);

  useEffect(() => {
    refresh();
    const un = events.historyUpdated(refresh);
    return () => {
      un.then((u) => u());
    };
  }, [refresh]);

  return { entries, stats, refresh };
}
