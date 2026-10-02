import { Check, Copy, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { api, wordCount, type Entry } from "@/lib/api";

export function EntryRow({ entry, onDeleted }: { entry: Entry; onDeleted?: () => void }) {
  const [copied, setCopied] = useState(false);
  const time = new Date(entry.time).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const meta = [`${wordCount(entry.text)} words`, entry.audio_seconds ? `${Math.round(entry.audio_seconds)} s` : null].filter(Boolean);

  return (
    <div className="group relative flex gap-5 px-5 py-4 transition-colors not-last:border-b hover:bg-muted/40">
      <div className="w-16 shrink-0 pt-0.5 text-xs text-muted-foreground tabular-nums">{time}</div>
      <div className="min-w-0 flex-1">
        <p data-selectable className="text-[14px] leading-relaxed break-words whitespace-pre-wrap">
          {entry.text}
        </p>
        <p className="mt-1.5 text-[11px] text-muted-foreground/80 tabular-nums">{meta.join(" · ")}</p>
      </div>
      <div className="absolute top-3 right-3 flex gap-0.5 rounded-lg border bg-card p-0.5 opacity-0 shadow-sm transition-opacity group-hover:opacity-100 focus-within:opacity-100">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Copy"
              onClick={async () => {
                await api.copy(entry.text);
                setCopied(true);
                setTimeout(() => setCopied(false), 1200);
              }}
            >
              {copied ? <Check className="text-emerald-600" /> : <Copy />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>Copy</TooltipContent>
        </Tooltip>
        {onDeleted && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Delete"
                onClick={async () => {
                  await api.deleteEntry(entry.time);
                  toast("Dictation deleted");
                  onDeleted();
                }}
              >
                <Trash2 />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Delete</TooltipContent>
          </Tooltip>
        )}
      </div>
    </div>
  );
}
