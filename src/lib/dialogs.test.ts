import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";

// The global test setup replaces the module with a stand-in; this file tests the real one.
const load = () => vi.importActual<typeof import("./dialogs")>("./dialogs");

describe("dialogs", () => {
  beforeEach(() => {
    invokeMock.mockClear();
  });

  it("maps options to a dialog_pick request", async () => {
    const real = await load();
    mockCommand("dialog_pick", () => ["C:/a.jpg"]);
    const path = await real.pickFile({ title: "Pick", defaultPath: "C:/Pics", filters: [{ name: "Images", extensions: ["jpg"] }] });
    expect(path).toBe("C:/a.jpg");
    expect(invokeMock).toHaveBeenCalledWith("dialog_pick", {
      req: { kind: "open-file", title: "Pick", start: "C:/Pics", name: null, filters: [{ label: "Images", extensions: ["jpg"] }] },
    });
  });

  it("uses the kind for each helper and keeps name for save only", async () => {
    const real = await load();
    mockCommand("dialog_pick", () => ["x"]);
    await real.pickFiles();
    await real.pickFolder();
    await real.pickSave({ name: "out.png" });
    await real.pickFile({ title: "t" });
    const kinds = invokeMock.mock.calls.map((c) => (c[1] as { req: { kind: string; name: string | null } }).req);
    expect(kinds.map((r) => r.kind)).toEqual(["open-multiple", "open-folder", "save", "open-file"]);
    expect(kinds[2].name).toBe("out.png");
    expect(kinds[3].name).toBeNull();
  });

  it("cancelled resolves to null; many paths stay a list", async () => {
    const real = await load();
    mockCommand("dialog_pick", () => null);
    expect(await real.pickFolder()).toBeNull();
    expect(await real.pickFiles()).toBeNull();
    mockCommand("dialog_pick", () => ["a", "b"]);
    expect(await real.pickFiles()).toEqual(["a", "b"]);
  });
});
