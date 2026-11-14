import { render } from "@testing-library/react";
import type { ReactElement } from "react";
import { MemoryRouter } from "react-router";
import { Providers } from "@/app";
import { mockCommand } from "@/test/tauri-mock";

/** Round 22 B4: what the Repair page asks of a library with nothing to report. */
export function mockRepairDefaults(over: { health?: object } = {}) {
  mockCommand("get_library_health", () => ({ total: 100, unreadable: 0, missing: 0, recovered: 0, checkedAt: null, causes: [], ...over.health }));
  mockCommand("get_check_status", () => ({ running: null }));
  mockCommand("get_recovered", () => []);
  mockCommand("get_repair_outcomes", () => []);
  mockCommand("list_import_batches", () => []);
  mockCommand("list_unreadable", () => ({ items: [], nextCursor: null }));
  mockCommand("list_repair_sources", () => []);
  mockCommand("start_check_library", () => null);
}

export const IDLE = {
  phase: "idle",
  done: 0,
  total: 0,
  counts: { safe: 0, new: 0, damaged: 0, sourceDamaged: 0, sidecar: 0, inTrash: 0 },
  cancelled: false,
  current: null,
  bytesDone: 0,
  bytesTotal: 0,
} as const;

export function renderRepair(ui: ReactElement, entry = "/import?mode=repair") {
  return render(
    <Providers>
      <MemoryRouter initialEntries={[entry]}>{ui}</MemoryRouter>
    </Providers>,
  );
}
