import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { AttentionList } from "./attention-list";
import { mockRepairDefaults, renderRepair } from "./test-helpers";

const health = { total: 100, unreadable: 2, missing: 0, recovered: 0, checkedAt: null, causes: [{ cause: "incomplete", n: 1 }, { cause: "tooLarge", n: 1 }] };
const row = (id: number, cause: string) => ({ id, filename: `IMG_${id}.JPG`, relativePath: `2023/IMG_${id}.JPG`, extension: "jpg", byteSize: 1000, contentHash: "ab", cause });

describe("AttentionList", () => {
  it("lists flagged files with a cause and Recover, none for a file too large", async () => {
    mockRepairDefaults({ health });
    mockCommand("list_unreadable", () => ({ items: [row(1, "incomplete"), row(2, "tooLarge")], nextCursor: null }));
    mockCommand("start_recover", () => null);
    mockCommand("get_repair_status", () => ({}));
    renderRepair(<AttentionList health={health as never} sourceName={null} onRecoverStarted={vi.fn()} />);
    expect(await screen.findByText("File is incomplete")).toBeInTheDocument();
    expect(screen.getByText("Too large to open")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Recover" })).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "Recover what's readable (2)" }));
    expect(invokeMock).toHaveBeenCalledWith("start_recover", { ids: null });
  });

  it("keeps a recovered file", async () => {
    const none = { ...health, unreadable: 0 };
    mockRepairDefaults({ health: none });
    mockCommand("get_recovered", () => [{ assetId: 5, name: "IMG_5.JPG", kind: "partial", percent: 62, width: 4000, height: 3000 }]);
    mockCommand("keep_recovered", (a) => ({ done: a?.ids, failed: [] }));
    renderRepair(<AttentionList health={none as never} sourceName={null} onRecoverStarted={vi.fn()} />);
    expect(await screen.findByText("IMG_5.JPG")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Keep" }));
    expect(invokeMock).toHaveBeenCalledWith("keep_recovered", { ids: [5] });
  });
});
