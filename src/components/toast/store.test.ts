import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dismissToast, toast, toastStore } from "./store";

describe("toast store", () => {
  beforeEach(() => {
    toastStore.set({ toasts: [] });
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  it("auto-dismisses after 6s by default", () => {
    toast({ message: "Moved to Trash" });
    expect(toastStore.get().toasts).toHaveLength(1);
    vi.advanceTimersByTime(5999);
    expect(toastStore.get().toasts).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(toastStore.get().toasts).toHaveLength(0);
  });

  it("keeps at most three and dismisses by id", () => {
    const ids = [1, 2, 3, 4].map((n) => toast({ message: `t${n}` }));
    expect(toastStore.get().toasts.map((t) => t.message)).toEqual(["t2", "t3", "t4"]);
    dismissToast(ids[3]);
    expect(toastStore.get().toasts.map((t) => t.message)).toEqual(["t2", "t3"]);
  });
});
