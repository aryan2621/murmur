import { useMemo, useState, type ReactNode } from "react";
import { Area, AreaChart, Bar, BarChart, CartesianGrid, XAxis, YAxis } from "recharts";
import { ChartContainer, ChartLegend, ChartLegendContent, ChartTooltip, type ChartConfig } from "@/components/ui/chart";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ChartColumn, Table2 } from "lucide-react";
import { EmptyState, PageHeader, Panel } from "@/components/bits";
import { useExchanges } from "@/assistant/use-exchanges";
import { formatSeconds, type Exchange } from "@/lib/api";
import { DAY, medianResponseMs, questionsPerDay } from "@/assistant/metrics";

type Range = "7" | "30" | "all";

const series: ChartConfig = { value: { label: "Value", color: "var(--chart-1)" } };
// Two parts of one response, stacked: understanding you, then the model starting to answer.
const timing: ChartConfig = {
  heard: { label: "Understanding you", color: "var(--chart-1)" },
  think: { label: "Starting the answer", color: "var(--chart-3)" },
};
const axis = { tickLine: false, axisLine: false, tick: { fill: "var(--muted-foreground)", fontSize: 11 } } as const;
const grid = { stroke: "var(--border)", strokeOpacity: 0.7 } as const;

function useInsights(entries: Exchange[], range: Range) {
  return useMemo(() => {
    const now = Date.now();
    const cutoff = range === "all" ? 0 : now - Number(range) * DAY;
    const list = entries.filter((e) => new Date(e.time).getTime() >= cutoff);

    const oldest = list.length ? Math.min(...list.map((e) => +new Date(e.time))) : now;
    const days = range === "all" ? Math.min(60, Math.max(7, Math.ceil((now - oldest) / DAY) + 1)) : Number(range);
    const perDay = questionsPerDay(list, days, now);

    const byHour = Array.from({ length: 24 }, (_, h) => ({
      label: new Date(2000, 0, 1, h).toLocaleTimeString([], { hour: "numeric" }),
      value: 0,
    }));
    for (const e of list) byHour[new Date(e.time).getHours()].value += 1;
    const busiest = byHour.reduce((a, b) => (b.value > a.value ? b : a), byHour[0]);

    // Last 30 spoken questions, oldest first.
    const recent = list
      .filter((e) => e.mode !== "typed" && e.first_word_ms > 0)
      .slice(0, 30)
      .reverse()
      .map((e) => ({
        label: new Date(e.time).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }),
        heard: +(e.heard_ms / 1000).toFixed(2),
        think: +(e.first_word_ms / 1000).toFixed(2),
      }));

    const conversation = list.filter((e) => e.mode === "conversation").length;
    return {
      count: list.length,
      perDay,
      byHour,
      recent,
      median: medianResponseMs(list),
      conversationShare: list.length ? conversation / list.length : 0,
      busiest: busiest.value ? busiest.label : null,
    };
  }, [entries, range]);
}

/** Chart panel with a Chart/Table switch so every chart has a non-visual view. */
function ChartPanel({ title, description, table, children }: { title: string; description: ReactNode; table: { head: string[]; rows: (string | number)[][] }; children: ReactNode }) {
  const [view, setView] = useState("chart");
  return (
    <Panel>
      <div className="flex items-start justify-between gap-4 px-6 pt-5">
        <div>
          <h3 className="text-[15px] font-semibold tracking-tight">{title}</h3>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{description}</p>
        </div>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon-sm" className="-mt-1 -mr-2 text-muted-foreground" aria-label={view === "chart" ? "Show as table" : "Show as chart"} onClick={() => setView(view === "chart" ? "table" : "chart")}>
              {view === "chart" ? <Table2 /> : <ChartColumn />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{view === "chart" ? "Show data" : "Show chart"}</TooltipContent>
        </Tooltip>
      </div>
      <div className="px-4 pt-4 pb-4">
        {view === "chart" ? (
          children
        ) : (
          <div className="mx-2 max-h-64 overflow-y-auto rounded-lg border">
            <Table>
              <TableHeader>
                <TableRow>{table.head.map((h) => <TableHead key={h}>{h}</TableHead>)}</TableRow>
              </TableHeader>
              <TableBody>
                {table.rows.map((r, i) => (
                  <TableRow key={i}>{r.map((c, j) => <TableCell key={j} className="tabular-nums">{c}</TableCell>)}</TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>
    </Panel>
  );
}

function Tooltip1({ active, payload, render }: { active?: boolean; payload?: any[]; render: (p: any) => ReactNode }) {
  const p = active && payload?.[0]?.payload;
  if (!p) return null;
  return <div className="rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md">{render(p)}</div>;
}

function Hero({ value, label, hint }: { value: string; label: string; hint: string }) {
  return (
    <Panel className="p-5">
      <div className="font-display text-[44px] leading-[0.9] tracking-[-0.02em] tabular-nums">{value}</div>
      <div className="mt-3 text-[13px] font-medium">{label}</div>
      <div className="text-xs text-muted-foreground">{hint}</div>
    </Panel>
  );
}

export default function AssistantInsights() {
  const { entries } = useExchanges("", 100_000);
  const [range, setRange] = useState<Range>("30");
  const d = useInsights(entries ?? [], range);

  return (
    <>
      <PageHeader
        title="Insights"
        description={d.busiest ? `You ask most around ${d.busiest}. Here's what you ask and how fast answers come back.` : "What you ask and how fast answers come back."}
        actions={
          <ToggleGroup type="single" variant="outline" size="sm" value={range} onValueChange={(v) => v && setRange(v as Range)} className="rounded-lg bg-card shadow-lift">
            <ToggleGroupItem value="7" className="px-3">7 days</ToggleGroupItem>
            <ToggleGroupItem value="30" className="px-3">30 days</ToggleGroupItem>
            <ToggleGroupItem value="all" className="px-3">All time</ToggleGroupItem>
          </ToggleGroup>
        }
      />

      {entries && d.count === 0 ? (
        <Panel>
          <EmptyState title="Nothing to chart yet">Ask a few questions in this period and your charts appear here.</EmptyState>
        </Panel>
      ) : (
        <div className="flex flex-col gap-4">
          <div className="grid grid-cols-3 gap-3">
            <Hero value={d.count.toLocaleString()} label="Questions" hint="asked in this period" />
            <Hero value={d.median ? formatSeconds(d.median) : "–"} label="Response time" hint="median, last word → first word" />
            <Hero value={`${Math.round(d.conversationShare * 100)}%`} label="In conversations" hint="the rest were quick questions or typed" />
          </div>

          <ChartPanel title="Questions per day" description="Everything you asked, day by day." table={{ head: ["Day", "Questions"], rows: d.perDay.map((p) => [p.label, p.value]) }}>
            <ChartContainer config={series} className="aspect-auto h-56 w-full">
              <AreaChart data={d.perDay} margin={{ left: -14, right: 8, top: 8 }}>
                <defs>
                  <linearGradient id="q-fill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="var(--color-value)" stopOpacity={0.28} />
                    <stop offset="100%" stopColor="var(--color-value)" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid vertical={false} {...grid} />
                <XAxis dataKey="label" {...axis} minTickGap={24} />
                <YAxis {...axis} allowDecimals={false} width={44} />
                <ChartTooltip
                  cursor={{ stroke: "var(--muted-foreground)", strokeOpacity: 0.4 }}
                  content={(props: any) => <Tooltip1 {...props} render={(p) => <><span className="font-medium">{p.value} question{p.value === 1 ? "" : "s"}</span> · {p.label}</>} />}
                />
                <Area dataKey="value" type="monotone" stroke="var(--color-value)" strokeWidth={2} fill="url(#q-fill)" dot={false} activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--card)" }} />
              </AreaChart>
            </ChartContainer>
          </ChartPanel>

          <ChartPanel
            title="Where the time goes"
            description="Your last 30 spoken questions: understanding your speech, then the model starting its answer."
            table={{ head: ["When", "Understanding (s)", "Answer starts (s)"], rows: d.recent.map((r) => [r.label, r.heard, r.think]) }}
          >
            <ChartContainer config={timing} className="aspect-auto h-56 w-full">
              <BarChart data={d.recent} margin={{ left: -14, right: 6, top: 8 }} barCategoryGap={3}>
                <CartesianGrid vertical={false} {...grid} />
                <XAxis dataKey="label" {...axis} tick={false} height={4} />
                <YAxis {...axis} width={52} tickFormatter={(v: number) => `${+v.toFixed(1)} s`} />
                <ChartTooltip
                  cursor={{ fill: "var(--muted)", opacity: 0.7 }}
                  content={(props: any) => (
                    <Tooltip1
                      {...props}
                      render={(p) => (
                        <>
                          <div className="font-medium">{(p.heard + p.think).toFixed(1)} s total</div>
                          <div className="text-muted-foreground">{p.heard} s understanding · {p.think} s to start · {p.label}</div>
                        </>
                      )}
                    />
                  )}
                />
                <ChartLegend content={<ChartLegendContent />} />
                <Bar dataKey="heard" stackId="t" fill="var(--color-heard)" maxBarSize={22} />
                <Bar dataKey="think" stackId="t" fill="var(--color-think)" radius={[4, 4, 0, 0]} maxBarSize={22} />
              </BarChart>
            </ChartContainer>
          </ChartPanel>

          <ChartPanel title="When you ask" description="Questions by hour of day." table={{ head: ["", "Questions"], rows: d.byHour.map((h) => [h.label, h.value]) }}>
            <ChartContainer config={series} className="aspect-auto h-48 w-full">
              <BarChart data={d.byHour} margin={{ left: -18, right: 6, top: 8 }} barCategoryGap={2}>
                <CartesianGrid vertical={false} {...grid} />
                <XAxis dataKey="label" {...axis} interval={2} minTickGap={6} />
                <YAxis {...axis} allowDecimals={false} width={44} />
                <ChartTooltip
                  cursor={{ fill: "var(--muted)", opacity: 0.7 }}
                  content={(props: any) => <Tooltip1 {...props} render={(p) => <><span className="font-medium">{p.value}</span> question{p.value === 1 ? "" : "s"} · {p.label}</>} />}
                />
                <Bar dataKey="value" fill="var(--color-value)" radius={[4, 4, 0, 0]} maxBarSize={26} />
              </BarChart>
            </ChartContainer>
          </ChartPanel>
        </div>
      )}
    </>
  );
}
