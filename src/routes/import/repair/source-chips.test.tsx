import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { mockCommand } from "@/test/tauri-mock";
import { SourceChips } from "./source-chips";
import { renderRepair } from "./test-helpers";

describe("SourceChips", () => {
  it("offers a chip per source, turns off one that is not ready and puts Restore…'s folders first", async () => {
    const onCheck = vi.fn();
    mockCommand("list_repair_sources", () => [
      { ref: { kind: "volume", id: "E:/" }, name: "EOS_DIGITAL", detail: "58 GB", ready: true, removable: true },
      { ref: { kind: "iphone", id: "u1" }, name: "An iPhone", detail: "Unlock the phone", ready: false, removable: false },
    ]);
    renderRepair(<SourceChips onCheck={onCheck} disabled={false} restoring={{ ids: [1], folders: [{ path: "E:/DCIM", files: 1 }] }} />);
    expect(await screen.findByRole("button", { name: /EOS_DIGITAL/ })).toBeEnabled();
    const chips = screen.getAllByRole("button");
    expect(chips[0]).toHaveTextContent("Look in E:/DCIM");
    expect(screen.getByRole("button", { name: /An iPhone/ })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: /EOS_DIGITAL/ }));
    expect(onCheck).toHaveBeenCalledWith({ kind: "volume", id: "E:/" }, "EOS_DIGITAL");
  });
});
