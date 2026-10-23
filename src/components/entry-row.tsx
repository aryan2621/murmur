import { Check, Copy, Loader2, Pencil, Play, RotateCcw, Trash2, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { api, wordCount, type Entry, type Learning } from "@/lib/api";
import { useAppState } from "@/hooks/use-app";

export function EntryRow({ entry, onDeleted, onUpdated }: { entry: Entry; onDeleted?: () => void; onUpdated?: () => void }) {
  const { state } = useAppState();
  const [copied, setCopied] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(entry.text);
  const [saving, setSaving] = useState(false);
  const [learnings, setLearnings] = useState<Learning[]>([]);
  const [picked, setPicked] = useState<Record<number, boolean>>({});
  const [learnOpen, setLearnOpen] = useState(false);
  const [redoing, setRedoing] = useState(false);

  const time = new Date(entry.time).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const meta = [`${wordCount(entry.text)} words`, entry.audio_seconds ? `${Math.round(entry.audio_seconds)} s` : null].filter(Boolean);

  async function saveEdit() {
    const next = draft.trim();
    if (!next || next === entry.text) {
      setEditing(false);
      setDraft(entry.text);
      return;
    }
    setSaving(true);
    try {
      await api.updateEntry(entry.time, next);
      if (state?.config.learn_from_corrections) {
        const suggestions = await api.suggestLearnings(entry.time, next);
        if (suggestions.length) {
          setLearnings(suggestions);
          setPicked(Object.fromEntries(suggestions.map((_, i) => [i, true])));
          setLearnOpen(true);
        } else {
          toast("Dictation updated");
        }
      } else {
        toast("Dictation updated");
      }
      setEditing(false);
      onUpdated?.();
    } catch (e) {
      toast.error(`Couldn't save: ${e}`);
    } finally {
      setSaving(false);
    }
  }

  async function retranscribe() {
    setRedoing(true);
    try {
      await api.retranscribe(entry.time);
      toast("Transcribed again with your current settings");
      onUpdated?.();
    } catch (e) {
      toast.error(`Couldn't transcribe: ${e}`);
    } finally {
      setRedoing(false);
    }
  }

  async function applyLearnings() {
    const chosen = learnings.filter((_, i) => picked[i]);
    if (chosen.length) {
      try {
        await api.applyLearnings(chosen);
        toast("Saved for next time");
      } catch (e) {
        toast.error(`Couldn't save: ${e}`);
      }
    }
    setLearnOpen(false);
    setLearnings([]);
  }

  return (
    <>
      <div className="group relative flex gap-5 px-5 py-4 transition-colors not-last:border-b hover:bg-muted/40">
        <div className="w-16 shrink-0 pt-0.5 text-xs text-muted-foreground tabular-nums">{time}</div>
        <div className="min-w-0 flex-1">
          {editing ? (
            <Textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              rows={3}
              className="text-[14px] leading-relaxed"
              disabled={saving}
            />
          ) : (
            <p data-selectable className="text-[14px] leading-relaxed break-words whitespace-pre-wrap">
              {entry.text}
            </p>
          )}
          <p className="mt-1.5 text-[11px] text-muted-foreground/80 tabular-nums">{meta.join(" · ")}</p>
        </div>
        <div className="absolute top-3 right-3 flex gap-0.5 rounded-lg border bg-card p-0.5 opacity-0 shadow-sm transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          {editing ? (
            <>
              <Button variant="ghost" size="icon-sm" aria-label="Save" disabled={saving} onClick={() => void saveEdit()}>
                <Check />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Cancel edit"
                disabled={saving}
                onClick={() => {
                  setEditing(false);
                  setDraft(entry.text);
                }}
              >
                <X />
              </Button>
            </>
          ) : (
            <>
              {entry.audio && (
                <>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button variant="ghost" size="icon-sm" aria-label="Play audio" onClick={() => api.playAudio(entry.time).catch((e) => toast.error(String(e)))}>
                        <Play />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Play audio</TooltipContent>
                  </Tooltip>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button variant="ghost" size="icon-sm" aria-label="Transcribe again" disabled={redoing} onClick={() => void retranscribe()}>
                        {redoing ? <Loader2 className="animate-spin" /> : <RotateCcw />}
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Transcribe again</TooltipContent>
                  </Tooltip>
                </>
              )}
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon-sm" aria-label="Edit" onClick={() => setEditing(true)}>
                    <Pencil />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Edit</TooltipContent>
              </Tooltip>
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
            </>
          )}
        </div>
      </div>

      <AlertDialog open={learnOpen} onOpenChange={setLearnOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remember this fix?</AlertDialogTitle>
            <AlertDialogDescription>Murmur can apply it automatically on future dictations.</AlertDialogDescription>
          </AlertDialogHeader>
          <div className="flex flex-col gap-3 py-2">
            {learnings.map((l, i) => (
              <label key={i} className="flex items-start gap-2 text-sm">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={!!picked[i]}
                  onChange={(e) => setPicked((p) => ({ ...p, [i]: e.target.checked }))}
                />
                <span>
                  {l.kind === "replacement" ? (
                    <>
                      Always replace <strong>{l.from}</strong> with <strong>{l.to}</strong>
                    </>
                  ) : (
                      <>
                        Add <strong>{l.to}</strong> to vocabulary
                      </>
                    )}
                </span>
              </label>
            ))}
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => setLearnOpen(false)}>Skip</AlertDialogCancel>
            <AlertDialogAction onClick={() => void applyLearnings()}>Save</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
