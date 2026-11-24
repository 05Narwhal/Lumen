import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";

const { confirmMock, toastMock } = vi.hoisted(() => ({ confirmMock: vi.fn(), toastMock: vi.fn((_opts: unknown) => 7) }));
vi.mock("@/components/confirm-dialog", () => ({ confirm: confirmMock }));
vi.mock("@/components/toast", () => ({ toast: toastMock, dismissToast: vi.fn() }));

import { reviewTarget, runPluginTool } from "./plugin-tool";

const calls = (name: string) => invokeMock.mock.calls.filter(([c]) => c === name);
const model = (downloaded: boolean) => ({ id: "image", size: 610_000_000, licence: "CC-BY-NC-4.0", downloaded });
const finder = { id: "acme.finder", name: "Finder" };

describe("plugin tools from a contribution", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    confirmMock.mockReset();
    toastMock.mockClear();
  });

  it("asks before the download, repeats the tool while it works, then offers the review", async () => {
    confirmMock.mockResolvedValue(true);
    mockCommand("plugin_models_status", () => [model(false)]);
    mockCommand("plugin_models_download", () => [model(true)]);
    const answers = [{ status: "working" }, { status: "working" }, { status: "done", suggested: 12, review: "/review/places?plugin=acme.finder" }];
    mockCommand("plugin_tool_run", () => answers.shift());
    const navigate = vi.fn();
    await runPluginTool(finder, "find", { scope: "album", album: 4 }, navigate);
    expect(confirmMock).toHaveBeenCalledOnce();
    expect((confirmMock.mock.calls[0][0] as { title: string }).title).toContain("Finder");
    expect(calls("plugin_models_download")).toHaveLength(1);
    expect(calls("plugin_tool_run")).toHaveLength(3);
    expect(calls("plugin_tool_run")[0][1]).toMatchObject({ pluginId: "acme.finder", tool: "find", args: { scope: "album", album: 4 } });
    const offer = toastMock.mock.calls.at(-1)![0] as { message: string; action: { run: () => void } };
    expect(offer.message).toContain("12");
    offer.action.run();
    expect(navigate).toHaveBeenCalledWith("/review/places?plugin=acme.finder");
  });

  it("runs nothing when the person declines the download", async () => {
    confirmMock.mockResolvedValue(false);
    mockCommand("plugin_models_status", () => [model(false)]);
    await runPluginTool(finder, "find", { scope: "library" }, vi.fn());
    expect(calls("plugin_models_download")).toHaveLength(0);
    expect(calls("plugin_tool_run")).toHaveLength(0);
  });

  it("shows the plugin's own line when it has nothing to review", async () => {
    mockCommand("plugin_models_status", () => []);
    mockCommand("plugin_tool_run", () => ({ status: "done", suggested: 0, message: "Every photo already has a place." }));
    await runPluginTool(finder, "find", { scope: "selection" }, vi.fn());
    expect(toastMock.mock.calls.at(-1)![0]).toEqual({ message: "Every photo already has a place." });
  });

  it("says why when the tool fails", async () => {
    mockCommand("plugin_models_status", () => [model(true)]);
    mockCommand("plugin_tool_run", () => {
      throw { code: "plugin_missing", message: "acme.finder isn't installed or isn't turned on" };
    });
    await runPluginTool(finder, "find", { scope: "library" }, vi.fn());
    expect(confirmMock).not.toHaveBeenCalled();
    expect(toastMock.mock.calls.at(-1)![0]).toMatchObject({ message: "acme.finder isn't installed or isn't turned on" });
  });

  it("opens only a review page, and always the calling plugin's", () => {
    expect(reviewTarget("acme.finder", "/review/places?plugin=other.plugin")).toBe("/review/places?plugin=acme.finder");
    expect(reviewTarget("acme.finder", "dates")).toBe("/review/dates?plugin=acme.finder");
    expect(reviewTarget("acme.finder", "/settings")).toBeNull();
    expect(reviewTarget("acme.finder", "/review/../x")).toBeNull();
    expect(reviewTarget("acme.finder", undefined)).toBeNull();
  });
});
