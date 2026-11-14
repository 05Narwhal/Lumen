import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { emitMock, invokeMock, mockCommand } from "@/test/tauri-mock";
import { usePlugins } from "./use-plugins";

const calls = () => invokeMock.mock.calls.filter(([c]) => c === "list_plugins").length;

describe("usePlugins", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockCommand("list_plugins", () => []);
    mockCommand("get_plugin_developer_mode", () => true);
  });

  it("refetches_on_changed", async () => {
    renderHook(() => usePlugins());
    await waitFor(() => expect(calls()).toBe(1));
    await act(async () => emitMock("plugins://changed", null));
    await waitFor(() => expect(calls()).toBe(2));
  });

  it("developer_mode_comes_from_the_backend: on with no dev plugin loaded", async () => {
    const { result } = renderHook(() => usePlugins());
    await waitFor(() => expect(result.current.developerMode).toBe(true));
  });
});
