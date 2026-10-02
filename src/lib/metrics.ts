// Client-side aggregations over dictation history.
import { wordCount, type Entry } from "@/lib/api";

export const DAY = 86_400_000;
export const dayKey = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;

/** Words per calendar day for the last `days` days, oldest first. */
export function wordsPerDay(entries: Entry[], days: number, now = Date.now()) {
  const out = Array.from({ length: days }, (_, i) => {
    const d = new Date(now - (days - 1 - i) * DAY);
    return {
      key: dayKey(d),
      label: d.toLocaleDateString([], { month: "short", day: "numeric" }),
      weekday: d.toLocaleDateString([], { weekday: "short" }),
      value: 0,
    };
  });
  const byKey = new Map(out.map((d) => [d.key, d]));
  for (const e of entries) {
    const slot = byKey.get(dayKey(new Date(e.time)));
    if (slot) slot.value += wordCount(e.text);
  }
  return out;
}

/** Consecutive days with at least one dictation, counting back from today (or yesterday). */
export function streak(entries: Entry[], now = Date.now()) {
  const days = new Set(entries.map((e) => dayKey(new Date(e.time))));
  let start = days.has(dayKey(new Date(now))) ? 0 : 1;
  let n = 0;
  while (days.has(dayKey(new Date(now - (start + n) * DAY)))) n++;
  return n;
}

export const formatMinutes = (m: number) => (m >= 60 ? `${(m / 60).toFixed(1)} h` : `${Math.max(0, Math.round(m))} min`);
