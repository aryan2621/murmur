import { AudioLines, Sparkles } from "lucide-react";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { View } from "@/lib/view";

/** Dictation | Assistant switch at the top of Insights and History. */
export function ViewSwitch({ value, onChange, assistantName }: { value: View; onChange: (v: View) => void; assistantName: string }) {
  return (
    <ToggleGroup type="single" variant="outline" size="sm" value={value} onValueChange={(v) => v && onChange(v as View)} className="mb-6 rounded-lg bg-card shadow-lift">
      <ToggleGroupItem value="dictation" className="px-3">
        <AudioLines /> Dictation
      </ToggleGroupItem>
      <ToggleGroupItem value="assistant" className="px-3">
        <Sparkles /> {assistantName}
      </ToggleGroupItem>
    </ToggleGroup>
  );
}
