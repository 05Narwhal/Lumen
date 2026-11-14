import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { vi } from "vitest";
import { lerpNumber, useTween } from "./use-tween";

describe("useTween", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "performance", "setTimeout"] });
    delete document.documentElement.dataset.reducedMotion;
  });
  afterEach(() => vi.useRealTimers());

  it("starts where it is and glides to a new target", () => {
    const { result, rerender } = renderHook(({ v }) => useTween(v, lerpNumber, 200), { initialProps: { v: 0 } });
    expect(result.current).toBe(0);
    rerender({ v: 100 });
    expect(result.current).toBe(0);
    act(() => vi.advanceTimersByTime(100));
    expect(result.current).toBeGreaterThan(0);
    expect(result.current).toBeLessThan(100);
    act(() => vi.advanceTimersByTime(200));
    expect(result.current).toBe(100);
  });

  it("jumps when told to (a drag follows the pointer)", () => {
    const { result, rerender } = renderHook(({ v, jump }) => useTween(v, lerpNumber, 200, jump), { initialProps: { v: 0, jump: false } });
    rerender({ v: 40, jump: true });
    expect(result.current).toBe(40);
  });

  it("jumps when motion is reduced", () => {
    document.documentElement.dataset.reducedMotion = "true";
    const { result, rerender } = renderHook(({ v }) => useTween(v, lerpNumber, 200), { initialProps: { v: 0 } });
    rerender({ v: 70 });
    expect(result.current).toBe(70);
  });

  it("asks for no frames while the target stays put", () => {
    const raf = vi.spyOn(window, "requestAnimationFrame");
    const { rerender } = renderHook(({ v }) => useTween(v, lerpNumber, 200), { initialProps: { v: 5 } });
    rerender({ v: 5 });
    expect(raf).not.toHaveBeenCalled();
  });
});
