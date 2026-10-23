import { useDeferredValue, useEffect, useMemo, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Download, Search, Trash2 } from "lucide-react";
import { toast } from "sonner";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { EntryRow } from "@/components/entry-row";
import { EmptyState, PageHeader, Panel } from "@/components/bits";
import { usePagedHistory } from "@/hooks/use-paged";
import { Pager } from "@/components/pager";
import { api, type Entry } from "@/lib/api";

function dayLabel(d: Date) {
  const today = new Date();
  const yesterday = new Date(Date.now() - 86_400_000);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", month: "short", day: "numeric" });
}

export default function DictationHistory() {
  const [query, setQuery] = useState("");
  const deferred = useDeferredValue(query);
  const { entries, total, page, setPage, size, setSize, pages, refresh } = usePagedHistory(api.historyPage, deferred);
  const [dropping, setDropping] = useState(false);

  // Audio files dropped on the window are transcribed into History.
  useEffect(() => {
    const off = getCurrentWebview().onDragDropEvent(async ({ payload }) => {
      if (payload.type === "over" || payload.type === "enter") return setDropping(true);
      setDropping(false);
      if (payload.type !== "drop") return;
      for (const path of payload.paths) {
        const name = path.split(/[\\/]/).pop();
        const id = toast.loading(`Transcribing ${name}…`);
        try {
          await api.transcribeFile(path);
          toast.success(`Transcribed ${name}`, { id });
          refresh();
        } catch (e) {
          toast.error(`${name}: ${e}`, { id });
        }
      }
    });
    return () => void off.then((f) => f());
  }, [refresh]);

  const groups = useMemo(() => {
    const out: { label: string; items: Entry[] }[] = [];
    for (const e of entries ?? []) {
      const label = dayLabel(new Date(e.time));
      if (out.at(-1)?.label !== label) out.push({ label, items: [] });
      out.at(-1)!.items.push(e);
    }
    return out;
  }, [entries]);

  return (
    <>
      <PageHeader
        title="History"
        description={`${entries ? `${total.toLocaleString()} dictation${total === 1 ? "" : "s"}${query ? " match" : ""} · ` : ""}${dropping ? "Drop to transcribe" : "Stored only on this Mac · drop an audio file here to transcribe it"}`}
        actions={
          <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={!total}
            onClick={() => api.exportHistory("md").then((p) => toast(`Exported to ${p}`), (e) => toast.error(`Couldn't export: ${e}`))}
          >
            <Download /> Export
          </Button>
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button variant="outline" size="sm" disabled={!total}>
                <Trash2 /> Clear all
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Delete all history?</AlertDialogTitle>
                <AlertDialogDescription>Every saved dictation is removed from this Mac. This can't be undone.</AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction
                  variant="destructive"
                  onClick={async () => {
                    await api.clearHistory();
                    toast("History cleared");
                    refresh();
                  }}
                >
                  Delete all
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
          </div>
        }
      />

      <div className="relative mb-8">
        <Search className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search everything you've said"
          className="h-11 rounded-xl bg-card pl-10 text-[14px] shadow-lift"
          type="search"
        />
      </div>

      {entries && !entries.length ? (
        <Panel>
          <EmptyState title={query ? "No matches" : "No dictations yet"}>
            {query ? "Try a different word." : "Everything you dictate shows up here."}
          </EmptyState>
        </Panel>
      ) : (
        groups.map((g) => (
          <section key={g.label} className="mb-8">
            <div className="mb-3 px-1">
              <h2 className="font-display text-[26px] leading-none">{g.label}</h2>
            </div>
            <Panel className="overflow-hidden">
              {g.items.map((e) => <EntryRow key={e.time} entry={e} onDeleted={refresh} onUpdated={refresh} />)}
            </Panel>
          </section>
        ))
      )}
      <Pager page={page} pages={pages} size={size} total={total} onPage={setPage} onSize={setSize} />
    </>
  );
}
