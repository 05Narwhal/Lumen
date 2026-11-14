import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invokeMock, mockCommand, mockLibraryDefaults, resetInvokeMock } from "@/test/tauri-mock";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import { useSlideshowEditor } from "./use-slideshow-editor";

function withQueryClient({ children }: { children: ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

const doc = (): SlideshowDoc => ({
  version: 1,
  resolution: [1920, 1080],
  fps: 30,
  audio: null,
  clips: [1, 2, 3].map((asset_id) => ({
    asset_id,
    extra: [],
    frame: "fill" as const,
    duration_ms: 4000,
    transition: { kind: "crossfade", duration_ms: 600 },
    motion: { kind: "none", from: [0, 0, 1], to: [0, 0, 1] },
  })),
});

const saves = () => invokeMock.mock.calls.filter(([c]) => c === "save_slideshow");
const order = (d: SlideshowDoc | null) => d?.clips.map((c) => c.asset_id);

async function mounted() {
  const hook = renderHook(() => useSlideshowEditor(3), { wrapper: withQueryClient });
  await waitFor(() => expect(hook.result.current.doc).not.toBeNull());
  return hook;
}

describe("useSlideshowEditor", () => {
  beforeEach(() => {
    resetInvokeMock();
    mockLibraryDefaults();
    mockCommand("get_slideshow", () => ({ id: 3, name: "Trip to Lisbon", doc: doc() }));
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => vi.useRealTimers());

  it("loads the document and saves nothing on its own", async () => {
    const { result } = await mounted();
    expect(result.current.name).toBe("Trip to Lisbon");
    expect(order(result.current.doc)).toEqual([1, 2, 3]);
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(saves()).toHaveLength(0);
  });

  it("saves once for a burst of edits", async () => {
    const { result } = await mounted();
    act(() => {
      result.current.updateClip(0, { duration_ms: 3000 });
      result.current.updateClip(0, { duration_ms: 2500 });
      result.current.rename("Lisbon");
    });
    await act(async () => {
      vi.advanceTimersByTime(700);
    });
    expect(saves()).toHaveLength(1);
    const args = saves()[0][1] as { name: string; doc: SlideshowDoc };
    expect(args.name).toBe("Lisbon");
    expect(args.doc.clips[0].duration_ms).toBe(2500);
  });

  it("moves a clip and keeps it selected", async () => {
    const { result } = await mounted();
    act(() => result.current.select(0));
    act(() => result.current.moveClip(0, 2));
    expect(order(result.current.doc)).toEqual([2, 3, 1]);
    expect(result.current.selected).toBe(2);
  });

  it("removes the selected clip and selects its neighbour", async () => {
    const { result } = await mounted();
    act(() => result.current.select(2));
    act(() => result.current.removeClip(2));
    expect(order(result.current.doc)).toEqual([1, 2]);
    expect(result.current.selected).toBe(1);
  });

  it("trims in 100 ms steps and never below half a second", async () => {
    const { result } = await mounted();
    act(() => result.current.trimClip(1, 1234));
    expect(result.current.doc?.clips[1].duration_ms).toBe(5200);
    act(() => result.current.trimClip(1, -99_000));
    expect(result.current.doc?.clips[1].duration_ms).toBe(500);
  });

  it("edits the document itself", async () => {
    const { result } = await mounted();
    act(() => result.current.update((d) => ({ ...d, fps: 60, resolution: [3840, 2160] })));
    expect(result.current.doc?.fps).toBe(60);
    await act(async () => {
      vi.advanceTimersByTime(700);
    });
    expect((saves()[0][1] as { doc: SlideshowDoc }).doc.resolution).toEqual([3840, 2160]);
  });
});
