import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { addToDevelop, clearDevelop, lastDevelopPhoto } from "@/lib/develop-source";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { useMcpScope } from "./use-mcp-scope";

const scopeCalls = () => invokeMock.mock.calls.filter(([cmd]) => cmd === "mcp_set_scope").map(([, args]) => args);

describe("useMcpScope", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("strip_ids", () => [4, 5, 6]);
    mockCommand("mcp_set_scope", () => null);
    clearDevelop();
    lastDevelopPhoto.set(null);
  });

  it("sends nothing while off", async () => {
    addToDevelop([4, 5, 6]);
    renderHook(() => useMcpScope(false), { wrapper: Providers });
    await new Promise((r) => setTimeout(r, 20));
    expect(scopeCalls()).toEqual([]);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === "strip_ids")).toBe(false);
  });

  it("sends the strip and the open photo, and empties it when turned off", async () => {
    const { rerender } = renderHook(({ on }) => useMcpScope(on), { wrapper: Providers, initialProps: { on: true } });
    await waitFor(() => expect(scopeCalls()).toEqual([{ ids: [], open: null }]));
    act(() => {
      addToDevelop([4, 5, 6]);
      lastDevelopPhoto.set(5);
    });
    await waitFor(() => expect(scopeCalls().at(-1)).toEqual({ ids: [4, 5, 6], open: 5 }));
    act(() => lastDevelopPhoto.set(9));
    await waitFor(() => expect(scopeCalls().at(-1)).toEqual({ ids: [4, 5, 6], open: null }));
    rerender({ on: false });
    await waitFor(() => expect(scopeCalls().at(-1)).toEqual({ ids: [], open: null }));
  });
});
