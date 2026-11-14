import { toast } from "@/components/toast";
import type { AiTask } from "@/hooks/use-ai-task";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { prismInstalled } from "@/lib/experimental";
import { isAppError } from "@/lib/ipc";
import { createStore } from "@/lib/store";
import type { ExperimentalTask } from "@/types/generated/ExperimentalTask";
import type { RetouchResult } from "@/types/generated/RetouchResult";
import type { Settings } from "@/types/generated/Settings";

/** Asks the open Develop screen to run *Retouch* (`Mod+Shift+U`, the palette's *Retouch photo*). */
export const retouchRequests = createStore(0);
export const requestRetouch = () => retouchRequests.set((n) => n + 1);

/** Whether *Retouch* is offered: its Experimental switch on and smart features on (the backend
 *  only runs classic Auto without them). */
export function retouchOn(settings: Settings | undefined): boolean {
  return settings?.smart?.enabled !== false && settings?.smart?.experimental?.retouch === true;
}

/** Whether *Retouch* can run (Round 14): its switch on and a Lumen Prism model installed. */
export function retouchReady(settings: Settings | undefined, catalog: Pick<ExperimentalTask, "task" | "models">[] | undefined): boolean {
  return retouchOn(settings) && prismInstalled(catalog);
}

/** What the toast says after *Retouch*. */
export function retouchMessage(r: RetouchResult): string {
  if (!r.usedModel) return "Retouch model isn't ready — used Auto instead.";
  const retouch = r.model ? `Retouch with ${r.model}` : "Retouch";
  return r.summary.length > 0 ? `${retouch} — ${r.summary.join(", ")}` : retouch;
}

/** Runs *Retouch* through its button's task (one at a time, cancellable) and says how it went. */
export async function retouchPhoto(session: DevelopSession, ai: AiTask<RetouchResult | null>): Promise<void> {
  try {
    const r = await ai.run((task, stillWanted) => session.retouch(task, stillWanted));
    if (r) toast({ message: retouchMessage(r) });
  } catch (e) {
    if (isAppError(e) && e.code === "cancelled") return;
    toast({ message: isAppError(e) ? e.message : "Retouch could not read this photo." });
  }
}
