import { confirm } from "@/components/confirm-dialog";
import { toast } from "@/components/toast";
import { invoke } from "@/lib/ipc";
import type { ImportPlan } from "@/types/generated/ImportPlan";
import { formatStorage } from "@/utils/format";

const n = (v: number) => v.toLocaleString("en-US");

export type RoomQuestion =
  | { kind: "all" }
  | { kind: "some"; title: string; body: string; confirmLabel: string; limitBytes: number }
  | { kind: "none"; message: string };

/** What to ask before an import, from how much of it fits (round 3: keep 20 GB free). */
export function roomQuestion(p: ImportPlan): RoomQuestion {
  if (p.freeBytes === null || p.fitsCount >= p.count) return { kind: "all" };
  const buffer = formatStorage(p.buffer);
  if (p.fitsCount === 0) {
    return { kind: "none", message: `There is no room to import while keeping ${buffer} free on the library's drive (${formatStorage(p.freeBytes)} free).` };
  }
  return {
    kind: "some",
    title: "Not enough space for everything",
    body: `${n(p.fitsCount)} of ${n(p.count)} items (${formatStorage(p.fitsBytes)}) fit and keep ${buffer} free on the library's drive. The newest come first; the rest wait until you free up space.`,
    confirmLabel: `Import ${n(p.fitsCount)}`,
    limitBytes: p.fitsBytes,
  };
}

export type ImportPlanRequest =
  | { kind: "folder"; id: string; recursive?: boolean }
  | { kind: "files"; paths: string[] }
  | { kind: "volume" | "iphone"; id: string }
  /** The tiles picked in a device browser (Round 4E). */
  | { kind: "browser"; session: number; indexes: number[] };

/**
 * Before an import: whether there is room. `null` imports everything, a number imports the
 * newest that fit in that many bytes, and `false` means don't start (cancelled, or no room).
 */
export async function checkRoom(req: ImportPlanRequest): Promise<number | null | false> {
  let plan: ImportPlan;
  try {
    plan =
      req.kind === "browser"
        ? await invoke<ImportPlan>("plan_device_browser_import", { session: req.session, indexes: req.indexes })
        : await invoke<ImportPlan>("plan_import", {
            kind: req.kind,
            id: "id" in req ? req.id : null,
            paths: "paths" in req ? req.paths : null,
            recursive: "recursive" in req ? (req.recursive ?? true) : null,
          });
  } catch {
    // Planning is advice: without it the import still runs, and the ingest guard still
    // stops before the drive drops under the buffer.
    return null;
  }
  const q = roomQuestion(plan);
  if (q.kind === "all") return null;
  if (q.kind === "none") {
    toast({ message: q.message });
    return false;
  }
  const ok = await confirm({ title: q.title, body: q.body, confirmLabel: q.confirmLabel });
  return ok ? q.limitBytes : false;
}
