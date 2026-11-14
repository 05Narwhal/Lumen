// The face radar (Round 9A): after a correction, Lumen re-checks the photos around it and
// fixes the faces it placed itself. One toast, one Undo, for the whole sweep.

import type { QueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { toast } from "@/components/toast";
import type { FacesRadar } from "@/types/generated/FacesRadar";
import { invalidateLayouts } from "./events";
import { invoke } from "./ipc";

export const FACES_RADAR = "faces://radar";

/** *Fixed 7 more photos of Anna*; *Took Ruta off 3 more photos* when she only lost faces. */
export function radarMessage(e: FacesRadar): string {
  const photos = e.photos === 1 ? "1 more photo" : `${e.photos} more photos`;
  if (e.undo.added === 0 && e.undo.removed > 0) return `Took ${e.name ?? "this person"} off ${photos}`;
  return `Fixed ${photos} of ${e.name ?? "this person"}`;
}

function refresh(qc: QueryClient) {
  void Promise.all([qc.invalidateQueries({ queryKey: ["people"] }), invalidateLayouts(qc)]);
}

export function onFacesRadar(qc: QueryClient, e: FacesRadar) {
  refresh(qc);
  toast({ message: radarMessage(e), undo: () => void invoke("restore_faces", { places: e.undo.moves }).then(() => refresh(qc)) });
}

/** Registered by the event bridge. */
export function listenFacesRadar(qc: QueryClient) {
  return listen<FacesRadar>(FACES_RADAR, (e) => onFacesRadar(qc, e.payload));
}
