import { listen } from "@tauri-apps/api/event";
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AI_FINISHED, AI_PROGRESS } from "@/lib/events";
import { stageLabel } from "@/lib/ai-stages";
import { emitMock, invokeMock, mockCommand } from "@/test/tauri-mock";
import type { AiButtonState } from "@/components/ai-button";
import { useAiTask } from "./use-ai-task";

/** A render-by-render record of the state, so "never visible" is checked over time. */
function mount() {
  const seen: (AiButtonState | null)[] = [];
  const hook = renderHook(() => {
    const t = useAiTask<string>("auto");
    seen.push(t.state);
    return t;
  });
  return { ...hook, seen };
}

/** A fn that resolves after `ms` and remembers the task id it was given. */
function slow(ms: number, value = "ok") {
  const ids: string[] = [];
  const fn = vi.fn(async (task: string) => {
    ids.push(task);
    await new Promise((r) => setTimeout(r, ms));
    return value;
  });
  return { fn, ids };
}

describe("stageLabel", () => {
  it("names each stage and falls back to Working…", () => {
    expect(stageLabel("reading", 0, 0)).toBe("Reading photo…");
    expect(stageLabel("trying", 2, 3)).toBe("Trying looks 2/3");
    expect(stageLabel("predicting", 0, 0)).toBe("Using the model…");
    expect(stageLabel("applying", 0, 0)).toBe("Applying…");
    expect(stageLabel("working", 0, 0)).toBe("Working…");
    expect(stageLabel("something-new", 1, 2)).toBe("Working…");
  });
});

describe("useAiTask", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockCommand("ai_cancel", () => true);
  });
  afterEach(() => vi.useRealTimers());

  it("never shows anything for an action that finishes inside the delay", async () => {
    const { result, seen } = mount();
    const { fn } = slow(120);
    let out: string | null = null;
    await act(async () => {
      const p = result.current.run(fn);
      await vi.advanceTimersByTimeAsync(120);
      out = await p;
    });
    expect(out).toBe("ok");
    expect(seen.some((s) => s?.visible)).toBe(false);
    expect(result.current.state).toBeNull();
    await act(async () => void (await vi.advanceTimersByTimeAsync(500)));
    expect(seen.some((s) => s?.visible)).toBe(false);
  });

  it("shows the latest stage once the delay has passed, from its own task's events only", async () => {
    const { result } = mount();
    const { fn, ids } = slow(600);
    let p!: Promise<string | null>;
    await act(async () => {
      p = result.current.run(fn);
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.state).toEqual({ visible: false, stage: "Working…", done: 0, total: 0 });
    act(() => emitMock(AI_PROGRESS, { task: ids[0], feature: "auto", stage: "reading", done: 0, total: 0 }));
    await act(async () => void (await vi.advanceTimersByTimeAsync(260)));
    expect(result.current.state).toEqual({ visible: true, stage: "Reading photo…", done: 0, total: 0 });
    act(() => emitMock(AI_PROGRESS, { task: ids[0], feature: "auto", stage: "trying", done: 2, total: 3 }));
    act(() => emitMock(AI_PROGRESS, { task: "someone-else", feature: "auto", stage: "applying", done: 0, total: 0 }));
    expect(result.current.state).toEqual({ visible: true, stage: "Trying looks 2/3", done: 2, total: 3 });
    await act(async () => void (await vi.advanceTimersByTimeAsync(400)));
    await expect(p).resolves.toBe("ok");
    expect(result.current.state).toBeNull();
  });

  it("ignores a result that arrives after Cancel and goes back to idle", async () => {
    const { result } = mount();
    const { fn, ids } = slow(600);
    let p!: Promise<string | null>;
    await act(async () => {
      p = result.current.run(fn);
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(result.current.state?.visible).toBe(true);
    await act(async () => {
      await result.current.cancel();
    });
    expect(invokeMock).toHaveBeenCalledWith("ai_cancel", { task: ids[0] });
    expect(result.current.state).toBeNull();
    await act(async () => void (await vi.advanceTimersByTimeAsync(400)));
    await expect(p).resolves.toBeNull();
    expect(result.current.state).toBeNull();
  });

  it("treats a finished event that says cancelled as a cancel", async () => {
    const { result } = mount();
    const { fn, ids } = slow(100);
    let p!: Promise<string | null>;
    await act(async () => {
      p = result.current.run(fn);
      await vi.advanceTimersByTimeAsync(10);
    });
    act(() => emitMock(AI_FINISHED, { task: ids[0], feature: "auto", ok: false, cancelled: true }));
    await act(async () => void (await vi.advanceTimersByTimeAsync(100)));
    await expect(p).resolves.toBeNull();
  });

  it("ignores a second run while one is running", async () => {
    const { result } = mount();
    const first = slow(300);
    const second = slow(300, "second");
    let p1!: Promise<string | null>;
    let r2: string | null = "unset";
    await act(async () => {
      p1 = result.current.run(first.fn);
      await vi.advanceTimersByTimeAsync(10);
      r2 = await result.current.run(second.fn);
    });
    expect(r2).toBeNull();
    expect(second.fn).not.toHaveBeenCalled();
    await act(async () => void (await vi.advanceTimersByTimeAsync(400)));
    await expect(p1).resolves.toBe("ok");
  });

  it("can run again after a run has finished", async () => {
    const { result } = mount();
    await act(async () => {
      const p = result.current.run(slow(10).fn);
      await vi.advanceTimersByTimeAsync(10);
      await p;
    });
    let again: string | null = null;
    await act(async () => {
      const p = result.current.run(slow(10, "again").fn);
      await vi.advanceTimersByTimeAsync(10);
      again = await p;
    });
    expect(again).toBe("again");
  });

  it("stops listening when it unmounts", async () => {
    const offs: ReturnType<typeof vi.fn>[] = [];
    vi.mocked(listen).mockImplementation(async () => {
      const off = vi.fn();
      offs.push(off);
      return off;
    });
    const { result, unmount } = mount();
    await act(async () => {
      void result.current.run(slow(1000).fn);
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(offs).toHaveLength(2);
    expect(offs.every((o) => o.mock.calls.length === 0)).toBe(true);
    unmount();
    await vi.advanceTimersByTimeAsync(0);
    expect(offs.every((o) => o.mock.calls.length === 1)).toBe(true);
  });
});
