import { describe, expect, it, vi } from "vitest";
import { createStore } from "./store";

describe("createStore", () => {
  it("updates and notifies", () => {
    const s = createStore({ n: 1 });
    const fn = vi.fn();
    const off = s.subscribe(fn);
    s.set((p) => ({ n: p.n + 1 }));
    expect(s.get().n).toBe(2);
    expect(fn).toHaveBeenCalledTimes(1);
    off();
    s.set({ n: 5 });
    expect(fn).toHaveBeenCalledTimes(1);
  });
});
