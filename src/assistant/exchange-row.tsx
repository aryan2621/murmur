import { Check, Copy, Trash2, Zap } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { api, formatSeconds, responseMs, type Exchange } from "@/lib/api";

const MODE_LABEL: Record<string, string> = { quick: "Quick question", conversation: "Conversation", typed: "Typed" };

export function ExchangeRow({ entry, onDeleted }: { entry: Exchange; onDeleted?: () => void }) {
  const [copied, setCopied] = useState(false);
  const time = new Date(entry.time).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const meta = [
    MODE_LABEL[entry.mode],
    entry.first_word_ms ? `answered in ${formatSeconds(responseMs(entry))}` : null,
    entry.model || null,
  ].filter(Boolean);

  return (
    <div className="group relative flex gap-5 px-5 py-4 transition-colors not-last:border-b hover:bg-muted/40">
      <div className="w-16 shrink-0 pt-0.5 text-xs text-muted-foreground tabular-nums">{time}</div>
      <div className="min-w-0 flex-1 pr-16">
        <p data-selectable className="text-[14px] leading-relaxed font-medium break-words">
          {entry.question}
        </p>
        {!!entry.actions?.length && (
          <ul className="mt-1.5 flex flex-wrap gap-1">
            {entry.actions.map((a, i) => (
              <li key={i} className="flex items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                <Zap className="size-3" />
                {a}
              </li>
            ))}
          </ul>
        )}
        <p data-selectable className="mt-1 text-[14px] leading-relaxed break-words whitespace-pre-wrap text-foreground/75">
          {entry.answer}
        </p>
        <p className="mt-1.5 text-[11px] text-muted-foreground/80 tabular-nums">{meta.join(" · ")}</p>
      </div>
      <div className="absolute top-3 right-3 flex gap-0.5 rounded-lg border bg-card p-0.5 opacity-0 shadow-sm transition-opacity group-hover:opacity-100 focus-within:opacity-100">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Copy answer"
              onClick={async () => {
                try {
                  await api.copy(entry.answer);
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1200);
                } catch (error) {
                  toast.error(`Couldn't copy: ${error}`);
                }
              }}
            >
              {copied ? <Check className="text-emerald-600" /> : <Copy />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>Copy answer</TooltipContent>
        </Tooltip>
        {onDeleted && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Delete"
                onClick={async () => {
                  try {
                    await api.deleteExchange(entry.time);
                    toast("Deleted");
                    onDeleted();
                  } catch (error) {
                    toast.error(`Couldn't delete: ${error}`);
                  }
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
