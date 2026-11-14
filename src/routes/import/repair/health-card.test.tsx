import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { HealthCard } from "./health-card";
import { IDLE, mockRepairDefaults, renderRepair } from "./test-helpers";

describe("HealthCard", () => {
  it("counts the files that open, lists the parts and starts the check of the library", async () => {
    mockRepairDefaults({ health: { total: 100, unreadable: 3, missing: 2, recovered: 1, checkedAt: null } });
    renderRepair(<HealthCard repair={IDLE} />);
    expect(await screen.findByText("97 of 100 open fine")).toBeInTheDocument();
    expect(screen.getByText("Not checked yet")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Check the library" }));
    expect(invokeMock.mock.calls.map((c) => c[0])).toContain("start_check_library");
  });

  it("shows the phase and Cancel while a repair runs", async () => {
    mockRepairDefaults();
    mockCommand("cancel_repair", () => null);
    renderRepair(<HealthCard repair={{ ...IDLE, phase: "probing", done: 5, total: 10, current: "IMG_1.HEIC" }} />);
    expect(await screen.findByRole("status")).toHaveTextContent("Testing files · IMG_1.HEIC");
    expect(screen.getByRole("progressbar", { name: "Progress" })).toHaveAttribute("aria-valuenow", "50");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(invokeMock.mock.calls.map((c) => c[0])).toContain("cancel_repair");
  });
});
