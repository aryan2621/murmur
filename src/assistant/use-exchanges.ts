import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { api, events, type Exchange } from "@/lib/api";

/** The assistant's question/answer history, refreshed whenever a new answer is saved. */
export function useExchanges(query = "", limit?: number) {
  const [entries, setEntries] = useState<Exchange[] | null>(null);
  const request = useRef(0);
  const refresh = useCallback(async () => {
    const id = ++request.current;
    try {
      const next = await api.exchanges(query, limit);
      if (id === request.current) setEntries(next);
    } catch (error) {
      if (id === request.current) toast.error(`Couldn't load history: ${error}`, { id: "history-error" });
    }
  }, [query, limit]);

  useEffect(() => {
    setEntries(null);
    refresh();
    const un = events.historyUpdated(refresh);
    return () => {
      ++request.current;
      un.then((u) => u());
    };
  }, [refresh]);

  return { entries, refresh };
}
