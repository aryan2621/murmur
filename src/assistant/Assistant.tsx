import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowRight, ArrowUp, Download, Loader2, MessageSquarePlus, ShieldAlert, Sparkles, Square } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ExchangeRow } from "@/assistant/exchange-row";
import { EmptyState, Kbd, PageHeader, Panel, SectionTitle } from "@/components/bits";
import { api, events, keyLabel, type Reply } from "@/lib/api";
import { setView } from "@/lib/view";
import { questionsPerDay } from "@/assistant/metrics";
import type { useAppState } from "@/hooks/use-app";
import { useBusy } from "@/hooks/use-busy";
import { useExchanges } from "@/assistant/use-exchanges";
import { cn } from "@/lib/utils";
import type { Tab } from "@/App";


/** The assistant's page: ask by typing, see the live answer, and recent questions. */
export default function Assistant({ app, goTo }: { app: ReturnType<typeof useAppState>; goTo: (t: Tab) => void }) {
  const { entries } = useExchanges("", 100_000);
  const [live, setLive] = useState<Reply | null>(null);
  const [asking, setAsking] = useState("");
  const [draft, setDraft] = useState("");
  const replyRef = useRef<HTMLDivElement>(null);
  const s = app.state;
  const perms = s?.permissions;
  const a = s?.assistant;
  const key = s ? keyLabel(s, s.config.assistant_hotkey) : "⌥";
  const name = s?.config.assistant_name || "Jarvis";
  const usesOllama = s?.config.brain === "ollama";
  const modelMissing = usesOllama && a?.ollama.running && !a.ollama.models.includes(s!.config.llm_model);
  const brainProgress = s?.downloads.brain;
  const { busy: acting, run } = useBusy();

  const today = useMemo(() => questionsPerDay(entries ?? [], 1).at(-1)?.value ?? 0, [entries]);
  const brainLabel = s && a ? (usesOllama ? s.config.llm_model : a.brain_label) : null;

  // Answers stream in here whether they were spoken, held or typed.
  useEffect(() => {
    const un = events.reply((r) => setLive(r));
    const unAsk = events.confirm(setAsking);
    const unStop = events.replyStopped(() => {
      setLive((reply) => reply ? { ...reply, state: "done" } : reply);
      setAsking("");
    });
    const unErr = events.replyError((m) => {
      toast.error(m);
      setLive(null);
    });
    return () => {
      un.then((u) => u());
      unAsk.then((u) => u());
      unStop.then((u) => u());
      unErr.then((u) => u());
    };
  }, []);

  useEffect(() => {
    replyRef.current?.scrollTo({ top: replyRef.current.scrollHeight });
  }, [live?.reply]);

  const busy = live && live.state !== "done";

  async function ask() {
    const q = draft.trim();
    if (!q) return;
    setDraft("");
    setLive({ state: "thinking", question: q, reply: "" });
    try {
      await api.ask(q);
    } catch (e) {
      toast.error(String(e));
      setDraft((current) => current || q);
      setLive(null);
    }
  }

  return (
    <>
      <PageHeader
        title={<>{name}<span className="text-primary">.</span></>}
        description={
          <>
            Hold <Kbd>{key}</Kbd> to ask a quick question, tap it for a conversation. It answers out loud and can act on this Mac.
            {brainLabel && <> Runs on {brainLabel} · {today ? `${today} question${today === 1 ? "" : "s"} today` : "no questions yet today"}.</>}
          </>
        }
      />

      {perms && !perms.accessibility && (
        <Alert className="mb-4 rounded-xl">
          <ShieldAlert />
          <AlertTitle>Allow Accessibility</AlertTitle>
          <AlertDescription>Needed so {name} can catch its key from any app.</AlertDescription>
          <AlertAction>
            <Button size="sm" onClick={() => api.openPrivacy("accessibility")}>Open Settings</Button>
          </AlertAction>
        </Alert>
      )}
      {perms?.microphone === "denied" && (
        <Alert className="mb-4 rounded-xl">
          <ShieldAlert />
          <AlertTitle>Allow Microphone</AlertTitle>
          <AlertDescription>{name} can't hear you. Turn it on in Privacy &amp; Security.</AlertDescription>
          <AlertAction>
            <Button size="sm" onClick={() => api.openPrivacy("microphone")}>Open Settings</Button>
          </AlertAction>
        </Alert>
      )}
      {s && !s.config.assistant_enabled && (
        <Alert className="mb-4 rounded-xl">
          <ShieldAlert />
          <AlertTitle>{name} is turned off</AlertTitle>
          <AlertDescription>Its key does nothing until you turn it on in Settings.</AlertDescription>
          <AlertAction>
            <Button size="sm" variant="outline" onClick={() => goTo("settings")}>Settings</Button>
          </AlertAction>
        </Alert>
      )}
      {a && s?.config.assistant_enabled && !usesOllama && !a.brain_ready && (
        <Alert className="mb-4 rounded-xl">
          <Download />
          <AlertTitle>Download {name}'s AI</AlertTitle>
          <AlertDescription>
            {a.brain_label}, about {(a.brain_size_mb / 1000).toFixed(1)} GB, once. It runs on this Mac; after that {name} works offline.
          </AlertDescription>
          <AlertAction>
            <Button
              size="sm"
              disabled={brainProgress != null || acting !== null}
              onClick={() =>
                run("download", async () => {
                  await api.downloadBrain();
                  await app.refresh();
                }, "Couldn't start the download: ")
              }
            >
              {(brainProgress != null || acting === "download") && <Loader2 className="animate-spin" />}
              {brainProgress != null ? `${Math.round(brainProgress * 100)}%` : "Download"}
            </Button>
          </AlertAction>
        </Alert>
      )}
      {a && usesOllama && !a.ollama.running && (
        <Alert className="mb-4 rounded-xl">
          <ShieldAlert />
          <AlertTitle>Ollama isn't running</AlertTitle>
          <AlertDescription>
            {name} thinks with Ollama on this Mac. Open the Ollama app, or run <code data-selectable className="rounded bg-muted px-1">brew services start ollama</code>.
          </AlertDescription>
          <AlertAction>
            <Button size="sm" variant="outline" onClick={() => app.refresh()}>Check again</Button>
          </AlertAction>
        </Alert>
      )}
      {modelMissing && (
        <Alert className="mb-4 rounded-xl">
          <ShieldAlert />
          <AlertTitle>Model “{s.config.llm_model}” isn't installed</AlertTitle>
          <AlertDescription>
            Run <code data-selectable className="rounded bg-muted px-1">ollama pull {s.config.llm_model}</code>, or pick an installed model in Settings.
          </AlertDescription>
          <AlertAction>
            <Button size="sm" variant="outline" onClick={() => goTo("settings")}>Settings</Button>
          </AlertAction>
        </Alert>
      )}

      <SectionTitle
        action={
          <Button
            variant="ghost"
            size="sm"
            className="-mr-2 text-muted-foreground"
            disabled={acting !== null}
            onClick={() =>
              run("new", async () => {
                await api.newConversation();
                setLive(null);
                toast("New conversation — earlier questions are forgotten");
              }, "Couldn't start a new conversation: ")
            }
          >
            {acting === "new" ? <Loader2 className="animate-spin" /> : <MessageSquarePlus />} New conversation
          </Button>
        }
      >
        Ask {name}
      </SectionTitle>
      <Panel className="overflow-hidden">
        <div className="flex items-center gap-2 border-b px-5 py-3 text-xs text-muted-foreground">
          <Sparkles className="size-3.5" />
          Type below, or hold <Kbd>{key}</Kbd> to ask out loud and tap it for a conversation.
        </div>
        {live && (
          <div className="border-b px-5 py-4">
            <p data-selectable className="text-[13px] text-muted-foreground">“{live.question}”</p>
            <div ref={replyRef} data-selectable className="mt-1.5 max-h-56 overflow-y-auto text-[15px] leading-relaxed whitespace-pre-wrap">
              {live.state === "thinking" ? <span className="animate-pulse text-muted-foreground">{live.reply || "Thinking…"}</span> : live.reply}
            </div>
            {asking && (
              <div className="mt-3 flex items-center gap-2">
                <span className="mr-auto text-[14px] font-medium">{asking}</span>
                <Button size="sm" onClick={() => { setAsking(""); api.confirm(true); }}>Yes</Button>
                <Button size="sm" variant="outline" onClick={() => { setAsking(""); api.confirm(false); }}>No</Button>
              </div>
            )}
          </div>
        )}
        <div className="flex items-end gap-2 px-3 py-3">
          <Textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                ask();
              }
            }}
            rows={1}
            placeholder={`Ask ${name} anything…`}
            className="max-h-32 min-h-10 flex-1 resize-none border-0 bg-transparent px-2 py-2 text-[15px] shadow-none focus-visible:ring-0 dark:bg-transparent"
          />
          {busy ? (
            <Button size="icon" variant="outline" aria-label="Stop" onClick={() => api.stop().then(() => setLive((l) => (l ? { ...l, state: "done" } : l)))}>
              <Square className="fill-current" />
            </Button>
          ) : (
            <Button size="icon" aria-label="Ask" disabled={!draft.trim()} onClick={ask} className={cn(!draft.trim() && "opacity-40")}>
              <ArrowUp />
            </Button>
          )}
        </div>
      </Panel>

      <SectionTitle
        action={
          <Button variant="ghost" size="sm" className="-mr-2 text-muted-foreground" onClick={() => { setView("history", "assistant"); goTo("history"); }}>
            View all <ArrowRight />
          </Button>
        }
      >
        Recent
      </SectionTitle>
      <Panel className="overflow-hidden">
        {entries === null ? null : entries.length ? (
          entries.slice(0, 3).map((e) => <ExchangeRow key={e.time} entry={e} />)
        ) : (
          <EmptyState title="Nothing yet">Hold {key} anywhere and ask {name} a question.</EmptyState>
        )}
      </Panel>
    </>
  );
}
