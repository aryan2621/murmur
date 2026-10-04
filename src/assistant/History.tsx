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
import { ExchangeRow } from "@/assistant/exchange-row";
import { EmptyState, PageHeader, Panel } from "@/components/bits";
import { usePagedHistory } from "@/hooks/use-paged";
import { Pager } from "@/components/pager";
import { api, type Exchange } from "@/lib/api";

function dayLabel(d: Date) {
  const today = new Date();
  const yesterday = new Date(Date.now() - 86_400_000);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", month: "short", day: "numeric" });
}

export default function AssistantHistory() {
  const [query, setQuery] = useState("");
  const deferred = useDeferredValue(query);
  const { entries, total, page, setPage, size, setSize, pages, refresh } = usePagedHistory(api.exchangePage, deferred);

  const groups = useMemo(() => {
    const out: { label: string; items: Exchange[] }[] = [];
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
        description={entries ? `${total.toLocaleString()} question${total === 1 ? "" : "s"}${query ? " match" : ""} · stored only on this Mac` : "Stored only on this Mac"}
        actions={
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button variant="outline" size="sm" disabled={!total}>
                <Trash2 /> Clear all
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Delete all history?</AlertDialogTitle>
                <AlertDialogDescription>Every saved question and answer is removed from this Mac. This can't be undone.</AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction
                  variant="destructive"
                  onClick={async () => {
                    try {
                      await api.clearExchanges();
                      toast("History cleared");
                      refresh();
                    } catch (error) {
                      toast.error(`Couldn't clear history: ${error}`);
                    }
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
          placeholder="Search questions and answers"
          className="h-11 rounded-xl bg-card pl-10 text-[14px] shadow-lift"
          type="search"
        />
      </div>

      {entries && !entries.length ? (
        <Panel>
          <EmptyState title={query ? "No matches" : "No questions yet"}>
            {query ? "Try a different word." : "Everything you ask shows up here, with its answer."}
          </EmptyState>
        </Panel>
      ) : (
        groups.map((g) => (
          <section key={g.label} className="mb-8">
            <div className="mb-3 px-1">
              <h2 className="font-display text-[26px] leading-none">{g.label}</h2>
            </div>
            <Panel className="overflow-hidden">
              {g.items.map((e) => <ExchangeRow key={e.time} entry={e} onDeleted={refresh} />)}
            </Panel>
          </section>
        ))
      )}
      <Pager page={page} pages={pages} size={size} total={total} onPage={setPage} onSize={setSize} />
    </>
  );
}
