import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowRight, ClipboardCopy, Eraser, NotebookPen, ShieldAlert } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { EntryRow } from "@/components/entry-row";
import { Kbd, PageHeader, Panel, SectionTitle } from "@/components/bits";
import { api, events, wordCount } from "@/lib/api";
import { wordsPerDay } from "@/lib/metrics";
import { useAppState, useHistory } from "@/hooks/use-app";
import { focusedField, insertAtCursor } from "@/lib/utils";
import type { Tab } from "@/App";

function greeting() {
  const h = new Date().getHours();
  return h < 5 ? "Working late" : h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : "Good evening";
}

export default function Home({ app, goTo }: { app: ReturnType<typeof useAppState>; goTo: (t: Tab) => void }) {
  const { entries } = useHistory("", 100_000);
  const [notes, setNotes] = useState("");
  const [flash, setFlash] = useState(false);
  const notesRef = useRef<HTMLTextAreaElement>(null);
  const saveTimer = useRef<number | undefined>(undefined);
  const perms = app.state?.permissions;
  const key = app.state?.hotkeys.find((h) => h.id === app.state?.config.hotkey)?.label.split(" ")[0] ?? "fn";

  const today = useMemo(() => wordsPerDay(entries ?? [], 1).at(-1)?.value ?? 0, [entries]);

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
  const fresh = entries !== null && entries.length === 0;

  return (
    <>
      <PageHeader
        title={<>{greeting()}<span className="text-primary">.</span></>}
        description={fresh ? <>Hold <Kbd>{key}</Kbd> in any app, speak, and let go. Your words appear where you were typing.</> : <>{date} · {today ? `${today.toLocaleString()} words dictated today` : "Nothing dictated yet today"}</>}
      />

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
          <p className="px-5 py-6 text-sm text-muted-foreground">Your last three dictations show up here, ready to copy or fix.</p>
        )}
      </Panel>
    </>
  );
}
