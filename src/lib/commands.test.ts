import { describe, expect, it, vi } from "vitest";
import type { QueryClient } from "@tanstack/react-query";
import { autoRequests, retouchRequests } from "@/components/develop-panels";
import { queryKeys } from "@/lib/query-keys";
import { toastStore } from "@/components/toast/store";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore } from "@/lib/develop-source";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { closePluginInstall, pluginInstallStore } from "@/components/plugin-install/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { appCommands, type AppCommand } from "./commands";
import { petBoxesRequest } from "./face-tagging";
import { textOverlayRequest } from "./photo-text";

const run = (id: string, navigate = vi.fn()) => {
  const c = appCommands.find((x) => x.id === id) as AppCommand;
  expect(c, id).toBeDefined();
  c.run({ navigate, qc: {} as QueryClient, update: vi.fn() });
  return navigate;
};

describe("palette commands", () => {
  it("palette_install_entry: asks for a .lumenplugin file, then opens the sheet; cancel does nothing", async () => {
    closePluginInstall();
    expect((appCommands.find((c) => c.id === "install-plugin") as AppCommand).label).toBe("Install plugin from file…");
    vi.mocked(openDialog).mockResolvedValueOnce(null);
    run("install-plugin");
    await vi.waitFor(() => expect(openDialog).toHaveBeenCalled());
    expect(vi.mocked(openDialog).mock.calls.at(-1)?.[0]).toMatchObject({ filters: [{ extensions: ["lumenplugin"] }] });
    expect(pluginInstallStore.get().path).toBeNull();
    vi.mocked(openDialog).mockResolvedValueOnce("C:/x/a.lumenplugin");
    run("install-plugin");
    await vi.waitFor(() => expect(pluginInstallStore.get().path).toBe("C:/x/a.lumenplugin"));
    closePluginInstall();
  });

  it("offers the text in a photo only while reading text is on (Round 13-3)", async () => {
    const qcWith = (readText: boolean) => ({ getQueryData: (k: unknown) => (JSON.stringify(k) === JSON.stringify(queryKeys.settings) ? { smart: { enabled: true, experimental: { readText } } } : undefined) }) as unknown as QueryClient;
    const ids = ["show-text", "copy-text", "filter-has-text"];
    const cmds = ids.map((id) => appCommands.find((c) => c.id === id) as AppCommand);
    expect(cmds.map((c) => c.label)).toEqual(["Show text in photo", "Copy text from photo", "Filter: has text"]);
    expect(cmds[0].shortcut).toBe("t");
    expect(cmds.every((c) => c.available?.(qcWith(true)))).toBe(true);
    expect(cmds.some((c) => c.available?.(qcWith(false)))).toBe(false);

    window.history.replaceState(null, "", "/library?asset=42");
    run("show-text");
    expect(textOverlayRequest.get()).toBe(42);
    textOverlayRequest.set(null);

    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    mockCommand("asset_text", () => [{ ord: 0, text: "Gostilna", quad: [] }, { ord: 1, text: "Kosilo", quad: [] }]);
    run("copy-text");
    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith("Gostilna\nKosilo"));

    const navigate = run("filter-has-text");
    expect(String(navigate.mock.calls[0][0])).toMatch(/^\/library\?tags=/);
    expect(decodeURIComponent(String(navigate.mock.calls[0][0]))).toContain('"type":"hasText"');
  });

  it("offers Reload window and Restart Lumen, which asks for the screen in view", async () => {
    expect(appCommands.map((c) => c.label)).toEqual(expect.arrayContaining(["Reload window", "Restart Lumen"]));
    mockCommand("restart_lumen", () => null);
    window.history.replaceState(null, "", "/library?asset=4");
    run("restart-lumen");
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith("restart_lumen", { location: "/library?asset=4" }));
  });

  it("asks the open Develop screen for Auto", () => {
    const before = autoRequests.get();
    run("develop-auto");
    expect(autoRequests.get()).toBe(before + 1);
    expect(appCommands.find((c) => c.id === "develop-auto")?.shortcut).toBe("Mod+U");
  });

  it("asks the open Develop screen for Retouch, offered only with its Experimental switch on", () => {
    const before = retouchRequests.get();
    run("develop-retouch");
    expect(retouchRequests.get()).toBe(before + 1);
    const c = appCommands.find((x) => x.id === "develop-retouch") as AppCommand;
    expect(c.shortcut).toBe("Mod+Shift+U");
    const prism = [{ task: "retouch", models: [{ id: "lumen-prism-1.0.1-d" }] }];
    const qc = (retouch: boolean | null, catalog: unknown = prism) =>
      ({
        getQueryData: (k: unknown) => {
          if (JSON.stringify(k) === JSON.stringify(queryKeys.experimentalCatalog)) return catalog ?? undefined;
          return JSON.stringify(k) === JSON.stringify(queryKeys.settings) && retouch !== null ? { smart: { experimental: { autoModel: false, retouch } } } : undefined;
        },
      }) as unknown as QueryClient;
    expect(c.available?.(qc(true))).toBe(true);
    expect(c.available?.(qc(false))).toBe(false);
    expect(c.available?.(qc(null))).toBe(false);
    // Round 14: no Retouch photo runs until a Lumen Prism model is installed.
    expect(c.available?.(qc(true, [{ task: "retouch", models: [] }]))).toBe(false);
    expect(c.available?.(qc(true, null))).toBe(false); // not read yet
  });

  it("asks the open viewer to show its pets' boxes (Round 8B)", () => {
    window.history.replaceState(null, "", "/library?asset=7");
    petBoxesRequest.set(null);
    run("show-pets");
    expect(petBoxesRequest.get()).toBe(7);
    expect(appCommands.find((c) => c.id === "show-pets")?.shortcut).toBe("Shift+P");
  });

  it("clears Develop's filmstrip, with Undo", () => {
    toastStore.set({ toasts: [] });
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: [1, 2], label: "Selection" });
    const navigate = run("develop-clear-filmstrip");
    expect(developSourceStore.get().only).toEqual([]);
    expect(navigate).toHaveBeenCalledWith("/develop");
    toastStore.get().toasts[0].action?.run();
    expect(developSourceStore.get().only).toEqual([1, 2]);
  });
});
