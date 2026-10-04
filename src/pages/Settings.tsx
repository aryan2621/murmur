import { useEffect, useRef, useState, type ReactNode } from "react";
import { ArrowRight, AudioLines, FolderOpen, Info, Keyboard, Mic, Plus, RefreshCw, RotateCcw, ShieldCheck, Sparkles, Type, X, type LucideIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { IconTile, PageHeader, Panel } from "@/components/bits";
import { api, type Config, type Replacement } from "@/lib/api";
import type { useAppState } from "@/hooks/use-app";

const LANGUAGES: [string, string][] = [
  ["en", "English"], ["auto", "Detect automatically"], ["hi", "Hindi"], ["bn", "Bengali"], ["mr", "Marathi"],
  ["ta", "Tamil"], ["te", "Telugu"], ["gu", "Gujarati"], ["ur", "Urdu"], ["es", "Spanish"], ["fr", "French"],
  ["de", "German"], ["pt", "Portuguese"], ["it", "Italian"], ["nl", "Dutch"], ["ru", "Russian"],
  ["ar", "Arabic"], ["tr", "Turkish"], ["ja", "Japanese"], ["ko", "Korean"], ["zh", "Chinese"],
];
const SYSTEM_DEFAULT = "__default__";

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

function Section({ icon, tint, title, children }: { icon: LucideIcon; tint: string; title: string; children: ReactNode }) {
  return (
    <section className="mb-7">
      <div className="mb-2.5 flex items-center gap-2.5 px-1">
        <IconTile icon={icon} className={tint} />
        <h2 className="text-[13px] font-semibold tracking-tight">{title}</h2>
      </div>
      <Panel className="overflow-hidden">{children}</Panel>
    </section>
  );
}

const formatSize = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`);

export default function Settings({ app, onRunSetup }: { app: ReturnType<typeof useAppState>; onRunSetup: () => void }) {
  const { state, setState, refresh } = app;
  const [vocab, setVocab] = useState("");
  const [repl, setRepl] = useState<Replacement[]>([]);
  const loaded = useRef(false);

  useEffect(() => {
    if (state && !loaded.current) {
      loaded.current = true;
      setVocab(state.config.vocabulary.join("\n"));
      setRepl(state.config.replacements);
    }
  }, [state]);

  if (!state) return null;
  const c = state.config;
  const model = state.models.find((m) => m.id === c.model);

  async function save(patch: Partial<Config>) {
    const config = { ...state!.config, ...patch };
    setState((s) => (s ? { ...s, config } : s));
    try {
      const res = await api.saveConfig(config);
      if (res.restarting) toast("Restarting Murmur with the new key…");
      else if (res.reloading_model) {
        toast("Loading model — progress shows in the sidebar");
        setTimeout(refresh, 800);
      }
    } catch (e) {
      toast.error(`Couldn't save: ${e}`);
    }
  }

  const saveRepl = (list: Replacement[]) => save({ replacements: list.filter((r) => r.from.trim()) });

  return (
    <>
      <PageHeader title="Settings" description="Changes save automatically and apply right away." />

      <Section icon={Keyboard} tint="bg-violet-500" title="Shortcut">
        <Row id="hotkey" title="Dictation key" description="Hold to talk, double-tap for hands-free. Changing it restarts Murmur.">
          <Select value={c.hotkey} onValueChange={(v) => save({ hotkey: v })}>
            <SelectTrigger id="hotkey" className="w-52"><SelectValue /></SelectTrigger>
            <SelectContent>{state.hotkeys.map((h) => <SelectItem key={h.id} value={h.id}>{h.label}</SelectItem>)}</SelectContent>
          </Select>
        </Row>
      </Section>

      <Section icon={AudioLines} tint="bg-sky-500" title="Speech">
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
        <Row id="language" title="Language" description="What you speak. “Detect automatically” helps with mixed languages.">
          <Select value={c.language} onValueChange={(v) => save({ language: v })}>
            <SelectTrigger id="language" className="w-52"><SelectValue /></SelectTrigger>
            <SelectContent>{LANGUAGES.map(([v, l]) => <SelectItem key={v} value={v}>{l}</SelectItem>)}</SelectContent>
          </Select>
        </Row>
        <Row id="translate" title="Translate to English" description="Speak any language, get English text. Needs Large v3 or Small (multilingual).">
          <Switch id="translate" checked={c.translate} onCheckedChange={(v) => save({ translate: v })} />
        </Row>
      </Section>

      <Section icon={Mic} tint="bg-orange-500" title="Microphone">
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
        <Row
          id="echo"
          title="Echo cancellation"
          description="Removes music and videos playing from this Mac’s speakers from what Murmur hears. Opens the mic about a quarter of a second slower. Works with the System default microphone."
        >
          <Switch id="echo" checked={c.echo_cancellation} onCheckedChange={(v) => save({ echo_cancellation: v })} />
        </Row>
      </Section>

      <Section icon={Type} tint="bg-emerald-500" title="Text">
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

      <Section icon={Sparkles} tint="bg-pink-500" title="Behaviour">
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

      <Section icon={ShieldCheck} tint="bg-teal-600" title="Privacy & memory">
        <Row id="history" title="Save history" description="Keep a searchable log of dictations on this Mac.">
          <Switch id="history" checked={c.save_history} onCheckedChange={(v) => save({ save_history: v })} />
        </Row>
        <Row id="unload" title="Free memory when idle" description="Unloads the model (~700 MB) after a while; it reloads as you start speaking.">
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

      <Section icon={Info} tint="bg-stone-500" title="About">
        <Row title="Setup" description="Walk through permissions, the speech model and a test dictation again.">
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
    </>
  );
}
