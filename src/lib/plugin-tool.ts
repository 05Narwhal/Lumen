import { confirm } from "@/components/confirm-dialog";
import { dismissToast, toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import type { PluginModelStatus } from "@/types/generated/PluginModelStatus";

/**
 * What a `tool:<name>` contribution works on, passed to the tool as its arguments. A selection from
 * a grid or the viewer carries its `assets`, which the host reads as the selection for that call
 * (`library.query` with `selection`); without them the selection is Develop's filmstrip.
 */
export type ToolScope = { scope: "library" } | { scope: "album"; album: number } | { scope: "selection"; assets?: number[] };

/** A resumable tool's answer: `working` means call again; `review` names the page to check it on. */
interface Answer {
  status?: "working" | "done";
  suggested?: number;
  message?: string;
  review?: string;
}

/** The plugin a tool belongs to, as the slot knows it. */
export interface ToolOwner {
  id: string;
  name: string;
}

const REVIEW_ID = /^[A-Za-z0-9_-]+$/;

/**
 * The review page a tool's answer may open: only `/review/<id>` (or a bare `<id>`), and always
 * for the plugin that answered, whatever `plugin=` it wrote. Anything else opens nothing.
 */
export function reviewTarget(pluginId: string, review: string | undefined): string | null {
  if (!review) return null;
  const id = review.startsWith("/review/") ? review.slice("/review/".length).split(/[?#]/)[0] : review;
  return REVIEW_ID.test(id) ? `/review/${id}?plugin=${encodeURIComponent(pluginId)}` : null;
}

const mb = (n: number) => `${Math.round(n / 1e6).toLocaleString("en")} MB`;
const running = new Set<string>();

/** Downloads the plugin's models after asking (they can be large); false when the person says no. */
async function ensureModels(owner: ToolOwner): Promise<boolean> {
  const status = await invoke<PluginModelStatus[]>("plugin_models_status", { pluginId: owner.id });
  const missing = status.filter((m) => !m.downloaded);
  if (missing.length === 0) return true;
  const ok = await confirm({
    title: `Download ${owner.name}'s model?`,
    body: `${owner.name} needs ${mb(missing.reduce((n, m) => n + m.size, 0))} once (${[...new Set(missing.map((m) => m.licence))].join(", ")}). It runs on this PC; your photos never leave it.`,
    confirmLabel: "Download",
  });
  if (!ok) return false;
  await invoke("plugin_models_download", { pluginId: owner.id });
  return true;
}

/**
 * Runs a plugin's resumable `[[tools]]` entry on a scope: each call works a while and says
 * `working` until it is done; then the answer's line is shown, or its review page offered.
 * Nothing here knows which plugin it is running.
 */
export async function runPluginTool(owner: ToolOwner, tool: string, scope: ToolScope, navigate: (to: string) => void): Promise<void> {
  const key = `${owner.id}\u0000${tool}`;
  if (running.has(key)) {
    toast({ message: `${owner.name} is already working on that.` });
    return;
  }
  running.add(key);
  let busy: number | null = null;
  try {
    if (!(await ensureModels(owner))) return;
    busy = toast({ message: `${owner.name} is working… this can take a while. Keep Lumen open.`, durationMs: 3_600_000 });
    for (;;) {
      const a = await invoke<Answer | string>("plugin_tool_run", { pluginId: owner.id, tool, args: scope });
      const answer: Answer = typeof a === "string" ? { status: "done", message: a } : a;
      if (answer.status === "working") continue;
      dismissToast(busy);
      busy = null;
      const to = answer.suggested ? reviewTarget(owner.id, answer.review) : null;
      if (to && answer.suggested) {
        toast({ message: `${owner.name} has ${answer.suggested.toLocaleString("en")} to check.`, durationMs: 15_000, action: { label: "Review", run: () => navigate(to) } });
      } else {
        toast({ message: answer.message ?? `${owner.name} is done.` });
      }
      return;
    }
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : `${owner.name} couldn't do that.` });
  } finally {
    if (busy !== null) dismissToast(busy);
    running.delete(key);
  }
}
