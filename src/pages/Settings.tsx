import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowRight, AudioLines, Brain, FolderOpen, Info, Keyboard, MessagesSquare, Play, Plus, RefreshCw, RotateCcw, ShieldCheck, Sparkles, Type, Volume2, X, Zap, type LucideIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Panel } from "@/components/bits";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { api, type Config, type Replacement, type Voice } from "@/lib/api";
import type { useAppState } from "@/hooks/use-app";

const LANGUAGES: [string, string][] = [
  ["en", "English"], ["auto", "Detect automatically"], ["hi", "Hindi"], ["bn", "Bengali"], ["mr", "Marathi"],
  ["ta", "Tamil"], ["te", "Telugu"], ["gu", "Gujarati"], ["ur", "Urdu"], ["es", "Spanish"], ["fr", "French"],
  ["de", "German"], ["pt", "Portuguese"], ["it", "Italian"], ["nl", "Dutch"], ["ru", "Russian"],
  ["ar", "Arabic"], ["tr", "Turkish"], ["ja", "Japanese"], ["ko", "Korean"], ["zh", "Chinese"],
];
const SYSTEM_DEFAULT = "__default__";

const RATES: [number, string][] = [[160, "Slow"], [195, "Normal"], [230, "Fast"], [265, "Very fast"]];
const PAUSES = [1, 1.5, 2, 2.5, 3];
const TIMEOUTS: [number, string][] = [[10, "10 seconds"], [20, "20 seconds"], [30, "30 seconds"], [60, "1 minute"]];
const SENSITIVITY: [number, string][] = [[0.008, "High — quiet voice"], [0.012, "Normal"], [0.02, "Low — noisy room"], [0.03, "Very low — very noisy"]];
const FORGET: [number, string][] = [[2, "2 minutes"], [5, "5 minutes"], [15, "15 minutes"], [30, "30 minutes"], [60, "1 hour"]];
const KEEP_ALIVE: [string, string][] = [["5m", "5 minutes"], ["30m", "30 minutes"], ["1h", "1 hour"], ["-1", "Always"]];

function Row({ id, title, description, children, stack }: { id?: string; title: string; description: ReactNode; children: ReactNode; stack?: boolean }) {
  return (
    <div className={stack ? "flex flex-col gap-3 px-5 py-4 not-last:border-b" : "flex min-h-16 items-center gap-6 px-5 py-3.5 not-last:border-b"}>
      <div className="min-w-0 flex-1">
        <Label htmlFor={id} className="text-[13px] font-medium">{title}</Label>
        <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{description}</p>
      </div>
      {children}
    </div>
  );
}

function Section({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="mb-6">
      {title && <h3 className="mb-2 px-1 text-[13px] font-semibold tracking-tight">{title}</h3>}
      <Panel className="overflow-hidden">{children}</Panel>
    </section>
  );
}

/** A select over fixed choices that still shows a hand-edited value from config.json. */
function Choice<T extends string | number>({ id, value, options, onChange, className, format }: { id: string; value: T; options: [T, string][]; onChange: (v: T) => void; className: string; format?: (v: T) => string }) {
  const all = options.some(([v]) => v === value) ? options : [...options, [value, format ? format(value) : String(value)] as [T, string]];
  return (
    <Select value={String(value)} onValueChange={(v) => onChange((typeof value === "number" ? Number(v) : v) as T)}>
      <SelectTrigger id={id} className={className}><SelectValue /></SelectTrigger>
      <SelectContent>{all.map(([v, l]) => <SelectItem key={String(v)} value={String(v)}>{l}</SelectItem>)}</SelectContent>
    </Select>
  );
}

const languageName = (locale: string) => {
  try {
    return new Intl.DisplayNames([], { type: "language" }).of(locale.replace("_", "-")) ?? locale;
  } catch {
    return locale;
  }
};

/** Voices grouped by language, English first. */
function groupVoices(voices: Voice[]) {
  const groups = new Map<string, Voice[]>();
  for (const v of voices) {
    const lang = languageName(v.locale);
    groups.set(lang, [...(groups.get(lang) ?? []), v]);
  }
  return [...groups.entries()].sort(([a], [b]) => Number(b.startsWith("English")) - Number(a.startsWith("English")) || a.localeCompare(b));
}

const formatSize = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`);

export type Pane = "shortcuts" | "speech" | "text" | "brain" | "actions" | "voice" | "conversation" | "behaviour" | "privacy" | "about";

type NavItem = { id: Pane; label: string; description: string; icon: LucideIcon };

function navGroups(assistant: string | null): { label: string; items: NavItem[] }[] {
  return [
    {
      label: "General",
      items: [
        { id: "shortcuts", label: "Shortcuts", description: "The keys for dictation and the assistant.", icon: Keyboard },
        { id: "speech", label: "Speech & microphone", description: "How Murmur hears you. Used by dictation and the assistant.", icon: AudioLines },
        { id: "behaviour", label: "Behaviour", description: "Sounds, the clipboard and starting at login.", icon: Sparkles },
        { id: "privacy", label: "Privacy & memory", description: "What is kept on this Mac, and the memory it uses.", icon: ShieldCheck },
      ],
    },
    {
      label: "Dictation",
      items: [{ id: "text", label: "Dictated text", description: "Tidying up what you say before it's typed.", icon: Type }],
    },
    ...(assistant
      ? [
          {
            label: "Assistant",
            items: [
              { id: "brain", label: assistant, description: "The AI that answers, its name and personality.", icon: Brain },
              { id: "actions", label: "Actions & web", description: "What it may do on this Mac and look up online.", icon: Zap },
              { id: "voice", label: "Voice", description: "How answers are spoken.", icon: Volume2 },
              { id: "conversation", label: "Conversation", description: "When it answers and when it stops listening.", icon: MessagesSquare },
            ] satisfies NavItem[],
          },
        ]
      : []),
    { label: "Murmur", items: [{ id: "about", label: "About", description: "Version, data folder and setup.", icon: Info }] },
  ];
}

/** Settings in a dialog, like Claude's: sections on the left, the chosen one on the right. */
export default function SettingsDialog({ app, open, onOpenChange, pane, onPaneChange, onRunSetup }: { app: ReturnType<typeof useAppState>; open: boolean; onOpenChange: (open: boolean) => void; pane: Pane; onPaneChange: (pane: Pane) => void; onRunSetup: () => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[min(640px,calc(100vh-4rem))] w-[min(880px,calc(100vw-3rem))] max-w-none gap-0 overflow-hidden bg-background p-0">
        {open && <SettingsPanel app={app} pane={pane} setPane={onPaneChange} onRunSetup={onRunSetup} />}
      </DialogContent>
    </Dialog>
  );
}

function SettingsPanel({ app, pane, setPane, onRunSetup }: { app: ReturnType<typeof useAppState>; pane: Pane; setPane: (pane: Pane) => void; onRunSetup: () => void }) {
  const { state, setState, refresh } = app;
  const [vocab, setVocab] = useState("");
  const [repl, setRepl] = useState<Replacement[]>([]);
  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [location, setLocation] = useState("");
  const loaded = useRef(false);
  const savedConfig = useRef<Config | null>(null);
  const saveQueue = useRef(Promise.resolve());
  const pendingSaves = useRef(0);

  useEffect(() => {
    if (state && pendingSaves.current === 0) savedConfig.current = state.config;
    if (state && !loaded.current) {
      loaded.current = true;
      setVocab(state.config.vocabulary.join("\n"));
      setRepl(state.config.replacements);
      setName(state.config.assistant_name);
      setPrompt(state.config.system_prompt ?? "");
      setLocation(state.config.location);
    }
  }, [state]);

  const voiceGroups = useMemo(() => groupVoices(state?.assistant?.voices ?? []), [state?.assistant?.voices]);

  if (!state) return null;
  const c = state.config;
  const a = state.assistant;
  const model = state.models.find((m) => m.id === c.model);
  const llmModels = !a ? [] : a.ollama.models.includes(c.llm_model) ? a.ollama.models : [c.llm_model, ...a.ollama.models];

  async function save(patch: Partial<Config>) {
    savedConfig.current ??= state!.config;
    ++pendingSaves.current;
    setState((s) => (s ? { ...s, config: { ...s.config, ...patch } } : s));
    // Persist patches in order, starting from the last successful save. Rapid changes
    // must not overwrite each other with snapshots from an older render.
    saveQueue.current = saveQueue.current.then(async () => {
      const config = { ...savedConfig.current!, ...patch };
      try {
        const res = await api.saveConfig(config);
        savedConfig.current = config;
        if (res.restarting) toast("Restarting Murmur with the new keys…");
        else if (res.reloading_model) {
          toast("Loading model — progress shows in the sidebar");
          setTimeout(refresh, 800);
        }
      } catch (e) {
        toast.error(`Couldn't save: ${e}`);
      } finally {
        if (--pendingSaves.current === 0) {
          setState((s) => (s ? { ...s, config: savedConfig.current! } : s));
        }
      }
    });
    await saveQueue.current;
  }

  const saveRepl = (list: Replacement[]) => save({ replacements: list.filter((r) => r.from.trim()) });


  const off = (
    <Section>
      <Row id="assistant-off" title={`${c.assistant_name} is turned off`} description="Turn the assistant on to set its AI, voice and conversation.">
        <Switch id="assistant-off" checked={false} onCheckedChange={(v) => save({ assistant_enabled: v })} />
      </Row>
    </Section>
  );
  const panes: Record<Pane, ReactNode> = {
    shortcuts: (
          <Section>
            <Row id="hotkey" title="Dictation key" description="Hold to talk, double-tap for hands-free. Changing a key restarts Murmur.">
              <Select value={c.hotkey} onValueChange={(v) => save({ hotkey: v })}>
                <SelectTrigger id="hotkey" className="w-52"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {state.hotkeys.map((h) => (
                    <SelectItem key={h.id} value={h.id} disabled={!!a && c.assistant_enabled && h.id === c.assistant_hotkey}>{h.label}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Row>
            {a && (
              <Row id="assistant" title={`${c.assistant_name}, the assistant`} description="Ask out loud and hear the answer; it can also act on this Mac. Off: its key does nothing and no AI is loaded.">
                <Switch id="assistant" checked={c.assistant_enabled} onCheckedChange={(v) => save({ assistant_enabled: v })} />
              </Row>
            )}
            {a && c.assistant_enabled && (
              <Row id="ask-key" title="Assistant key" description="Hold for a quick question, tap for a conversation.">
                <Select value={c.assistant_hotkey} onValueChange={(v) => save({ assistant_hotkey: v })}>
                  <SelectTrigger id="ask-key" className="w-52"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {state.hotkeys.map((h) => (
                      <SelectItem key={h.id} value={h.id} disabled={h.id === c.hotkey}>{h.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Row>
            )}
          </Section>
    ),
    speech: (
      <>
          <Section title="Speech">
            <Row
              id="model"
              title="Model"
              description={<>{model?.note} {model && !model.downloaded && `Downloads ${formatSize(model.size_mb)} once.`}</>}
            >
              <Select value={c.model} onValueChange={(v) => save({ model: v })}>
                <SelectTrigger id="model" className="w-72"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {state.models.map((m) => (
                    <SelectItem key={m.id} value={m.id}>
                      <span>{m.label}</span>
                      <span className="text-muted-foreground">{formatSize(m.size_mb)}</span>
                      {!m.downloaded && <Badge variant="outline" className="ml-1">download</Badge>}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Row>
            <Row id="language" title="Language" description="What you speak, to dictation and the assistant. “Detect automatically” helps with mixed languages.">
              <Select value={c.language} onValueChange={(v) => save({ language: v })}>
                <SelectTrigger id="language" className="w-52"><SelectValue /></SelectTrigger>
                <SelectContent>{LANGUAGES.map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
              </Select>
            </Row>
            <Row id="translate" title="Translate to English" description="Speak any language, get English text. Needs Large v3 or Small (multilingual).">
              <Switch id="translate" checked={c.translate} onCheckedChange={(v) => save({ translate: v })} />
            </Row>
          </Section>
          <Section title="Microphone">
            <Row id="mic" title="Input device" description="Built-in mics often sound better than Bluetooth headsets.">
              <div className="flex items-center gap-1">
                <Select value={c.input_device ?? SYSTEM_DEFAULT} onValueChange={(v) => save({ input_device: v === SYSTEM_DEFAULT ? null : v })}>
                  <SelectTrigger id="mic" className="w-60"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value={SYSTEM_DEFAULT}>System default</SelectItem>
                    {state.devices.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                    {c.input_device && !state.devices.includes(c.input_device) && (
                      <SelectItem value={c.input_device}>{c.input_device} (not connected)</SelectItem>
                    )}
                  </SelectContent>
                </Select>
                <Button variant="ghost" size="icon" aria-label="Refresh devices" onClick={() => refresh().then(() => toast("Devices refreshed", { duration: 1200 }))}>
                  <RefreshCw />
                </Button>
              </div>
            </Row>
          </Section>
      </>
    ),
    text: (
          <Section>
            <Row id="fillers" title="Remove filler words" description="Drops “um”, “uh”, stutters and set-off “you know”, “I mean”. Writes times as 10:30.">
              <Switch id="fillers" checked={c.remove_fillers} onCheckedChange={(v) => save({ remove_fillers: v })} />
            </Row>
            <Row stack id="vocab" title="Vocabulary" description="Names, products and jargon to spell correctly. One per line.">
              <Textarea
                id="vocab"
                rows={3}
                value={vocab}
                placeholder={"Acme Corp\nKubernetes"}
                onChange={(e) => setVocab(e.target.value)}
                onBlur={() => save({ vocabulary: vocab.split(/[\n,]/).map((s) => s.trim()).filter(Boolean) })}
              />
            </Row>
            <Row stack title="Replacements" description="Fix words Murmur keeps getting wrong. Whole words, any case.">
              <div className="flex flex-col gap-2">
                {repl.map((r, i) => (
                  <div key={i} className="flex items-center gap-2">
                    <Input
                      value={r.from}
                      placeholder="Heard as…"
                      onChange={(e) => setRepl(repl.map((x, j) => (j === i ? { ...x, from: e.target.value } : x)))}
                      onBlur={() => saveRepl(repl)}
                    />
                    <ArrowRight className="size-4 shrink-0 text-muted-foreground" />
                    <Input
                      value={r.to}
                      placeholder="Replace with…"
                      onChange={(e) => setRepl(repl.map((x, j) => (j === i ? { ...x, to: e.target.value } : x)))}
                      onBlur={() => saveRepl(repl)}
                    />
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label="Remove replacement"
                      onClick={() => {
                        const next = repl.filter((_, j) => j !== i);
                        setRepl(next);
                        saveRepl(next);
                      }}
                    >
                      <X />
                    </Button>
                  </div>
                ))}
                <div>
                  <Button variant="outline" size="sm" onClick={() => setRepl([...repl, { from: "", to: "" }])}>
                    <Plus /> Add replacement
                  </Button>
                </div>
              </div>
            </Row>
          </Section>
    ),
    brain: !a || !c.assistant_enabled ? off : (
          <Section>
            <Row
              id="brain"
              title="AI"
              description={
                c.brain === "builtin"
                  ? `${a.brain_label}, built into Murmur. ${a.brain_ready ? "Downloaded and ready." : "Not downloaded yet."}`
                  : "Use a model from your own Ollama instead (advanced)."
              }
            >
              <Choice
                id="brain"
                className="w-44"
                value={c.brain}
                options={[
                  ["builtin", "Built-in"],
                  ["ollama", "Ollama"],
                ]}
                onChange={(v) => save({ brain: v })}
              />
            </Row>
            {c.brain === "builtin" && (
            <Row
              id="builtin"
              title="Model"
              description={`${a.brains.find((b) => b.id === c.builtin_model)?.note ?? ""} This Mac has ${a.ram_gb} GB of memory.`}
            >
              <div className="flex items-center gap-2">
                <Select value={c.builtin_model} onValueChange={(v) => save({ builtin_model: v })}>
                  <SelectTrigger id="builtin" className="w-56"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {a.brains.map((b) => (
                      <SelectItem key={b.id} value={b.id}>
                        {b.label} · {formatSize(b.size_mb)}
                        {b.min_ram_gb > a.ram_gb && <Badge variant="outline" className="ml-1">needs {b.min_ram_gb} GB</Badge>}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {!a.brain_ready && (
                  <Button size="sm" variant="outline" disabled={state.downloads.brain != null} onClick={() => api.downloadBrain()}>
                    {state.downloads.brain != null ? `${Math.round((state.downloads.brain ?? 0) * 100)}%` : "Download"}
                  </Button>
                )}
              </div>
            </Row>
            )}
            {c.brain === "ollama" && (
            <Row
              id="llm"
              title="Ollama model"
              description={a.ollama.running ? "Models installed in Ollama on this Mac. Bigger models are smarter but slower." : "Ollama isn't running, so installed models can't be listed."}
            >
              <div className="flex items-center gap-1">
                <Select value={c.llm_model} onValueChange={(v) => save({ llm_model: v })}>
                  <SelectTrigger id="llm" className="w-56"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {llmModels.map((m) => (
                      <SelectItem key={m} value={m}>
                        {m}
                        {!a.ollama.models.includes(m) && <Badge variant="outline" className="ml-1">not installed</Badge>}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button variant="ghost" size="icon" aria-label="Refresh models" onClick={() => refresh().then(() => toast("Models refreshed", { duration: 1200 }))}>
                  <RefreshCw />
                </Button>
              </div>
            </Row>
            )}
            <Row id="name" title="Name" description="What the assistant calls itself.">
              <Input id="name" className="w-56" value={name} onChange={(e) => setName(e.target.value)} onBlur={() => name.trim() && name !== c.assistant_name && save({ assistant_name: name.trim() })} />
            </Row>
            <Row stack id="prompt" title="Personality" description="Your own instructions for how it should answer. Leave empty for the built-in one: short, spoken, no lists.">
              <Textarea
                id="prompt"
                rows={4}
                value={prompt}
                placeholder={`You are ${c.assistant_name}, a friendly assistant. Answer in one or two short sentences…`}
                onChange={(e) => setPrompt(e.target.value)}
                onBlur={() => (prompt.trim() || null) !== c.system_prompt && save({ system_prompt: prompt.trim() || null })}
              />
            </Row>
            <Row id="forget" title="Memory" description="Follow-up questions remember the conversation until it goes quiet for this long.">
              <Choice id="forget" className="w-44" value={c.forget_after_minutes} options={FORGET} format={(v) => `${v} minutes`} onChange={(v) => save({ forget_after_minutes: v })} />
            </Row>
            <Row id="keep" title="Keep model loaded" description="Loading takes a few seconds; while loaded the 8B uses about 6 GB of memory (the 4B about 3 GB).">
              <Choice id="keep" className="w-44" value={c.keep_alive} options={KEEP_ALIVE} onChange={(v) => save({ keep_alive: v })} />
            </Row>
          </Section>
    ),
    actions: !a || !c.assistant_enabled ? off : (
          <Section>
            <Row
              id="actions"
              title="Act on this Mac"
              description="Open apps and websites, search sites, manage browser tabs, find and open files, set timers and reminders, read your calendar, change the volume and control music. macOS asks once before it can use each app. Opening an archive or installer is always asked about first."
            >
              <Switch id="actions" checked={c.actions} onCheckedChange={(v) => save({ actions: v })} />
            </Row>
            <Row
              id="web"
              title="Look things up online"
              description="Web search, reading pages and the weather. Only your search words and page addresses leave this Mac; your voice and conversation never do."
            >
              <Switch id="web" checked={c.web_access} onCheckedChange={(v) => save({ web_access: v })} />
            </Row>
            {c.web_access && (
              <Row id="location" title="Your city" description="For “what's the weather?”. Leave empty to have it guessed from your internet connection.">
                <Input
                  id="location"
                  className="w-56"
                  value={location}
                  placeholder="e.g. Pune"
                  onChange={(e) => setLocation(e.target.value)}
                  onBlur={() => location.trim() !== c.location && save({ location: location.trim() })}
                />
              </Row>
            )}
          </Section>
    ),
    voice: !a || !c.assistant_enabled ? off : (
          <Section>
            <Row
              id="voice"
              title="Voice"
              description={
                <>
                  Built-in macOS voices. ▶ plays a sample.{" "}
                  <button className="text-primary underline-offset-2 hover:underline" onClick={() => api.openVoiceSettings()}>
                    Get more natural Premium voices
                  </button>
                </>
              }
            >
              <div className="flex items-center gap-1">
                <Select value={c.voice} onValueChange={(v) => save({ voice: v })}>
                  <SelectTrigger id="voice" className="w-60"><SelectValue /></SelectTrigger>
                  <SelectContent className="max-h-80">
                    {voiceGroups.map(([lang, list]) => (
                      <SelectGroup key={lang}>
                        <SelectLabel>{lang}</SelectLabel>
                        {list.map((v) => <SelectItem key={v.name} value={v.name}>{v.name}</SelectItem>)}
                      </SelectGroup>
                    ))}
                    {!a.voices.some((v) => v.name === c.voice) && <SelectItem value={c.voice}>{c.voice}</SelectItem>}
                  </SelectContent>
                </Select>
                <Button variant="ghost" size="icon" aria-label="Play sample" onClick={() => api.previewVoice(c.voice, c.speech_rate)}>
                  <Play />
                </Button>
              </div>
            </Row>
            <Row id="rate" title="Speed" description="How fast answers are spoken.">
              <Choice id="rate" className="w-44" value={c.speech_rate} options={RATES} format={(v) => `${v} wpm`} onChange={(v) => save({ speech_rate: v })} />
            </Row>
            <Row id="speak" title="Speak answers" description="Off: answers only appear on screen.">
              <Switch id="speak" checked={c.speak_replies} onCheckedChange={(v) => save({ speak_replies: v })} />
            </Row>
          </Section>
    ),
    conversation: !a || !c.assistant_enabled ? off : (
          <Section>
            <Row id="pause" title="Pause before answering" description="Tap mode: how long you stay quiet before it answers. Longer gives you time to think mid-sentence.">
              <Choice id="pause" className="w-44" value={c.pause_seconds} options={PAUSES.map((p) => [p, `${p} seconds`] as [number, string])} format={(v) => `${v} seconds`} onChange={(v) => save({ pause_seconds: v })} />
            </Row>
            <Row id="timeout" title="End after silence" description="The conversation ends if you don't say anything for this long.">
              <Choice id="timeout" className="w-44" value={c.conversation_timeout_seconds} options={TIMEOUTS} format={(v) => `${v} seconds`} onChange={(v) => save({ conversation_timeout_seconds: v })} />
            </Row>
            <Row id="threshold" title="Mic sensitivity" description="If it never notices you've stopped (noisy room), lower it. If it misses a quiet voice, raise it.">
              <Choice id="threshold" className="w-56" value={c.speech_threshold} options={SENSITIVITY} onChange={(v) => save({ speech_threshold: v })} />
            </Row>
          </Section>
    ),
    behaviour: (
          <Section>
            <Row id="sounds" title="Sounds" description="Soft tones when recording starts and stops.">
              <Switch id="sounds" checked={c.sounds} onCheckedChange={(v) => save({ sounds: v })} />
            </Row>
            <Row id="clip" title="Restore clipboard" description="Put back what you had copied (text, images, files) after typing.">
              <Switch id="clip" checked={c.restore_clipboard} onCheckedChange={(v) => save({ restore_clipboard: v })} />
            </Row>
            <Row id="login" title="Start at login" description="Launch Murmur in the menu bar when you log in.">
              <Switch
                id="login"
                checked={state.login_enabled}
                onCheckedChange={async (v) => {
                  const enabled = await api.setLogin(v);
                  setState((s) => (s ? { ...s, login_enabled: enabled } : s));
                }}
              />
            </Row>
          </Section>
    ),
    privacy: (
          <Section>
            <Row id="history" title="Save history" description="Keep a searchable log of dictations and questions on this Mac. Speech, the AI and the voice all run locally.">
              <Switch id="history" checked={c.save_history} onCheckedChange={(v) => save({ save_history: v })} />
            </Row>
            <Row id="unload" title="Free memory when idle" description="Unloads the speech model (~700 MB) after a while; it reloads as you start speaking.">
              <Select value={String(c.unload_after_minutes)} onValueChange={(v) => save({ unload_after_minutes: Number(v) })}>
                <SelectTrigger id="unload" className="w-44"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="0">Never</SelectItem>
                  <SelectItem value="5">After 5 minutes</SelectItem>
                  <SelectItem value="15">After 15 minutes</SelectItem>
                  <SelectItem value="30">After 30 minutes</SelectItem>
                  <SelectItem value="60">After 1 hour</SelectItem>
                </SelectContent>
              </Select>
            </Row>
          </Section>
    ),
    about: (
          <Section>
            <Row title="Setup" description="Walk through permissions, the models and a first try again.">
              <Button variant="outline" size="sm" onClick={onRunSetup}>
                <RotateCcw /> Run setup again
              </Button>
            </Row>
            <Row title={`Murmur ${state.version}`} description={<span data-selectable>{state.data_dir}</span>}>
              <Button variant="outline" size="sm" onClick={() => api.openDataFolder().catch((e) => toast.error(`Couldn't open the folder: ${e}`))}>
                <FolderOpen /> Show data folder
              </Button>
            </Row>
          </Section>
    ),
  };

  const groups = navGroups(a ? c.assistant_name : null);
  const active = groups.flatMap((g) => g.items).find((i) => i.id === pane) ?? groups[0].items[0];

  return (
    <div className="flex min-h-0 flex-1">
      <aside className="flex w-56 shrink-0 flex-col border-r bg-sidebar">
        <div className="px-5 pt-5 pb-3">
          <DialogTitle className="text-xl">Settings</DialogTitle>
        </div>
        <nav className="flex-1 space-y-4 overflow-y-auto px-2.5 pb-4" aria-label="Settings sections">
          {groups.map((g) => (
            <div key={g.label}>
              <p className="mb-1 px-2.5 text-xs font-medium text-muted-foreground">{g.label}</p>
              <ul className="space-y-0.5">
                {g.items.map(({ id, label, icon: Icon }) => (
                  <li key={id}>
                    <button
                      type="button"
                      onClick={() => setPane(id)}
                      className={cn(
                        "flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] transition-colors",
                        id === active.id ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground" : "text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-foreground",
                      )}
                    >
                      <Icon className="size-4 shrink-0 opacity-80" />
                      {label}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </nav>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="shrink-0 px-8 pt-6 pb-4">
          <h2 className="font-display text-2xl">{active.label}</h2>
          <DialogDescription className="mt-1">{active.description}</DialogDescription>
        </header>
        <div key={active.id} className="flex-1 overflow-y-auto px-8 pb-8">
          {panes[active.id]}
        </div>
      </div>
    </div>
  );
}
