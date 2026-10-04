// Client-side aggregations over question/answer history.
import { responseMs, type Exchange } from "@/lib/api";
import { DAY, dayKey } from "@/lib/metrics";

export { DAY, streak } from "@/lib/metrics";

/** Questions per calendar day for the last `days` days, oldest first. */
export function questionsPerDay(entries: Exchange[], days: number, now = Date.now()) {
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
    if (slot) slot.value += 1;
  }
  return out;
}

/** Median of a list (0 when empty): steadier than the mean when one answer had a cold start. */
export function median(values: number[]) {
  if (!values.length) return 0;
  const s = [...values].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

/** Median time from the end of a spoken question to the first words of the answer. */
export const medianResponseMs = (entries: Exchange[]) => median(entries.filter((e) => e.mode !== "typed" && e.first_word_ms > 0).map(responseMs));
