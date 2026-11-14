import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { DENOISE_FINISHED, DENOISE_PROGRESS } from "@/lib/events";
import { emitMock, invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { useDenoise } from "./use-denoise";

const status = (over: object = {}) => ({ available: true, reason: null, model: "m@1", ready: false, running: false, gpu: true, ...over });

describe("useDenoise", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("denoise_status", () => status());
    mockCommand("start_denoise", () => null);
    mockCommand("cancel_denoise", () => null);
  });

  it("starts, follows this photo's progress and reports the finish", async () => {
    const onFinished = vi.fn();
    const { result } = renderHook(() => useDenoise(7, onFinished), { wrapper: Providers });
    await waitFor(() => expect(result.current.status?.available).toBe(true));
    await act(async () => result.current.start());
    expect(invokeMock).toHaveBeenCalledWith("start_denoise", { assetId: 7 });
    expect(result.current.progress).toBe(0);
    act(() => emitMock(DENOISE_PROGRESS, { assetId: 8, done: 7, total: 8 }));
    expect(result.current.progress).toBe(0);
    act(() => emitMock(DENOISE_PROGRESS, { assetId: 7, done: 2, total: 8 }));
    expect(result.current.progress).toBe(0.25);
    act(() => emitMock(DENOISE_FINISHED, { assetId: 7, ok: true, model: "m@1", onGpu: true, error: null }));
    expect(result.current.progress).toBeNull();
    expect(onFinished).toHaveBeenCalledWith(expect.objectContaining({ ok: true, model: "m@1" }));
  });

  it("cancels, and a failure is kept to show", async () => {
    const { result } = renderHook(() => useDenoise(7, vi.fn()), { wrapper: Providers });
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => result.current.start());
    await act(async () => result.current.cancel());
    expect(invokeMock).toHaveBeenCalledWith("cancel_denoise", { assetId: 7 });
    expect(result.current.progress).toBeNull();
    await act(async () => result.current.start());
    act(() => emitMock(DENOISE_FINISHED, { assetId: 7, ok: false, model: "m@1", onGpu: false, error: "The GPU ran out of memory." }));
    expect(result.current.error).toBe("The GPU ran out of memory.");
  });

  it("says it is slow after 90 s without progress, and a progress event clears it", async () => {
    const { result } = renderHook(() => useDenoise(7, vi.fn()), { wrapper: Providers });
    await waitFor(() => expect(result.current.status).not.toBeNull());
    vi.useFakeTimers();
    try {
      await act(async () => result.current.start());
      act(() => vi.advanceTimersByTime(89_000));
      expect(result.current.slow).toBe(false);
      act(() => vi.advanceTimersByTime(1_000));
      expect(result.current.slow).toBe(true);
      expect(result.current.progress).toBe(0);
      act(() => emitMock(DENOISE_PROGRESS, { assetId: 7, done: 1, total: 8 }));
      expect(result.current.slow).toBe(false);
      act(() => vi.advanceTimersByTime(90_000));
      expect(result.current.slow).toBe(true);
      act(() => emitMock(DENOISE_FINISHED, { assetId: 7, ok: true, model: "m@1", onGpu: true, error: null }));
      expect(result.current.slow).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
