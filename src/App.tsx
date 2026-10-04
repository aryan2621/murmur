import { useEffect, useRef, useState } from "react";
import { ThemeProvider } from "next-themes";
import { toast } from "sonner";
import { BarChart3, History as HistoryIcon, Home as HomeIcon, Mic, Settings as SettingsIcon, Sparkles } from "lucide-react";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Kbd } from "@/components/bits";
import { api, events, keyLabel } from "@/lib/api";
import { useAppState } from "@/hooks/use-app";
import { cn, focusedField, insertAtCursor } from "@/lib/utils";
import Home from "@/pages/Home";
import Assistant from "@/assistant/Assistant";
import Insights from "@/pages/Insights";
import History from "@/pages/History";
import SettingsDialog, { type Pane } from "@/pages/Settings";
import Setup from "@/pages/Setup";

/** Pages, plus "settings", which opens the Settings dialog over the current page. */
export type Tab = "home" | "assistant" | "insights" | "history" | "settings";
type Page = Exclude<Tab, "settings">;
const TABS: { id: Page; label: string; icon: typeof HomeIcon }[] = [
  { id: "home", label: "Home", icon: HomeIcon },
  { id: "assistant", label: "Assistant", icon: Sparkles },
  { id: "insights", label: "Insights", icon: BarChart3 },
  { id: "history", label: "History", icon: HistoryIcon },
];

const isTab = (t: string): t is Page => TABS.some((x) => x.id === t);
const tabFromHash = (): Page => {
  const t = location.hash.slice(1);
  return isTab(t) ? t : "home";
};

function statusTone(status: string) {
  if (status.startsWith("Ready") || status.startsWith("In conversation")) return "bg-emerald-500 shadow-[0_0_0_3px] shadow-emerald-500/20";
  if (/Allow|Grant|failed|isn't|Ollama|Download/i.test(status)) return "bg-amber-500 shadow-[0_0_0_3px] shadow-amber-500/20";
  return "bg-primary animate-pulse";
}

export default function App() {
  const [tab, setTab] = useState<Page>(tabFromHash);
  const [settingsOpen, setSettingsOpen] = useState(location.hash === "#settings");
  const [pane, setPane] = useState<Pane>("shortcuts");
  const goTo = (t: Tab) => (t === "settings" ? setSettingsOpen(true) : setTab(t));
  const app = useAppState();
  /** Setup opened again from Settings. */
  const [rerunSetup, setRerunSetup] = useState(false);
  // No assistant on Windows: its tab is hidden there.
  const hasAssistant = app.state ? app.state.assistant !== null : true;
  const tabs = hasAssistant ? TABS : TABS.filter((t) => t.id !== "assistant");
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;

  useEffect(() => {
    history.replaceState(null, "", `#${tab}`);
    document.querySelector("main")?.scrollTo({ top: 0 });
  }, [tab]);

  useEffect(() => {
    const un = events.navigate((t) => (t === "settings" ? setSettingsOpen(true) : setTab(isTab(t) ? t : "home")));
    // ⌘1–⌘5 switch pages and ⌘, opens Settings, like most Mac apps.
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
        return;
      }
      const n = Number(e.key);
      if (e.metaKey && n >= 1 && n <= tabsRef.current.length) {
        e.preventDefault();
        setTab(tabsRef.current[n - 1].id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      un.then((u) => u());
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  const status = app.state?.status ?? "Starting…";
  const inSetup = !!app.state && (!app.state.config.setup_done || rerunSetup);

  // Dictation while Murmur is in front lands on Home's scratchpad or Setup's test box; on the
  // other pages it goes into the focused field, or onto the clipboard so it is never lost.
  const handledByPage = useRef(false);
  handledByPage.current = inSetup || tab === "home";
  useEffect(() => {
    const un = events.dictation((text) => {
      if (handledByPage.current) return;
      const el = focusedField();
      if (el) return insertAtCursor(el, text);
      api.copy(text).then(
        () => toast.success("Dictation copied — press ⌘V"),
        () => toast.error("Could not copy the dictation"),
      );
    });
    return () => {
      un.then((u) => u());
    };
  }, []);
  const endSetup = () => {
    app.setState((s) => (s ? { ...s, config: { ...s.config, setup_done: true } } : s));
    setRerunSetup(false);
    setTab("home");
  };
  const key = app.state ? keyLabel(app.state, app.state.config.hotkey) : "fn";
  const askKey = app.state ? keyLabel(app.state, app.state.config.assistant_hotkey) : "⌥";
  const assistantOn = hasAssistant && !!app.state?.config.assistant_enabled;
  const name = app.state?.config.assistant_name || "Jarvis";

  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
      <TooltipProvider delayDuration={300}>
        {inSetup ? (
          <Setup app={app} onDone={endSetup} />
        ) : (
          <div className="grid h-screen grid-cols-[232px_1fr] overflow-hidden">
            {/* Translucent in the app (native vibrancy shows through); tinted in a browser. */}
            <aside className="flex flex-col border-r border-sidebar-border bg-sidebar px-3 pb-3 [html.tauri_&]:bg-transparent">
              <div data-tauri-drag-region className="h-11 shrink-0" />
              <div data-tauri-drag-region className="flex items-center gap-3 px-2 pt-1 pb-7">
                <div className="grid size-9 place-items-center rounded-[10px] bg-brand shadow-md shadow-brand/25">
                  <Mic className="size-[18px] text-brand-foreground" strokeWidth={2.25} />
                </div>
                <div className="font-display text-[26px] leading-none tracking-tight">Murmur</div>
              </div>

              <nav className="flex flex-col gap-0.5">
                {tabs.map(({ id, label, icon: Icon }, i) => (
                  <button
                    key={id}
                    onClick={() => setTab(id)}
                    className={cn(
                      "group flex items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-[13px] transition-colors duration-150",
                      tab === id
                        ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground"
                        : "text-sidebar-foreground/75 hover:bg-black/[0.04] hover:text-sidebar-foreground dark:hover:bg-white/[0.05]",
                    )}
                  >
                    <Icon className="size-[17px]" strokeWidth={tab === id ? 2.25 : 1.75} />
                    <span className="flex-1 text-left">{id === "assistant" ? name : label}</span>
                    <span className="text-[11px] text-muted-foreground/0 transition-colors group-hover:text-muted-foreground/80">⌘{i + 1}</span>
                  </button>
                ))}
              </nav>

              <button
                onClick={() => setSettingsOpen(true)}
                className="group mt-auto mb-2 flex items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-[13px] text-sidebar-foreground/75 transition-colors duration-150 hover:bg-black/[0.04] hover:text-sidebar-foreground dark:hover:bg-white/[0.05]"
              >
                <SettingsIcon className="size-[17px]" strokeWidth={1.75} />
                <span className="flex-1 text-left">Settings</span>
                <span className="text-[11px] text-muted-foreground/0 transition-colors group-hover:text-muted-foreground/80">⌘,</span>
              </button>

              <div className="rounded-xl border border-sidebar-border bg-card/70 p-3 shadow-hairline backdrop-blur">
                <div className="flex items-start gap-2.5">
                  <span className={cn("mt-[5px] size-2 shrink-0 rounded-full", statusTone(status))} />
                  <span className="text-xs leading-snug font-medium text-foreground/85">{status.startsWith("Ready") ? "Ready to listen" : status}</span>
                </div>
                <div className="mt-2.5 grid grid-cols-[auto_1fr] items-center gap-x-2 gap-y-1.5 border-t border-sidebar-border pt-2.5 text-[11px] text-muted-foreground">
                  <span className="flex"><Kbd>{key}</Kbd></span> <span>hold to dictate</span>
                  <span className="flex gap-0.5"><Kbd>{key}</Kbd><Kbd>{key}</Kbd></span> <span>hands-free</span>
                  {assistantOn && (
                    <>
                      <span className="flex"><Kbd>{askKey}</Kbd></span> <span>hold to ask {name}</span>
                      <span className="flex"><Kbd>tap {askKey}</Kbd></span> <span>conversation</span>
                    </>
                  )}
                </div>
              </div>
            </aside>

            <main className="overflow-y-auto bg-background">
              <div data-tauri-drag-region className="sticky top-0 z-10 h-10 bg-gradient-to-b from-background via-background/80 to-transparent" />
              <div className="mx-auto max-w-[880px] px-10 pb-16" key={tab}>
                <div className="animate-in fade-in-0 slide-in-from-bottom-1 duration-300">
                  {tab === "home" && <Home app={app} goTo={goTo} />}
                  {tab === "assistant" && hasAssistant && <Assistant app={app} goTo={goTo} />}
                  {tab === "insights" && <Insights assistantName={hasAssistant ? name : null} />}
                  {tab === "history" && <History assistantName={hasAssistant ? name : null} />}
                </div>
              </div>
            </main>
          </div>
        )}
        {!inSetup && (
          <SettingsDialog
            app={app}
            open={settingsOpen}
            onOpenChange={setSettingsOpen}
            pane={pane}
            onPaneChange={setPane}
            onRunSetup={() => {
              setSettingsOpen(false);
              setRerunSetup(true);
            }}
          />
        )}
        <Toaster position="bottom-center" />
      </TooltipProvider>
    </ThemeProvider>
  );
}
