import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useOpenSet } from "./open-set";

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("useOpenSet", () => {
  it("starts from the default until something is toggled, then remembers", () => {
    const { result, unmount } = renderHook(() => useOpenSet("t.open", () => ["1", "2"]));
    expect(result.current.has("1")).toBe(true);
    act(() => result.current.toggle("1"));
    expect(result.current.has("1")).toBe(false);
    expect(result.current.has("2")).toBe(true);
    unmount();
    // A new default no longer applies: what was stored wins.
    const again = renderHook(() => useOpenSet("t.open", () => ["1", "2", "3"]));
    expect([...again.result.current.ids].sort()).toEqual(["2"]);
  });

  it("follows a default that arrives late", () => {
    let defaults: string[] = [];
    const { result, rerender } = renderHook(() => useOpenSet("t.late", () => defaults));
    expect(result.current.has("5")).toBe(false);
    defaults = ["5"];
    rerender();
    expect(result.current.has("5")).toBe(true);
  });

  it("set() opens and closes, and survives a refusing storage", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("refused");
    });
    const { result } = renderHook(() => useOpenSet("t.refuse"));
    act(() => result.current.set("a", true));
    expect(result.current.has("a")).toBe(true);
    act(() => result.current.set("a", false));
    expect(result.current.has("a")).toBe(false);
  });
});
