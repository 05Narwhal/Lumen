import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { mockCommand } from "@/test/tauri-mock";
import { SourceResult } from "./source-result";
import { IDLE, mockRepairDefaults, renderRepair } from "./test-helpers";

describe("SourceResult", () => {
  it("sourceDamaged rows cannot be ticked", async () => {
    mockRepairDefaults();
    mockCommand("get_repair_items", () => ({
      items: [{ id: 3, name: "IMG_3.HEIC", dir: "100APPLE", status: "sourceDamaged", assetId: 3, byteSize: 1000 }],
      nextCursor: null,
    }));
    const status = { ...IDLE, phase: "done", done: 1, total: 1, counts: { ...IDLE.counts, sourceDamaged: 1 } } as never;
    renderRepair(<SourceResult status={status} name="DCIM" onBack={vi.fn()} restoring={false} />);
    expect(await screen.findByText("IMG_3.HEIC")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Select IMG_3.HEIC" })).toBeDisabled();
  });

  it("goes back to the library health from the crumb", async () => {
    mockRepairDefaults();
    mockCommand("get_repair_items", () => ({ items: [], nextCursor: null }));
    const onBack = vi.fn();
    const status = { ...IDLE, phase: "done", counts: { ...IDLE.counts, safe: 4 } } as never;
    renderRepair(<SourceResult status={status} name="DCIM" onBack={onBack} restoring={false} />);
    await userEvent.click(await screen.findByRole("button", { name: /Library health/ }));
    expect(onBack).toHaveBeenCalled();
  });
});
