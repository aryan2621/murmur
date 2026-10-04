import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { PAGE_SIZES } from "@/hooks/use-paged";
import { cn } from "@/lib/utils";

/** Page numbers to show: the first, the last, and the ones around the current page. */
function pageList(page: number, pages: number): (number | "gap")[] {
  const keep = new Set([0, pages - 1, page - 1, page, page + 1].filter((p) => p >= 0 && p < pages));
  const out: (number | "gap")[] = [];
  [...keep].sort((a, b) => a - b).forEach((p, i, all) => {
    if (i > 0 && p - all[i - 1] > 1) out.push("gap");
    out.push(p);
  });
  return out;
}

/** Page size, "21–30 of 160", and page buttons, under a history list. */
export function Pager({ page, pages, size, total, onPage, onSize }: { page: number; pages: number; size: number; total: number; onPage: (p: number) => void; onSize: (s: number) => void }) {
  if (!total) return null;
  const go = (p: number) => {
    onPage(p);
    document.querySelector("main")?.scrollTo({ top: 0 });
  };
  const from = page * size + 1;
  const to = Math.min(total, from + size - 1);
  return (
    <div className="mt-2 flex items-center gap-3 text-xs text-muted-foreground">
      <span className="flex items-center gap-2">
        Show
        <Select value={String(size)} onValueChange={(v) => onSize(Number(v))}>
          <SelectTrigger size="sm" className="w-[72px]" aria-label="Entries per page"><SelectValue /></SelectTrigger>
          <SelectContent>{PAGE_SIZES.map((s) => <SelectItem key={s} value={String(s)}>{s}</SelectItem>)}</SelectContent>
        </Select>
        per page
      </span>
      <span className="flex-1 text-center tabular-nums">
        {from.toLocaleString()}–{to.toLocaleString()} of {total.toLocaleString()}
      </span>
      <nav className="flex items-center gap-1" aria-label="Pages">
        <Button variant="ghost" size="icon-sm" aria-label="Previous page" disabled={page === 0} onClick={() => go(page - 1)}>
          <ChevronLeft />
        </Button>
        {pageList(page, pages).map((p, i) =>
          p === "gap" ? (
            <span key={`gap-${i}`} className="px-1">…</span>
          ) : (
            <Button
              key={p}
              variant={p === page ? "outline" : "ghost"}
              size="sm"
              aria-current={p === page ? "page" : undefined}
              className={cn("h-7 min-w-7 px-2 tabular-nums", p === page && "bg-card font-semibold text-foreground")}
              onClick={() => go(p)}
            >
              {p + 1}
            </Button>
          ),
        )}
        <Button variant="ghost" size="icon-sm" aria-label="Next page" disabled={page >= pages - 1} onClick={() => go(page + 1)}>
          <ChevronRight />
        </Button>
      </nav>
    </div>
  );
}
