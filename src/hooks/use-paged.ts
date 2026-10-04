import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { events, type HistoryPage } from "@/lib/api";

export const PAGE_SIZES = [10, 25, 50, 100];

/** One page of history from `fetchPage`, refetched when the query, page or size changes and
 *  whenever an entry is saved or deleted. The page resets to the first when the query or size
 *  changes, and steps back if deletions leave it past the end. */
export function usePagedHistory<T>(fetchPage: (query: string, page: number, size: number) => Promise<HistoryPage<T>>, query: string) {
  const [page, setPage] = useState(0);
  const [size, setSize] = useState(PAGE_SIZES[0]);
  const [data, setData] = useState<HistoryPage<T> | null>(null);
  const request = useRef(0);

  useEffect(() => setPage(0), [query, size]);

  const refresh = useCallback(async () => {
    const id = ++request.current;
    try {
      const next = await fetchPage(query, page, size);
      if (id !== request.current) return;
      const last = Math.max(0, Math.ceil(next.total / size) - 1);
      if (page > last) setPage(last);
      else setData(next);
    } catch (error) {
      if (id === request.current) toast.error(`Couldn't load history: ${error}`, { id: "history-error" });
    }
  }, [fetchPage, query, page, size]);

  useEffect(() => {
    refresh();
    const un = events.historyUpdated(refresh);
    return () => {
      ++request.current;
      un.then((u) => u());
    };
  }, [refresh]);

  const pages = data ? Math.max(1, Math.ceil(data.total / size)) : 1;
  return { entries: data?.entries ?? null, total: data?.total ?? 0, page, setPage, size, setSize, pages, refresh };
}
