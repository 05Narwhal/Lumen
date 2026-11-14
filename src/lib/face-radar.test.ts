import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import type { FacesRadar } from "@/types/generated/FacesRadar";
import { onFacesRadar, radarMessage } from "./face-radar";

const sweep = (photos: number, name: string | null, added = 2, removed = 0): FacesRadar => ({
  person: 4,
  name,
  photos,
  undo: { person: 4, moves: [{ face: 10, person: 2, pinned: false }, { face: 11, person: null, pinned: false }], assets: [7, 8], added, removed },
});

describe("face radar", () => {
  beforeEach(() => toastStore.set({ toasts: [] }));

  it("says what it fixed in one sentence", () => {
    expect(radarMessage(sweep(7, "Anna"))).toBe("Fixed 7 more photos of Anna");
    expect(radarMessage(sweep(1, "Anna"))).toBe("Fixed 1 more photo of Anna");
    expect(radarMessage(sweep(3, null))).toBe("Fixed 3 more photos of this person");
    expect(radarMessage(sweep(3, "Ruta", 0, 3))).toBe("Took Ruta off 3 more photos");
    expect(radarMessage(sweep(4, "Ruta", 1, 3))).toBe("Fixed 4 more photos of Ruta");
  });

  it("offers one Undo for the whole sweep and refreshes People", async () => {
    mockCommand("restore_faces", () => null);
    const qc = new QueryClient();
    const invalidate = vi.spyOn(qc, "invalidateQueries");
    onFacesRadar(qc, sweep(2, "Anna"));
    const [t] = toastStore.get().toasts;
    expect(t.message).toBe("Fixed 2 more photos of Anna");
    expect(t.action?.label).toBe("Undo");
    expect(t.durationMs).toBe(6000);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["people"] });
    t.action?.run();
    expect(invokeMock).toHaveBeenCalledWith("restore_faces", { places: sweep(2, "Anna").undo.moves });
  });
});
