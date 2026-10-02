import { useEffect, useState } from "react";
import { ThemeProvider } from "next-themes";
import { BarChart3, History as HistoryIcon, Home as HomeIcon, Mic, Settings as SettingsIcon } from "lucide-react";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Kbd } from "@/components/bits";
import { events } from "@/lib/api";
import { useAppState } from "@/hooks/use-app";
import { cn } from "@/lib/utils";
import Home from "@/pages/Home";
import Insights from "@/pages/Insights";
import History from "@/pages/History";
import Settings from "@/pages/Settings";

export type Tab = "home" | "insights" | "history" | "settings";
const TABS: { id: Tab; label: string; icon: typeof HomeIcon }[] = [
  { id: "home", label: "Home", icon: HomeIcon },
  { id: "insights", label: "Insights", icon: BarChart3 },
  { id: "history", label: "History", icon: HistoryIcon },
  { id: "settings", label: "Settings", icon: SettingsIcon },
];

const isTab = (t: string): t is Tab => TABS.some((x) => x.id === t);
const tabFromHash = (): Tab => {
  const t = location.hash.slice(1);
  return isTab(t) ? t : "home";
};

function statusTone(status: string) {
  if (status.startsWith("Ready")) return "bg-emerald-500 shadow-[0_0_0_3px] shadow-emerald-500/20";
  if (/Allow|Grant|failed/i.test(status)) return "bg-amber-500 shadow-[0_0_0_3px] shadow-amber-500/20";
  return "bg-primary animate-pulse";
}

export default function App() {
  const [tab, setTab] = useState<Tab>(tabFromHash);
  const app = useAppState();

  useEffect(() => {
    history.replaceState(null, "", `#${tab}`);
    document.querySelector("main")?.scrollTo({ top: 0 });
  }, [tab]);

  useEffect(() => {
    const un = events.navigate((t) => setTab(isTab(t) ? t : "home"));
    // ⌘1–⌘4 switch pages, like most Mac apps.
    const onKey = (e: KeyboardEvent) => {
      const n = Number(e.key);
      if (e.metaKey && n >= 1 && n <= TABS.length) {
        e.preventDefault();
        setTab(TABS[n - 1].id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      un.then((u) => u());
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  const status = app.state?.status ?? "Starting…";
  const key = app.state?.hotkeys.find((h) => h.id === app.state?.config.hotkey)?.label.split(" ")[0] ?? "fn";

  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
      <TooltipProvider delayDuration={300}>
        <div className="grid h-screen grid-cols-[232px_1fr] overflow-hidden">
          {/* Translucent in the app (native vibrancy shows through); tinted in a browser. */}
          <aside className="flex flex-col border-r border-sidebar-border bg-sidebar px-3 pb-3 [html.tauri_&]:bg-transparent">
            <div data-tauri-drag-region className="h-11 shrink-0" />
            <div data-tauri-drag-region className="flex items-center gap-3 px-2 pt-1 pb-7">
              <div className="grid size-9 place-items-center rounded-[10px] bg-gradient-to-br from-violet-500 via-fuchsia-500 to-orange-400 shadow-md shadow-violet-500/25">
                <Mic className="size-[18px] text-white" strokeWidth={2.25} />
              </div>
              <div className="font-display text-[26px] leading-none tracking-tight">Murmur</div>
            </div>

            <nav className="flex flex-col gap-0.5">
              {TABS.map(({ id, label, icon: Icon }, i) => (
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
                  <span className="flex-1 text-left">{label}</span>
                  <span className="text-[11px] text-muted-foreground/0 transition-colors group-hover:text-muted-foreground/80">⌘{i + 1}</span>
                </button>
              ))}
            </nav>

            <div className="mt-auto rounded-xl border border-sidebar-border bg-card/70 p-3 shadow-hairline backdrop-blur">
              <div className="flex items-start gap-2.5">
                <span className={cn("mt-[5px] size-2 shrink-0 rounded-full", statusTone(status))} />
                <span className="text-xs leading-snug font-medium text-foreground/85">{status.startsWith("Ready") ? "Ready to listen" : status}</span>
              </div>
              <div className="mt-2.5 grid grid-cols-[auto_1fr] items-center gap-x-2 gap-y-1.5 border-t border-sidebar-border pt-2.5 text-[11px] text-muted-foreground">
                <span className="flex"><Kbd>{key}</Kbd></span> <span>hold to talk</span>
                <span className="flex gap-0.5"><Kbd>{key}</Kbd><Kbd>{key}</Kbd></span> <span>hands-free</span>
              </div>
            </div>
          </aside>

          <main className="overflow-y-auto bg-background">
            <div data-tauri-drag-region className="sticky top-0 z-10 h-10 bg-gradient-to-b from-background via-background/80 to-transparent" />
            <div className="mx-auto max-w-[880px] px-10 pb-16" key={tab}>
              <div className="animate-in fade-in-0 slide-in-from-bottom-1 duration-300">
                {tab === "home" && <Home app={app} goTo={setTab} />}
                {tab === "insights" && <Insights />}
                {tab === "history" && <History />}
                {tab === "settings" && <Settings app={app} />}
              </div>
            </div>
          </main>
        </div>
        <Toaster position="bottom-center" />
      </TooltipProvider>
    </ThemeProvider>
  );
}
