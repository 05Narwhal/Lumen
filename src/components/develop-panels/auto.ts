import { toast } from "@/components/toast";
import type { AiTask } from "@/hooks/use-ai-task";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { isAppError } from "@/lib/ipc";
import { createStore } from "@/lib/store";
import type { AutoResult } from "@/types/generated/AutoResult";

/** Asks the open Develop screen to run *Auto* (`Mod+U`, the palette's *Auto tone*). */
export const autoRequests = createStore(0);
export const requestAuto = () => autoRequests.set((n) => n + 1);

/** What the toast says after *Auto*. */
export function autoMessage(r: AutoResult): string {
  const auto = r.source === "model" && r.model ? `Auto with ${r.model}` : "Auto";
  // The trained model's look was applied even when the image model could not judge.
  if (!r.usedModel && auto === "Auto") return "Auto measured the photo; smart features are off or still downloading.";
  return r.scene ? `${auto} — read as a ${r.scene}` : auto;
}

/** Runs *Auto* through its button's task (one at a time, cancellable) and says how it went. */
export async function autoTone(session: DevelopSession, ai: AiTask<AutoResult | null>): Promise<void> {
  try {
    const r = await ai.run((task, stillWanted) => session.auto(task, stillWanted));
    if (r) toast({ message: autoMessage(r) });
  } catch (e) {
    if (isAppError(e) && e.code === "cancelled") return;
    toast({ message: isAppError(e) ? e.message : "Auto could not read this photo." });
  }
}
