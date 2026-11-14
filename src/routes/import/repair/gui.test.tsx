import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { RepairScreen } from "./gui";
import { IDLE, mockRepairDefaults, renderRepair } from "./test-helpers";

describe("RepairScreen", () => {
  it("checks a source from its chip, shows what it found and re-imports every row of the filter", async () => {
    let phase: "idle" | "done" = "idle";
    mockRepairDefaults();
    mockCommand("list_repair_sources", () => [
      { ref: { kind: "volume", id: "E:/" }, name: "EOS_DIGITAL", detail: "58 GB", ready: true, removable: true },
      { ref: { kind: "iphone", id: "udid-1" }, name: "An iPhone", detail: "Unlock the phone and tap Trust", ready: false, removable: false },
    ]);
    mockCommand("get_repair_status", () => ({ ...IDLE, phase, done: 9, total: 9, counts: { ...IDLE.counts, safe: 6, new: 3 } }));
    mockCommand("start_repair_check", () => {
      phase = "done";
      return null;
    });
    mockCommand("get_repair_items", () => ({
      items: [1, 2, 3].map((id) => ({ id, name: `IMG_${id}.HEIC`, dir: "100APPLE", status: "new", assetId: null, byteSize: 1000 })),
      nextCursor: null,
    }));
    mockCommand("start_repair_import", () => null);
    renderRepair(<RepairScreen />);
    expect(await screen.findByRole("button", { name: /An iPhone/ })).toBeDisabled();
    await userEvent.click(await screen.findByRole("button", { name: /EOS_DIGITAL/ }));
    expect(invokeMock).toHaveBeenCalledWith("start_repair_check", { source: { kind: "volume", id: "E:/" }, onlyAssets: null });
    expect(await screen.findByText("6 files are safely in the library.")).toBeInTheDocument();
    expect(await screen.findByText("IMG_2.HEIC")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
    await userEvent.click(screen.getByRole("button", { name: "Re-import selected (3)" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_repair_import", { selection: { all: true, status: "new", except: [] } }));
  });

  it("looks for a good copy of the files Restore… sent: the old folder first, only those files, every damaged row ticked", async () => {
    // The status is already "done" from an earlier check: it must not show before the person picks a source.
    mockRepairDefaults({ health: { unreadable: 2 } });
    mockCommand("unreadable_source_folders", (a) => (a?.ids ? [{ path: "E:/Card/DCIM", files: 2 }] : []));
    mockCommand("get_repair_status", () => ({ ...IDLE, phase: "done", done: 2, total: 2, counts: { ...IDLE.counts, damaged: 2 } }));
    mockCommand("start_repair_check", () => null);
    mockCommand("start_repair_import", () => null);
    mockCommand("get_repair_items", () => ({
      items: [7, 8].map((id) => ({ id, name: `IMG_${id}.JPG`, dir: "", status: "damaged", assetId: id, byteSize: 1000 })),
      nextCursor: null,
    }));
    renderRepair(<RepairScreen />, "/import?mode=repair&assets=7,8");
    expect(await screen.findByRole("heading", { name: "Where is a good copy of these 2 files?" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Result" })).not.toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: /Look in E:[\\/]Card[\\/]DCIM/ }));
    expect(invokeMock).toHaveBeenCalledWith("start_repair_check", { source: { kind: "folder", id: "E:/Card/DCIM" }, onlyAssets: [7, 8] });
    expect(await screen.findByText("IMG_8.JPG")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Re-import selected (2)" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("start_repair_import", { selection: { all: true, status: "damaged", except: [] } }));
  });

  it("says every photo opens fine when nothing is flagged, recovered or run", async () => {
    mockRepairDefaults();
    mockCommand("get_repair_status", () => IDLE);
    renderRepair(<RepairScreen />);
    expect(await screen.findByText("Every photo in this library opens fine.")).toBeInTheDocument();
    expect(await screen.findByText("100 of 100 open fine")).toBeInTheDocument();
  });
});
