import { ViewSwitch } from "@/components/view-switch";
import { useView } from "@/lib/view";
import AssistantHistory from "@/assistant/History";
import DictationHistory from "@/dictation/History";

/** History for dictation, or for the assistant when there is one. */
export default function History({ assistantName }: { assistantName: string | null }) {
  const [view, setView] = useView("history");
  if (!assistantName) return <DictationHistory />;
  return (
    <>
      <ViewSwitch value={view} onChange={setView} assistantName={assistantName} />
      {view === "assistant" ? <AssistantHistory /> : <DictationHistory />}
    </>
  );
}
