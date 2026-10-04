import { useEffect, useMemo, useState, type ReactNode } from "react";
import { ArrowLeft, ArrowRight, Brain, Check, Download, Keyboard, Loader2, Mic, PartyPopper, Play, ShieldCheck, Sparkles } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Kbd, Panel } from "@/components/bits";
import { api, events, keyLabel, type Reply, type Voice } from "@/lib/api";
import type { useAppState } from "@/hooks/use-app";
import { cn } from "@/lib/utils";

const DICTATION_STEPS = ["Welcome", "Permissions", "Speech", "Try it"] as const;
/** Optional: skipping them leaves the assistant to set up later from its page. */
const ASSISTANT_STEPS = ["Assistant", "Voice", "Ask"] as const;
const ALL_STEPS = [...DICTATION_STEPS, ...ASSISTANT_STEPS, "Done"] as const;
/** Survives the restart Murmur does when Accessibility is granted, so setup resumes there. */
const STEP_KEY = "murmur.setupStep";
/** Offered before the first download; the rest stay in Settings. */
const SETUP_MODELS = ["large-v3-turbo-q5_0", "small", "base.en"];

const savedStep = () => {
  try {
    const n = Number(localStorage.getItem(STEP_KEY));
    return Number.isInteger(n) && n > 0 && n < ALL_STEPS.length ? n : 0;
  } catch {
    return 0;
  }
};
const saveStep = (n: number | null) => {
  try {
    if (n === null) localStorage.removeItem(STEP_KEY);
    else localStorage.setItem(STEP_KEY, String(n));
  } catch {
    /* storage unavailable: setup just starts from the top next time */
  }
};

const formatSize = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`);

/** First-run setup: permissions, the speech model and a first dictation, then (optionally) the
 * assistant's AI, its voice and a first question. */
export default function Setup({ app, onDone }: { app: ReturnType<typeof useAppState>; onDone: () => void }) {
  const { state, refresh } = app;
  // Steps are numbered over all of them, so a saved step means the same thing on every Mac.
  const [step, setStep] = useState(savedStep);
  const go = (n: number) => {
    setStep(n);
    saveStep(n);
  };
  const hasAssistant = !!state?.assistant;
  const steps = hasAssistant ? ALL_STEPS : ALL_STEPS.filter((s) => !(ASSISTANT_STEPS as readonly string[]).includes(s));
  const done = ALL_STEPS.length - 1;
  /** Index of a step in the progress bar, which leaves out the assistant's on Windows. */
  const shown = steps.indexOf(ALL_STEPS[step]);

  const finish = async () => {
    try {
      await api.finishSetup();
    } catch (e) {
      toast.error(`Couldn't save: ${e}`);
      return;
    }
    saveStep(null);
    onDone();
  };

  if (!state) return null;
  const key = keyLabel(state, state.config.hotkey);
  const askKey = keyLabel(state, state.config.assistant_hotkey);
  const name = state.config.assistant_name || "Jarvis";
  const afterTry = hasAssistant ? 4 : done;

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background">
      <div data-tauri-drag-region className="flex h-12 shrink-0 items-center justify-end px-4">
        {step < done && (
          <Button variant="ghost" size="sm" className="text-muted-foreground" onClick={finish}>
            Skip setup
          </Button>
        )}
      </div>
      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-[560px] px-8 pt-4 pb-16">
          <ol className="mb-8 flex items-center gap-2" aria-label="Setup progress">
            {steps.map((label, i) => (
              <li key={label} className="flex flex-1 flex-col gap-1.5" aria-current={i === shown ? "step" : undefined}>
                <span className={cn("h-1 rounded-full transition-colors", i <= shown ? "bg-primary" : "bg-border")} />
                <span className={cn("text-[11px]", i === shown ? "font-medium text-foreground" : "text-muted-foreground")}>{label}</span>
              </li>
            ))}
          </ol>
          <div key={step} className="animate-in fade-in-0 slide-in-from-bottom-1 duration-300">
            {step === 0 && <Welcome onNext={() => go(1)} />}
            {step === 1 && <Permissions app={app} onBack={() => go(0)} onNext={() => go(2)} />}
            {step === 2 && <Model app={app} onBack={() => go(1)} onNext={() => go(3)} />}
            {step === 3 && <TryIt keyLabel={key} ready={state.model_loaded} onBack={() => go(2)} onNext={() => go(afterTry)} refresh={refresh} />}
            {step === 4 && <BrainStep app={app} name={name} onBack={() => go(3)} onNext={() => go(5)} onSkip={() => go(done)} />}
            {step === 5 && <VoiceStep app={app} onBack={() => go(4)} onNext={() => go(6)} />}
            {step === 6 && <AskIt app={app} keyLabel={askKey} name={name} onBack={() => go(5)} onNext={() => go(done)} />}
            {step === done && <Done keyLabel={key} askKey={state.assistant?.unavailable ? null : askKey} name={name} onFinish={finish} />}
          </div>
        </div>
      </main>
    </div>
  );
}

function StepHeader({ title, children }: { title: ReactNode; children: ReactNode }) {
  return (
    <header className="mb-6">
      <h1 className="font-display text-[40px] leading-[0.95] tracking-[-0.02em]">{title}</h1>
      <p className="mt-3 text-sm text-muted-foreground">{children}</p>
    </header>
  );
}

function Nav({ onBack, onNext, nextLabel = "Continue", nextDisabled, hint, quiet }: { onBack?: () => void; onNext: () => void; nextLabel?: string; nextDisabled?: boolean; hint?: ReactNode; quiet?: boolean }) {
  return (
    <div className="mt-6 flex items-center gap-3">
      {onBack && (
        <Button variant="ghost" onClick={onBack}>
          <ArrowLeft /> Back
        </Button>
      )}
      <span className="flex-1 text-right text-xs text-muted-foreground">{hint}</span>
      <Button variant={quiet ? "outline" : "default"} onClick={onNext} disabled={nextDisabled}>
        {nextLabel} <ArrowRight />
      </Button>
    </div>
  );
}

/** A row with a status on the right: done ✓, or an action. */
function Item({ icon, title, description, done, doneLabel = "Allowed", action, children }: { icon: ReactNode; title: string; description: ReactNode; done: boolean; doneLabel?: string; action?: ReactNode; children?: ReactNode }) {
  return (
    <div className="px-5 py-4 not-last:border-b">
      <div className="flex items-center gap-4">
      <span className={cn("grid size-9 shrink-0 place-items-center rounded-[10px]", done ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400" : "bg-muted text-muted-foreground")}>
        {done ? <Check className="size-[18px]" strokeWidth={2.5} /> : icon}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium">{title}</div>
        <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{description}</p>
      </div>
      {done ? (
        <Badge variant="secondary" className="text-emerald-700 dark:text-emerald-400">
          {doneLabel}
        </Badge>
      ) : (
        action
      )}
      </div>
      {children}
    </div>
  );
}

function Welcome({ onNext }: { onNext: () => void }) {
  return (
    <>
      <div className="mb-6 grid size-14 place-items-center rounded-2xl bg-brand shadow-lg shadow-brand/25">
        <Mic className="size-7 text-brand-foreground" strokeWidth={2.25} />
      </div>
      <StepHeader title="Welcome to Murmur">
        Hold a key, speak, and your words are typed wherever your cursor is. Speech is turned into text on this Mac, so nothing you say leaves it.
      </StepHeader>
      <Panel className="px-5 py-4 text-sm text-muted-foreground">
        Setup takes a few minutes: allow two permissions, download the speech model and try your first dictation. Then, if you like, set up the assistant you can ask out loud.
      </Panel>
      <Nav onNext={onNext} nextLabel="Set up Murmur" />
    </>
  );
}

function Permissions({ app, onBack, onNext }: { app: ReturnType<typeof useAppState>; onBack: () => void; onNext: () => void }) {
  const { state, refresh } = app;
  // Permissions change in System Settings, outside the app: check again every second or so.
  useEffect(() => {
    const id = window.setInterval(refresh, 1500);
    return () => window.clearInterval(id);
  }, [refresh]);
  if (!state) return null;
  const { accessibility, microphone } = state.permissions;
  const micDone = microphone === "granted";

  return (
    <>
      <StepHeader title="Two permissions">macOS asks you to allow these once. Murmur only listens while you hold the dictation key.</StepHeader>
      <Panel className="overflow-hidden">
        <Item
          icon={<Keyboard className="size-[18px]" />}
          title="Accessibility"
          description={<>To notice the dictation key in any app and type the text for you. Murmur restarts by itself once you turn it on, then setup carries on here.</>}
          done={accessibility}
          action={
            <Button size="sm" onClick={() => api.openPrivacy("accessibility")}>
              Allow
            </Button>
          }
        />
        <Item
          icon={<Mic className="size-[18px]" />}
          title="Microphone"
          description={microphone === "denied" ? "Access was turned off. Turn Murmur on in Privacy & Security → Microphone." : "To hear you while you hold the key."}
          done={micDone}
          action={
            microphone === "denied" ? (
              <Button size="sm" variant="outline" onClick={() => api.openPrivacy("microphone")}>
                Open Settings
              </Button>
            ) : (
              <Button size="sm" onClick={() => api.requestMicrophone().then(() => window.setTimeout(refresh, 800))}>
                Allow
              </Button>
            )
          }
        />
      </Panel>
      <Nav onBack={onBack} onNext={onNext} hint={accessibility && micDone ? null : "You can also allow these later"} nextLabel={accessibility && micDone ? "Continue" : "Continue anyway"} />
    </>
  );
}

function Model({ app, onBack, onNext }: { app: ReturnType<typeof useAppState>; onBack: () => void; onNext: () => void }) {
  const { state, refresh } = app;
  const [progress, setProgress] = useState<number | null>(state?.model_progress ?? null);
  const [choice, setChoice] = useState(state?.config.model ?? SETUP_MODELS[0]);
  const [starting, setStarting] = useState(false);

  useEffect(() => {
    const un = events.downloadProgress(({ model, progress: p }) => {
      if (model !== "speech") return;
      setProgress(p);
      if (p === null) refresh();
    });
    // Loading after the download finishes takes a moment; keep checking until it's ready.
    const id = window.setInterval(refresh, 2000);
    return () => {
      un.then((u) => u());
      window.clearInterval(id);
    };
  }, [refresh]);

  if (!state) return null;
  const current = state.models.find((m) => m.id === state.config.model);
  const downloading = progress !== null || state.model_progress !== null || starting;
  const pct = progress ?? state.model_progress ?? 0;
  const ready = state.model_loaded;
  const downloaded = !!current?.downloaded;

  const download = async () => {
    setStarting(true);
    try {
      await api.downloadModel(choice);
      await refresh();
    } catch (e) {
      toast.error(`Couldn't start the download: ${e}`);
    } finally {
      setStarting(false);
    }
  };

  let body: ReactNode;
  if (ready) {
    body = (
      <Item icon={<Download className="size-[18px]" />} title={current?.label ?? "Speech model"} description={`Downloaded (${formatSize(current?.size_mb ?? 0)}). Works offline from now on.`} done doneLabel="Ready" />
    );
  } else if (downloading || downloaded) {
    const loading = downloaded && !downloading;
    body = (
      <div className="px-5 py-5">
        <div className="flex items-center justify-between text-[13px]">
          <span className="flex items-center gap-2 font-medium">
            <Loader2 className="size-4 animate-spin text-muted-foreground" />
            {loading ? "Loading the speech model…" : `Downloading ${current?.label ?? "the speech model"}…`}
          </span>
          {!loading && <span className="text-xs text-muted-foreground tabular-nums">{Math.round(pct * 100)}%</span>}
        </div>
        <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-muted">
          <div className="h-full rounded-full bg-primary transition-[width] duration-300" style={{ width: `${loading ? 100 : Math.round(pct * 100)}%` }} />
        </div>
        <p className="mt-3 text-xs text-muted-foreground">One time only. You can keep going; it finishes in the background.</p>
      </div>
    );
  } else {
    const options = SETUP_MODELS.map((id) => state.models.find((m) => m.id === id)).filter((m) => !!m);
    body = (
      <div role="radiogroup" aria-label="Speech model">
        {options.map((m) => (
          <button
            key={m.id}
            role="radio"
            aria-checked={choice === m.id}
            onClick={() => setChoice(m.id)}
            className={cn("flex w-full items-center gap-4 px-5 py-4 text-left transition-colors not-last:border-b hover:bg-muted/40", choice === m.id && "bg-accent/60")}
          >
            <span className={cn("grid size-4 shrink-0 place-items-center rounded-full border-2", choice === m.id ? "border-primary" : "border-border")}>
              {choice === m.id && <span className="size-2 rounded-full bg-primary" />}
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-[13px] font-medium">{m.label}</span>
              <span className="mt-0.5 block text-xs text-muted-foreground">{m.note}</span>
            </span>
            <span className="text-xs text-muted-foreground tabular-nums">{formatSize(m.size_mb)}</span>
          </button>
        ))}
      </div>
    );
  }

  return (
    <>
      <StepHeader title="Speech model">Murmur turns speech into text with Whisper, running on this Mac. It needs a one-time download.</StepHeader>
      <Panel className="overflow-hidden">{body}</Panel>
      {!ready && !downloading && !downloaded ? (
        <div className="mt-6 flex items-center gap-3">
          <Button variant="ghost" onClick={onBack}>
            <ArrowLeft /> Back
          </Button>
          <span className="flex-1" />
          <Button onClick={download} disabled={starting}>
            <Download /> Download
          </Button>
        </div>
      ) : (
        <Nav onBack={onBack} onNext={onNext} />
      )}
    </>
  );
}

function TryIt({ keyLabel, ready, onBack, onNext, refresh }: { keyLabel: string; ready: boolean; onBack: () => void; onNext: () => void; refresh: () => Promise<void> }) {
  const [text, setText] = useState("");
  const [worked, setWorked] = useState(false);

  // Dictation while Murmur is in front arrives here instead of being typed into another app.
  useEffect(() => {
    const un = events.dictation((t) => {
      setText((prev) => (prev ? `${prev} ${t}` : t));
      setWorked(true);
    });
    const id = ready ? 0 : window.setInterval(refresh, 2000);
    return () => {
      un.then((u) => u());
      if (id) window.clearInterval(id);
    };
  }, [ready, refresh]);

  return (
    <>
      <StepHeader title="Try it">
        Hold <Kbd className="mx-0.5 align-middle">{keyLabel}</Kbd>, say a sentence, then let go. Your words appear below.
      </StepHeader>
      <Panel className={cn("overflow-hidden transition-shadow", worked && "ring-2 ring-emerald-500/40")}>
        <div className="flex items-center gap-2 border-b px-5 py-3 text-xs text-muted-foreground">
          {worked ? (
            <>
              <Check className="size-3.5 text-emerald-600 dark:text-emerald-400" strokeWidth={2.5} />
              <span className="font-medium text-emerald-700 dark:text-emerald-400">It works</span>
            </>
          ) : ready ? (
            <>
              <Sparkles className="size-3.5" /> Waiting for you to speak…
            </>
          ) : (
            <>
              <Loader2 className="size-3.5 animate-spin" /> The speech model is still getting ready…
            </>
          )}
        </div>
        <Textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={`Hold ${keyLabel} and say “Hello, this is my first dictation.”`}
          className="min-h-32 resize-none rounded-none border-0 bg-transparent px-5 py-4 text-[15px] leading-relaxed shadow-none focus-visible:ring-0 dark:bg-transparent"
        />
      </Panel>
      <Nav onBack={onBack} onNext={onNext} nextLabel={worked ? "Continue" : "Skip"} quiet={!worked} />
    </>
  );
}

function Done({ keyLabel, askKey, name, onFinish }: { keyLabel: string; askKey: string | null; name: string; onFinish: () => void }) {
  const tips: [ReactNode, string][] = [
    [<Kbd key="h">{keyLabel}</Kbd>, "Hold to dictate, let go to type it"],
    [
      <span key="d" className="flex gap-0.5">
        <Kbd>{keyLabel}</Kbd>
        <Kbd>{keyLabel}</Kbd>
      </span>,
      "Double-tap for hands-free; tap again to finish",
    ],
  ];
  if (askKey) {
    tips.push(
      [<Kbd key="a">hold {askKey}</Kbd>, `Ask ${name} a quick question`],
      [<Kbd key="t">tap {askKey}</Kbd>, "Start a conversation; pause and it answers, then listens again"],
    );
  }
  tips.push([<Kbd key="e">esc</Kbd>, askKey ? "Cancel a dictation, or stop an answer" : "Cancel a dictation"]);
  return (
    <>
      <div className="mb-6 grid size-14 place-items-center rounded-2xl bg-forest text-forest-foreground">
        <PartyPopper className="size-7" />
      </div>
      <StepHeader title="You're all set">Murmur lives in the menu bar. Use it in any app with a text field.</StepHeader>
      <Panel className="overflow-hidden">
        {tips.map(([keys, label], i) => (
          <div key={i} className="flex items-center gap-4 px-5 py-3.5 text-[13px] not-last:border-b">
            <span className="flex w-24 shrink-0">{keys}</span>
            {label}
          </div>
        ))}
      </Panel>
      <div className="mt-6 flex items-center gap-3">
        <span className="flex flex-1 items-center gap-1.5 text-xs text-muted-foreground">
          <ShieldCheck className="size-3.5" /> Everything stays on this Mac. You can run setup again from Settings.
        </span>
        <Button onClick={onFinish}>
          Open Murmur <ArrowRight />
        </Button>
      </div>
    </>
  );
}

/** The assistant's AI: a one-time download, or skip and set it up later. */
function BrainStep({ app, name, onBack, onNext, onSkip }: { app: ReturnType<typeof useAppState>; name: string; onBack: () => void; onNext: () => void; onSkip: () => void }) {
  const { state, refresh } = app;
  useEffect(() => {
    const unErr = events.downloadError((m) => toast.error(`Download failed — ${m}. Check your connection and try again.`));
    const id = window.setInterval(refresh, 2000);
    return () => {
      unErr.then((u) => u());
      window.clearInterval(id);
    };
  }, [refresh]);
  const a = state?.assistant;
  if (!state || !a) return null;
  const progress = state.downloads.brain ?? null;
  const usesOllama = state.config.brain === "ollama";
  const ready = a.brain_ready || usesOllama;
  return (
    <>
      <div className="mb-6 grid size-14 place-items-center rounded-2xl bg-brand shadow-lg shadow-brand/25">
        <Sparkles className="size-7 text-brand-foreground" strokeWidth={2.25} />
      </div>
      <StepHeader title={<>Meet {name}<span className="text-muted-foreground/60">.</span></>}>
        Murmur also has an assistant: ask anything out loud and hear the answer, or have it act on this Mac. Its AI runs on this Mac too, so it needs a one-time download. You can skip this and set it up later from its page.
      </StepHeader>
      <Panel className="overflow-hidden">
        <Item
          icon={<Brain className="size-[18px]" />}
          title="AI brain"
          description={usesOllama ? `Answers come from Ollama (${state.config.llm_model}).` : `${a.brain_label}, to think of the answers · ${formatSize(a.brain_size_mb)}`}
          done={ready}
          doneLabel="Ready"
        >
          {!ready && progress !== null && <ProgressBar value={progress} />}
        </Item>
      </Panel>
      {!ready && (
        <Button className="mt-4 w-full" disabled={progress !== null} onClick={() => api.downloadBrain().then(refresh)}>
          {progress !== null ? <Loader2 className="animate-spin" /> : <Download />}
          {progress !== null ? "Downloading… it keeps going in the background" : `Download (${formatSize(a.brain_size_mb)})`}
        </Button>
      )}
      <div className="mt-6 flex items-center gap-3">
        <Button variant="ghost" onClick={onBack}>
          <ArrowLeft /> Back
        </Button>
        <span className="flex-1" />
        {!ready && (
          <Button variant="ghost" onClick={onSkip}>
            Skip, maybe later
          </Button>
        )}
        <Button variant={ready ? "default" : "outline"} onClick={onNext}>
          {ready ? "Continue" : "Continue anyway"} <ArrowRight />
        </Button>
      </div>
    </>
  );
}

function ProgressBar({ value }: { value: number }) {
  const pct = Math.round(value * 100);
  return (
    <div className="mt-3 flex items-center gap-3 pl-13">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
        <div className="h-full rounded-full bg-primary transition-[width] duration-500" style={{ width: `${pct}%` }} />
      </div>
      <span className="w-9 text-right font-mono text-xs text-muted-foreground tabular-nums">{pct}%</span>
    </div>
  );
}

const languageName = (locale: string) => {
  try {
    return new Intl.DisplayNames([], { type: "language" }).of(locale.replace("_", "-")) ?? locale;
  } catch {
    return locale;
  }
};

function VoiceStep({ app, onBack, onNext }: { app: ReturnType<typeof useAppState>; onBack: () => void; onNext: () => void }) {
  const { state, setState } = app;
  // English voices, Indian and British first: the ones that suit an assistant best.
  const voices = useMemo(() => {
    const order = ["en_IN", "en_GB", "en_US", "en_AU", "en_IE", "en_ZA"];
    const rank = (v: Voice) => (order.includes(v.locale) ? order.indexOf(v.locale) : order.length);
    return (state?.assistant?.voices ?? []).filter((v) => v.locale.startsWith("en_")).sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
  }, [state?.assistant?.voices]);
  if (!state) return null;
  const c = state.config;
  const pick = async (v: Voice) => {
    const config = { ...c, voice: v.name };
    setState((s) => (s ? { ...s, config } : s));
    api.previewVoice(v.name, c.speech_rate);
    await api.saveConfig(config).catch((e) => toast.error(`Couldn't save: ${e}`));
  };
  return (
    <>
      <StepHeader title="Pick a voice">Click one to hear it. You can change it, or download more natural voices, any time in Settings.</StepHeader>
      <div className="grid max-h-[340px] grid-cols-2 gap-2 overflow-y-auto pr-1">
        {voices.map((v) => {
          const active = v.name === c.voice;
          return (
            <button
              key={v.name}
              onClick={() => pick(v)}
              className={cn(
                "flex items-center gap-3 rounded-xl border bg-card px-3.5 py-3 text-left shadow-lift transition-colors",
                active ? "border-primary ring-2 ring-primary/30" : "hover:border-foreground/20",
              )}
            >
              <span className={cn("grid size-8 shrink-0 place-items-center rounded-full", active ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground")}>
                {active ? <Check className="size-4" /> : <Play className="size-3.5" />}
              </span>
              <span className="min-w-0">
                <span className="block truncate text-[13px] font-medium">{v.name.split(" (")[0]}</span>
                <span className="block truncate text-xs text-muted-foreground">{languageName(v.locale)}</span>
              </span>
            </button>
          );
        })}
      </div>
      <Nav onBack={onBack} onNext={onNext} />
    </>
  );
}

function AskIt({ app, keyLabel, name, onBack, onNext }: { app: ReturnType<typeof useAppState>; keyLabel: string; name: string; onBack: () => void; onNext: () => void }) {
  const { state } = app;
  const [reply, setReply] = useState<Reply | null>(null);
  const [draft, setDraft] = useState("");
  useEffect(() => {
    const un = events.reply(setReply);
    const unErr = events.replyError((m) => toast.error(m));
    return () => {
      un.then((u) => u());
      unErr.then((u) => u());
    };
  }, []);
  const worked = reply?.state === "done";
  const notReady = state && (!state.model_loaded || !!state.assistant?.unavailable);
  const ask = async () => {
    const q = draft.trim();
    if (!q) return;
    setDraft("");
    setReply({ state: "thinking", question: q, reply: "" });
    await api.ask(q).catch((e) => toast.error(String(e)));
  };
  return (
    <>
      <StepHeader title="Ask your first question">
        Hold <Kbd>{keyLabel}</Kbd> anywhere, say “What can you do?”, then let go. Or type it below.
      </StepHeader>
      <Panel className="overflow-hidden">
        <div className="min-h-28 px-5 py-4">
          {reply ? (
            <>
              <p className="text-[13px] text-muted-foreground">“{reply.question}”</p>
              <p className="mt-1.5 text-[15px] leading-relaxed whitespace-pre-wrap">{reply.state === "thinking" ? <span className="animate-pulse text-muted-foreground">Thinking…</span> : reply.reply}</p>
              {worked && (
                <p className="mt-3 flex items-center gap-1.5 text-xs font-medium text-emerald-600 dark:text-emerald-400">
                  <Check className="size-3.5" /> It works
                </p>
              )}
            </>
          ) : (
            <p className="text-sm text-muted-foreground">{notReady ? "Waiting for the models to be ready…" : `Waiting for your question — ${name} will answer out loud.`}</p>
          )}
        </div>
        <div className="flex gap-2 border-t px-3 py-3">
          <Input value={draft} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => e.key === "Enter" && ask()} placeholder="Or type a question…" className="border-0 shadow-none focus-visible:ring-0" />
          <Button size="icon" onClick={ask} disabled={!draft.trim()} aria-label="Ask">
            <ArrowRight />
          </Button>
        </div>
      </Panel>
      <Nav onBack={onBack} onNext={onNext} quiet={!worked} nextLabel={worked ? "Continue" : "Skip"} />
    </>
  );
}

