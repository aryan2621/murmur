import { useDeferredValue, useMemo, useState } from "react";
import { Search, Trash2 } from "lucide-react";
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
import { useHistory } from "@/hooks/use-app";
import { api, type Entry } from "@/lib/api";

function dayLabel(d: Date) {
  const today = new Date();
  const yesterday = new Date(Date.now() - 86_400_000);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", month: "short", day: "numeric" });
}

export default function History() {
  const [query, setQuery] = useState("");
  const deferred = useDeferredValue(query);
  const { entries, refresh } = useHistory(deferred);

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
        description={entries ? `${entries.length.toLocaleString()} dictation${entries.length === 1 ? "" : "s"}${query ? " match" : ""} · stored only on this Mac` : "Stored only on this Mac"}
        actions={
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button variant="outline" size="sm" disabled={!entries?.length}>
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
            <div className="mb-3 flex items-baseline justify-between px-1">
              <h2 className="font-display text-[26px] leading-none">{g.label}</h2>
              <span className="text-xs text-muted-foreground tabular-nums">{g.items.length} dictation{g.items.length === 1 ? "" : "s"}</span>
            </div>
            <Panel className="overflow-hidden">
              {g.items.map((e) => <EntryRow key={e.time} entry={e} onDeleted={refresh} />)}
            </Panel>
          </section>
        ))
      )}
    </>
  );
}
