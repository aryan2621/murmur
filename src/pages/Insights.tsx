import { useMemo, useState, type ReactNode } from "react";
import { Area, AreaChart, Bar, BarChart, CartesianGrid, ReferenceLine, Scatter, ScatterChart, XAxis, YAxis, ZAxis } from "recharts";
import { ChartContainer, ChartTooltip, type ChartConfig } from "@/components/ui/chart";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ChartColumn, Table2 } from "lucide-react";
import { EmptyState, PageHeader, Panel } from "@/components/bits";
import { useHistory } from "@/hooks/use-app";
import { wordCount, type Entry } from "@/lib/api";
import { DAY, wordsPerDay } from "@/lib/metrics";

type Range = "7" | "30" | "all";

// One series per chart, so one validated colour (brand violet, checked on both surfaces).
const series: ChartConfig = { value: { label: "Value", color: "var(--chart-1)" } };
const axis = { tickLine: false, axisLine: false, tick: { fill: "var(--muted-foreground)", fontSize: 11 } } as const;
const grid = { stroke: "var(--border)", strokeOpacity: 0.7 } as const;

/** Least-squares fit y = a + b·x. */
function fit(points: { x: number; y: number }[]) {
  const n = points.length;
  if (n < 2) return null;
  const mx = points.reduce((s, p) => s + p.x, 0) / n;
  const my = points.reduce((s, p) => s + p.y, 0) / n;
  const sxx = points.reduce((s, p) => s + (p.x - mx) ** 2, 0);
  if (sxx === 0) return null;
  const b = points.reduce((s, p) => s + (p.x - mx) * (p.y - my), 0) / sxx;
  return { a: my - b * mx, b };
}

function useInsights(entries: Entry[], range: Range) {
  return useMemo(() => {
    const now = Date.now();
    const cutoff = range === "all" ? 0 : now - Number(range) * DAY;
    const list = entries.filter((e) => new Date(e.time).getTime() >= cutoff);

    const points = list
      .filter((e) => e.audio_seconds > 0 && e.transcribe_ms > 0)
      .map((e) => ({ x: +e.audio_seconds.toFixed(1), y: +(e.transcribe_ms / 1000).toFixed(2), words: wordCount(e.text) }));
    const trend = fit(points);
    const maxX = Math.max(1, ...points.map((p) => p.x));

    const oldest = list.length ? Math.min(...list.map((e) => +new Date(e.time))) : now;
    const days = range === "all" ? Math.min(60, Math.max(7, Math.ceil((now - oldest) / DAY) + 1)) : Number(range);
    const perDay = wordsPerDay(list, days, now);

    const byHour = Array.from({ length: 24 }, (_, h) => ({
      label: new Date(2000, 0, 1, h).toLocaleTimeString([], { hour: "numeric" }),
      value: 0,
    }));
    for (const e of list) byHour[new Date(e.time).getHours()].value += 1;


    const audio = list.reduce((s, e) => s + e.audio_seconds, 0);
    const compute = list.reduce((s, e) => s + e.transcribe_ms, 0) / 1000;
    const words = list.reduce((s, e) => s + wordCount(e.text), 0);
    const spoken = list.filter((e) => e.audio_seconds >= 1);
    const spokenWords = spoken.reduce((s, e) => s + wordCount(e.text), 0);
    const spokenSecs = spoken.reduce((s, e) => s + e.audio_seconds, 0);
    const busiest = byHour.reduce((a, b) => (b.value > a.value ? b : a), byHour[0]);

    return {
      count: list.length,
      points,
      trend,
      maxX,
      perDay,
      byHour,
      words,
      busiest: busiest.value ? busiest.label : null,
      realtime: compute > 0 ? audio / compute : 0,
      wpm: spokenSecs > 0 ? spokenWords / (spokenSecs / 60) : 0,
    };
  }, [entries, range]);
}

/** Chart panel with a Chart/Table switch so every chart has a non-visual view. */
function ChartPanel({ title, description, table, children, className }: { title: string; description: ReactNode; table: { head: string[]; rows: (string | number)[][] }; children: ReactNode; className?: string }) {
  const [view, setView] = useState("chart");
  return (
    <Panel className={className}>
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

function BarPanel({ title, description, data, unit, every = 0 }: { title: string; description: string; data: { label: string; value: number }[]; unit: string; every?: number }) {
  return (
    <ChartPanel title={title} description={description} table={{ head: ["", unit], rows: data.map((d) => [d.label, d.value]) }}>
      <ChartContainer config={series} className="aspect-auto h-48 w-full">
        <BarChart data={data} margin={{ left: -18, right: 6, top: 8 }} barCategoryGap={2}>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="label" {...axis} interval={every} minTickGap={6} />
          <YAxis {...axis} allowDecimals={false} width={44} />
          <ChartTooltip
            cursor={{ fill: "var(--muted)", opacity: 0.7 }}
            content={(props: any) => <Tooltip1 {...props} render={(p) => <><span className="font-medium">{p.value.toLocaleString()}</span> {unit.toLowerCase()} · {p.label}</>} />}
          />
          <Bar dataKey="value" fill="var(--color-value)" radius={[4, 4, 0, 0]} maxBarSize={26} />
        </BarChart>
      </ChartContainer>
    </ChartPanel>
  );
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

export default function Insights() {
  const { entries } = useHistory("", 100_000);
  const [range, setRange] = useState<Range>("30");
  const d = useInsights(entries ?? [], range);
  const t = d.trend;

  return (
    <>
      <PageHeader
        title="Insights"
        description={d.busiest ? `You dictate most around ${d.busiest}. Here's how you talk and how fast Murmur keeps up.` : "How you talk and how fast Murmur keeps up."}
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
          <EmptyState title="Nothing to chart yet">Dictate a few times in this period and your charts appear here.</EmptyState>
        </Panel>
      ) : (
        <div className="flex flex-col gap-4">
          <div className="grid grid-cols-3 gap-3">
            <Hero value={d.words.toLocaleString()} label="Words" hint={`across ${d.count.toLocaleString()} dictation${d.count === 1 ? "" : "s"}`} />
            <Hero value={d.wpm ? Math.round(d.wpm).toString() : "–"} label="Words per minute" hint="your speaking pace" />
            <Hero value={d.realtime ? `${d.realtime.toFixed(1)}×` : "–"} label="Faster than real time" hint="speech length ÷ processing time" />
          </div>

          <ChartPanel
            title="Words per day"
            description="Everything you dictated, day by day."
            table={{ head: ["Day", "Words"], rows: d.perDay.map((p) => [p.label, p.value]) }}
          >
            <ChartContainer config={series} className="aspect-auto h-56 w-full">
              <AreaChart data={d.perDay} margin={{ left: -14, right: 8, top: 8 }}>
                <defs>
                  <linearGradient id="words-fill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="var(--color-value)" stopOpacity={0.28} />
                    <stop offset="100%" stopColor="var(--color-value)" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid vertical={false} {...grid} />
                <XAxis dataKey="label" {...axis} minTickGap={24} />
                <YAxis {...axis} allowDecimals={false} width={44} />
                <ChartTooltip
                  cursor={{ stroke: "var(--muted-foreground)", strokeOpacity: 0.4 }}
                  content={(props: any) => <Tooltip1 {...props} render={(p) => <><span className="font-medium">{p.value.toLocaleString()} words</span> · {p.label}</>} />}
                />
                <Area dataKey="value" type="monotone" stroke="var(--color-value)" strokeWidth={2} fill="url(#words-fill)" dot={false} activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--card)" }} />
              </AreaChart>
            </ChartContainer>
          </ChartPanel>

          <ChartPanel
            title="Clip length vs. transcription time"
            description={
              t ? (
                <>
                  Each dot is one dictation. Murmur takes about <b className="font-medium text-foreground">{Math.max(0, t.a).toFixed(1)} s</b> fixed plus{" "}
                  <b className="font-medium text-foreground">{Math.max(0, t.b * 1000).toFixed(0)} ms</b> per second you speak (dashed line).
                </>
              ) : (
                "Each dot is one dictation."
              )
            }
            table={{ head: ["Spoken (s)", "Transcribe (s)", "Words"], rows: d.points.map((p) => [p.x, p.y, p.words]) }}
          >
            <ChartContainer config={series} className="aspect-auto h-64 w-full">
              <ScatterChart margin={{ left: -6, right: 14, top: 8, bottom: 4 }}>
                <CartesianGrid {...grid} />
                <XAxis type="number" dataKey="x" name="Spoken" unit="s" {...axis} domain={[0, "auto"]} />
                <YAxis type="number" dataKey="y" name="Transcription" unit="s" {...axis} domain={[0, "auto"]} width={48} />
                <ZAxis range={[64, 64]} />
                {t && (
                  <ReferenceLine
                    segment={[{ x: 0, y: Math.max(0, t.a) }, { x: d.maxX, y: Math.max(0, t.a + t.b * d.maxX) }]}
                    stroke="var(--muted-foreground)"
                    strokeWidth={2}
                    strokeDasharray="4 4"
                    ifOverflow="extendDomain"
                  />
                )}
                <ChartTooltip
                  cursor={{ strokeDasharray: "3 3" }}
                  content={(props: any) => <Tooltip1 {...props} render={(p) => <><div className="font-medium">{p.x} s spoken</div><div className="text-muted-foreground">{p.y} s to transcribe · {p.words} words</div></>} />}
                />
                <Scatter data={d.points} fill="var(--color-value)" stroke="var(--card)" strokeWidth={2} fillOpacity={0.9} />
              </ScatterChart>
            </ChartContainer>
          </ChartPanel>

          <BarPanel title="When you dictate" description="Dictations by hour of day." data={d.byHour} unit="Dictations" every={2} />
        </div>
      )}
    </>
  );
}
