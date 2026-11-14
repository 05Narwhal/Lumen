import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { api } from "@/lib/api";
import { previewState } from "@/dev/browser-shim";
import type { InstallState } from "@/types";

const GB = 1024 ** 3;

function mockState(over: Partial<InstallState> = {}) {
  const state: InstallState = { ...previewState(), installed: { kind: "none" }, lumenRunning: false, ...over };
  vi.spyOn(api, "getInstallState").mockResolvedValue(state);
  vi.spyOn(api, "checkFolder").mockResolvedValue({ kind: "ok", free: 214 * GB });
  vi.spyOn(api, "onProgress").mockResolvedValue(() => {});
  vi.spyOn(api, "onLumen").mockResolvedValue(() => {});
  return state;
}

beforeEach(() => vi.restoreAllMocks());

describe("App", () => {
  it("starts on Welcome for a PC without Lumen and walks Options → Welcome", async () => {
    mockState();
    render(<App />);
    expect(await screen.findByText("Your photos, on your machine, fast.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Options…" }));
    expect(await screen.findByText("Install options")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "Install" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(await screen.findByText("Your photos, on your machine, fast.")).toBeInTheDocument();
  });

  it("installs with the defaults and ends on Done", async () => {
    mockState();
    const install = vi.spyOn(api, "install").mockResolvedValue({ models: "later" });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Install" }));
    expect(await screen.findByText("Lumen is ready")).toBeInTheDocument();
    expect(install).toHaveBeenCalledWith(
      expect.objectContaining({ desktopShortcut: true, startWithWindows: false, downloadModels: true, mode: "install" }),
    );
    expect(screen.getByText(/finish downloading the next time Lumen is open/)).toBeInTheDocument();
  });

  it("shows Try again, not Done, when the installer fails", async () => {
    mockState();
    vi.spyOn(api, "install").mockRejectedValue("The installer stopped with code 2. Nothing was changed; try again.");
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Install" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("code 2");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("opens on Already installed over an older install and updates in that folder", async () => {
    mockState({ installed: { kind: "older", version: "26.1.0-alpha", location: "D:\\Apps\\Lumen" }, lumenRunning: true });
    const install = vi.spyOn(api, "install").mockResolvedValue({ models: "complete" });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Update" }));
    await screen.findByText("Lumen is ready");
    expect(install).toHaveBeenCalledWith(expect.objectContaining({ folder: "D:\\Apps\\Lumen", mode: "update" }));
  });

  it("Remove… leads to the Remove screen, and Cancel comes back", async () => {
    mockState({ installed: { kind: "same", version: "26.1.2-alpha", location: "C:\\Lumen" } });
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Remove…" }));
    expect(await screen.findByText("Remove Lumen from this PC?")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByText("Lumen is already installed")).toBeInTheDocument();
  });

  it("waits while Lumen closes; Cancel stops the wait and closes Setup", async () => {
    mockState({ installed: { kind: "older", version: "26.1.0-alpha", location: "D:\\Apps\\Lumen" }, lumenRunning: true });
    let tell: (c: "closing" | "asking-import" | "closed") => void = () => {};
    vi.spyOn(api, "onLumen").mockImplementation(async (h) => { tell = h; return () => {}; });
    vi.spyOn(api, "install").mockReturnValue(new Promise(() => {}));
    const cancelWait = vi.spyOn(api, "cancelWait").mockResolvedValue();
    const close = vi.spyOn(api, "close").mockResolvedValue();
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Update" }));
    act(() => tell("closing"));
    expect(await screen.findByRole("status")).toHaveTextContent("Closing Lumen…");
    act(() => tell("asking-import"));
    expect(screen.getByRole("status")).toHaveTextContent("Answer there and Setup continues.");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(cancelWait).toHaveBeenCalledOnce();
    expect(close).toHaveBeenCalledOnce();
  });

  it("opens on Remove when started from Installed apps, and Cancel closes Setup", async () => {
    mockState({ installed: { kind: "same", version: "26.1.2-alpha", location: "C:\\Lumen" }, removeOnly: true });
    const close = vi.spyOn(api, "close").mockResolvedValue();
    render(<App />);
    expect(await screen.findByText("Remove Lumen from this PC?")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(close).toHaveBeenCalledOnce();
  });

  it("goes back to the question with the reason when removal fails", async () => {
    mockState({ installed: { kind: "same", version: "26.1.2-alpha", location: "C:\\Lumen" } });
    vi.spyOn(api, "uninstall").mockRejectedValue("Lumen's uninstaller stopped with code 2. Lumen is still installed; try again.");
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Remove…" }));
    await userEvent.click(await screen.findByRole("button", { name: "Remove Lumen" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("code 2");
    expect(screen.getByRole("button", { name: "Remove Lumen" })).toBeInTheDocument();
  });

  it("removes with the box off unless the person ticks it", async () => {
    mockState({ installed: { kind: "same", version: "26.1.2-alpha", location: "C:\\Lumen" } });
    const uninstall = vi.spyOn(api, "uninstall").mockResolvedValue([]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Remove…" }));
    await userEvent.click(await screen.findByRole("button", { name: "Remove Lumen" }));
    expect(await screen.findByText("Lumen is removed.")).toBeInTheDocument();
    expect(uninstall).toHaveBeenCalledWith(false, null);
  });

  const LIB = "C:\\Users\\A\\Pictures\\Lumen Library";
  const detail = { name: "Lumen Library", path: LIB, photos: 3, bytes: 3_000, refused: null };

  it("deletes a library only after its name is typed, to the Recycle Bin when it fits", async () => {
    mockState({ installed: { kind: "same", version: "26.1.2-alpha", location: "C:\\Lumen" }, libraries: [LIB] });
    vi.spyOn(api, "libraryDetails").mockResolvedValue([detail]);
    const bin = vi.spyOn(api, "checkRecycleBin").mockResolvedValue({ fits: true, bytes: 3_000, room: 9_000 });
    const uninstall = vi.spyOn(api, "uninstall").mockResolvedValue([{ path: LIB, folderRemoved: true, problem: null }]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Remove…" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete libraries…" }));
    await userEvent.type(await screen.findByLabelText('Type "Lumen Library" to delete it'), "Lumen Library");
    await userEvent.click(screen.getByRole("button", { name: "Delete library" }));
    expect(bin).toHaveBeenCalledWith([LIB]);
    expect(await screen.findByRole("alert")).toHaveTextContent("Lumen Library will be moved to the Recycle Bin.");
    expect(uninstall).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Remove Lumen" }));
    expect(await screen.findByText("Lumen Library was moved to the Recycle Bin.")).toBeInTheDocument();
    expect(uninstall).toHaveBeenCalledWith(false, { paths: [LIB], mode: "recycle" });
  });

  it("asks before deleting for good when the library doesn't fit the Recycle Bin, and Undo clears the choice", async () => {
    mockState({ installed: { kind: "same", version: "26.1.2-alpha", location: "C:\\Lumen" }, libraries: [LIB] });
    vi.spyOn(api, "libraryDetails").mockResolvedValue([detail]);
    vi.spyOn(api, "checkRecycleBin").mockResolvedValue({ fits: false, bytes: 3_000, room: 1_000 });
    const uninstall = vi.spyOn(api, "uninstall").mockResolvedValue([]);
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Remove…" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete libraries…" }));
    await userEvent.type(await screen.findByLabelText('Type "Lumen Library" to delete it'), "Lumen Library");
    await userEvent.click(screen.getByRole("button", { name: "Delete library" }));
    await userEvent.click(await screen.findByRole("button", { name: "Delete for good" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Lumen Library will be deleted for good.");
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await userEvent.click(screen.getByRole("button", { name: "Remove Lumen" }));
    expect(await screen.findByText("Lumen is removed.")).toBeInTheDocument();
    expect(uninstall).toHaveBeenCalledWith(false, null);
  });
});
