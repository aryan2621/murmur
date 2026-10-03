import { useEffect, useMemo, useRef, useState } from "react";
import { Area, AreaChart, XAxis } from "recharts";
import { ArrowRight, ClipboardCopy, Eraser, Flame, NotebookPen, ShieldAlert } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { ChartContainer, ChartTooltip, type ChartConfig } from "@/components/ui/chart";
import { Textarea } from "@/components/ui/textarea";
import { EntryRow } from "@/components/entry-row";
import { EmptyState, Kbd, PageHeader, Panel, SectionTitle } from "@/components/bits";
import { api, events, wordCount } from "@/lib/api";
import { formatMinutes, streak, wordsPerDay } from "@/lib/metrics";
import { useAppState, useHistory } from "@/hooks/use-app";
import { focusedField, insertAtCursor } from "@/lib/utils";
import type { Tab } from "@/App";

function greeting() {
  const h = new Date().getHours();
  return h < 5 ? "Working late" : h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : "Good evening";
}

const spark: ChartConfig = { value: { label: "Words", color: "var(--chart-1)" } };

export default function Home({ app, goTo }: { app: ReturnType<typeof useAppState>; goTo: (t: Tab) => void }) {
  const { entries, stats } = useHistory("", 100_000);
  const [notes, setNotes] = useState("");
  const [flash, setFlash] = useState(false);
  const notesRef = useRef<HTMLTextAreaElement>(null);
  const saveTimer = useRef<number | undefined>(undefined);
  const perms = app.state?.permissions;
  const key = app.state?.hotkeys.find((h) => h.id === app.state?.config.hotkey)?.label.split(" ")[0] ?? "fn";

  const week = useMemo(() => wordsPerDay(entries ?? [], 7), [entries]);
  const days = useMemo(() => streak(entries ?? []), [entries]);
  const today = week.at(-1)?.value ?? 0;
  const avg = week.slice(0, -1).reduce((s, d) => s + d.value, 0) / 6;

  useEffect(() => {
    api.notes().then(setNotes);
  }, []);

  const updateNotes = (value: string) => {
    setNotes(value);
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => api.saveNotes(value), 400);
  };

  // Dictation while Murmur is in front: into whichever field has focus, else the scratchpad.
  useEffect(() => {
    const un = events.dictation((text) => {
      const el = focusedField();
      if (el && el !== notesRef.current) return insertAtCursor(el, text);
      const pad = notesRef.current;
      if (!pad) return;
      if (el !== pad) {
        pad.focus();
        pad.setSelectionRange(pad.value.length, pad.value.length);
        if (pad.value && !pad.value.endsWith("\n")) insertAtCursor(pad, "\n");
      }
      insertAtCursor(pad, text);
      setFlash(true);
      window.setTimeout(() => setFlash(false), 700);
    });
    return () => {
      un.then((u) => u());
    };
  }, []);

  const date = new Date().toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" });

  return (
    <>
      <PageHeader eyebrow={date} title={<>{greeting()}<span className="text-muted-foreground/60">.</span></>} />

      {perms && !perms.accessibility && (
        <Alert className="mb-4 rounded-xl">
          <ShieldAlert />
          <AlertTitle>Allow Accessibility</AlertTitle>
          <AlertDescription>Needed to catch the dictation key and type text for you.</AlertDescription>
          <AlertAction>
            <Button size="sm" onClick={() => api.openPrivacy("accessibility")}>Open Settings</Button>
          </AlertAction>
        </Alert>
      )}
      {perms?.microphone === "denied" && (
        <Alert className="mb-4 rounded-xl">
          <ShieldAlert />
          <AlertTitle>Allow Microphone</AlertTitle>
          <AlertDescription>Murmur can't hear you. Turn it on in Privacy &amp; Security.</AlertDescription>
          <AlertAction>
            <Button size="sm" onClick={() => api.openPrivacy("microphone")}>Open Settings</Button>
          </AlertAction>
        </Alert>
      )}

      {/* Hero: today's words with a 7-day sparkline, and the streak. */}
      <div className="grid grid-cols-[1.65fr_1fr] gap-4">
        <div className="relative overflow-hidden rounded-2xl bg-lavender p-6 text-lavender-foreground">
          <div className="flex items-start justify-between">
            <div>
              <div className="text-xs font-medium tracking-wide uppercase opacity-70">Today</div>
              <div className="mt-2 font-display text-[76px] leading-[0.85] tracking-[-0.03em] tabular-nums">{today.toLocaleString()}</div>
              <div className="mt-2 text-sm opacity-80">
                words dictated
                {avg > 0 && <span className="opacity-80"> · {today >= avg ? "above" : "below"} your {Math.round(avg)}/day average</span>}
              </div>
            </div>
          </div>
          <ChartContainer config={spark} className="mt-4 aspect-auto h-20 w-full">
            <AreaChart data={week} margin={{ left: 2, right: 2, top: 4, bottom: 0 }}>
              <defs>
                <linearGradient id="spark-fill" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--color-value)" stopOpacity={0.35} />
                  <stop offset="100%" stopColor="var(--color-value)" stopOpacity={0} />
                </linearGradient>
              </defs>
              <XAxis dataKey="weekday" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "currentColor", opacity: 0.6 }} interval={0} padding={{ left: 14, right: 14 }} />
              <ChartTooltip
                cursor={{ stroke: "currentColor", strokeOpacity: 0.25 }}
                content={({ active, payload }) => {
                  const p = active && payload?.[0]?.payload;
                  return p ? (
                    <div className="rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md">
                      <span className="font-medium">{p.value.toLocaleString()} words</span> · {p.label}
                    </div>
                  ) : null;
                }}
              />
              <Area dataKey="value" type="monotone" stroke="var(--color-value)" strokeWidth={2} fill="url(#spark-fill)" dot={false} activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--lavender)" }} />
            </AreaChart>
          </ChartContainer>
        </div>

        <div className="flex flex-col justify-between rounded-2xl bg-forest p-6 text-forest-foreground">
          <div className="flex items-center gap-2 text-xs font-medium tracking-wide uppercase opacity-75">
            <Flame className="size-3.5" /> Streak
          </div>
          <div>
            <div className="font-display text-[64px] leading-[0.85] tracking-[-0.03em] tabular-nums">{days}</div>
            <div className="mt-1.5 text-sm opacity-80">{days === 1 ? "day" : "days"} in a row</div>
          </div>
          <div className="border-t border-white/15 pt-3 text-xs">
            <span className="font-display text-2xl leading-none">{stats ? formatMinutes(stats.minutes_saved) : "–"}</span>
            <span className="ml-2 opacity-70">saved vs typing</span>
          </div>
        </div>
      </div>

      <SectionTitle>Scratchpad</SectionTitle>
      <Panel className={flash ? "ring-2 ring-primary/40 transition-shadow" : "transition-shadow"}>
        <div className="flex items-center gap-2 border-b px-5 py-3 text-xs text-muted-foreground">
          <NotebookPen className="size-3.5" />
          Hold <Kbd>{key}</Kbd> while this window is open to dictate here.
        </div>
        <Textarea
          ref={notesRef}
          value={notes}
          onChange={(e) => updateNotes(e.target.value)}
          placeholder="Start talking…"
          className="min-h-40 resize-y rounded-none border-0 bg-transparent px-5 py-4 text-[15px] leading-relaxed shadow-none focus-visible:ring-0 dark:bg-transparent"
        />
        <div className="flex items-center justify-between border-t px-3 py-2">
          <span className="pl-2 text-xs text-muted-foreground tabular-nums">{wordCount(notes)} words</span>
          <div className="flex gap-1">
            <Button
              variant="ghost"
              size="sm"
              disabled={!notes.trim()}
              onClick={async () => {
                await api.copy(notes);
                toast.success("Scratchpad copied");
              }}
            >
              <ClipboardCopy /> Copy
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={!notes}
              onClick={() => {
                const previous = notes;
                updateNotes("");
                toast("Scratchpad cleared", { action: { label: "Undo", onClick: () => updateNotes(previous) } });
              }}
            >
              <Eraser /> Clear
            </Button>
          </div>
        </div>
      </Panel>

      <SectionTitle
        action={
          <Button variant="ghost" size="sm" className="-mr-2 text-muted-foreground" onClick={() => goTo("history")}>
            View all <ArrowRight />
          </Button>
        }
      >
        Recent
      </SectionTitle>
      <Panel className="overflow-hidden">
        {entries === null ? null : entries.length ? (
          entries.slice(0, 3).map((e) => <EntryRow key={e.time} entry={e} />)
        ) : (
          <EmptyState title="Nothing yet">Hold your dictation key in any text field and start talking.</EmptyState>
        )}
      </Panel>
    </>
  );
}
