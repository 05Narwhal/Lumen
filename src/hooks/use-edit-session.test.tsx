import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand, mockLibraryDefaults, resetInvokeMock } from "@/test/tauri-mock";
import { IDENTITY, useEditSession } from "./use-edit-session";

function withQueryClient({ children }: { children: ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

const previews = () => invokeMock.mock.calls.filter(([c]) => c === "render_edit_preview");

describe("useEditSession", () => {
  beforeEach(() => {
    resetInvokeMock();
    mockLibraryDefaults();
    toastStore.set({ toasts: [] });
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => vi.useRealTimers());

  it("debounces preview renders to one per pause", async () => {
    const { result } = renderHook(() => useEditSession(5), { wrapper: withQueryClient });
    await waitFor(() => expect(result.current.loaded).toBe(true));
    const before = previews().length;
    act(() => {
      result.current.set({ straighten: 1 });
      result.current.set({ straighten: 2 });
      result.current.set({ straighten: 3 });
    });
    await act(async () => {
      vi.advanceTimersByTime(130);
    });
    const calls = previews().slice(before);
    expect(calls).toHaveLength(1);
    expect((calls[0][1] as { recipe: { straighten: number } }).recipe.straighten).toBe(3);
    await waitFor(() => expect(result.current.previewUrl).toContain("edit-preview/1"));
    expect(result.current.dirty).toBe(true);
  });

  it("sends the uncropped frame while the crop tool is open", async () => {
    mockCommand("get_edit", () => ({ ...IDENTITY, crop: { x: 0.1, y: 0.1, w: 0.5, h: 0.5 } }));
    const { result } = renderHook(() => useEditSession(5, { cropping: true }), { wrapper: withQueryClient });
    await waitFor(() => expect(result.current.saved).not.toBeNull());
    act(() => result.current.set({ straighten: 1 }));
    await act(async () => {
      vi.advanceTimersByTime(130);
    });
    const [, args] = previews().at(-1)!;
    expect((args as { recipe: { crop: unknown } }).recipe.crop).toBeNull();
    expect(result.current.draft.crop).not.toBeNull();
  });

  it("done saves, and Undo restores what was stored before", async () => {
    mockCommand("get_edit", () => null);
    const { result } = renderHook(() => useEditSession(5), { wrapper: withQueryClient });
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.set({ flip_v: true }));
    await act(() => result.current.done());
    expect(invokeMock).toHaveBeenCalledWith("save_edit", { assetId: 5, recipe: expect.objectContaining({ flip_v: true }) });
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Edit saved");
    // Undo of a first edit is a revert.
    await act(async () => toastStore.get().toasts.at(-1)!.action!.run());
    expect(invokeMock).toHaveBeenCalledWith("revert_edit", { assetId: 5 });
  });

  it("revert offers Undo that re-saves the removed recipe", async () => {
    const stored = { ...IDENTITY, flip_h: true };
    mockCommand("get_edit", () => stored);
    mockCommand("revert_edit", () => stored);
    const { result } = renderHook(() => useEditSession(5), { wrapper: withQueryClient });
    await waitFor(() => expect(result.current.saved).toEqual(stored));
    await act(() => result.current.revert());
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Reverted to original");
    await act(async () => toastStore.get().toasts.at(-1)!.action!.run());
    // Undo re-saves the removed recipe, after the revert (a refetch of the edit follows it).
    const order = invokeMock.mock.calls.map(([c]) => c);
    expect(order.lastIndexOf("save_edit")).toBeGreaterThan(order.lastIndexOf("revert_edit"));
    expect(invokeMock).toHaveBeenCalledWith("save_edit", { assetId: 5, recipe: stored });
  });

  it("cancel throws the draft away and reset returns one control to neutral", async () => {
    const stored = { ...IDENTITY, straighten: 5, rotate: 90 };
    mockCommand("get_edit", () => stored);
    const { result } = renderHook(() => useEditSession(5), { wrapper: withQueryClient });
    await waitFor(() => expect(result.current.saved).toEqual(stored));
    act(() => result.current.reset("straighten"));
    expect(result.current.draft.straighten).toBe(0);
    expect(result.current.draft.rotate).toBe(90);
    act(() => result.current.cancel());
    expect(result.current.draft).toEqual(stored);
    expect(result.current.dirty).toBe(false);
  });

  it("undoes and redoes, with a slider drag as one step", async () => {
    mockCommand("get_edit", () => null);
    const { result } = renderHook(() => useEditSession(5), { wrapper: withQueryClient });
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.canUndo).toBe(false);
    act(() => result.current.set({ rotate: 90 }));
    act(() => {
      result.current.set({ straighten: 1 }, "straighten");
      result.current.set({ straighten: 2 }, "straighten");
      result.current.set({ straighten: 3 }, "straighten");
    });
    act(() => result.current.undo());
    expect(result.current.draft.straighten).toBe(0);
    expect(result.current.draft.rotate).toBe(90);
    act(() => result.current.undo());
    expect(result.current.draft.rotate).toBe(0);
    expect(result.current.canUndo).toBe(false);
    act(() => result.current.redo());
    act(() => result.current.redo());
    expect(result.current.draft).toMatchObject({ rotate: 90, straighten: 3 });
    expect(result.current.canRedo).toBe(false);
    // A new change after an undo drops what could have been redone.
    act(() => result.current.undo());
    act(() => result.current.set({ flip_h: true }));
    expect(result.current.canRedo).toBe(false);
  });

  it("with client geometry, a turn or a crop needs no new render", async () => {
    mockCommand("get_edit", () => null);
    const { result } = renderHook(() => useEditSession(5, { clientGeometry: true }), { wrapper: withQueryClient });
    await waitFor(() => expect(result.current.loaded).toBe(true));
    await act(async () => {
      vi.advanceTimersByTime(130);
    });
    const before = previews().length;
    expect((previews().at(-1)![1] as { recipe: { rotate: number } }).recipe.rotate).toBe(0);
    act(() => result.current.set({ rotate: 90, crop: { x: 0, y: 0, w: 0.5, h: 0.5 } }));
    await act(async () => {
      vi.advanceTimersByTime(130);
    });
    expect(previews().length).toBe(before);
  });
});
