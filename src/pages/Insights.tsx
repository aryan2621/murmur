import { ViewSwitch } from "@/components/view-switch";
import { useView } from "@/lib/view";
import AssistantInsights from "@/assistant/Insights";
import DictationInsights from "@/dictation/Insights";

/** Insights for dictation, or for the assistant when there is one. */
export default function Insights({ assistantName }: { assistantName: string | null }) {
  const [view, setView] = useView("insights");
  if (!assistantName) return <DictationInsights />;
  return (
    <>
      <ViewSwitch value={view} onChange={setView} assistantName={assistantName} />
      {view === "assistant" ? <AssistantInsights /> : <DictationInsights />}
    </>
  );
}
