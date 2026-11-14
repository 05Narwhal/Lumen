import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { useAssetText } from "./use-asset-text";

const on = { smart: { enabled: true, experimental: { readText: true } } };
const off = { smart: { enabled: true, experimental: { readText: false } } };
const lines = [{ ord: 0, text: "Gostilna", quad: [0, 0, 1, 0, 1, 1, 0, 1] }];

function wrapper({ children }: { children: ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

describe("useAssetText", () => {
  it("asks for nothing while reading text is off", async () => {
    mockCommand("get_settings", () => off);
    mockCommand("asset_text", () => lines);
    const { result } = renderHook(() => useAssetText(7, true), { wrapper });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_settings", undefined));
    expect(result.current.on).toBe(false);
    expect(result.current.lines).toEqual([]);
    expect(invokeMock.mock.calls.some(([c]) => c === "asset_text")).toBe(false);
  });

  it("returns the photo's lines while on, and says it is reading meanwhile", async () => {
    mockCommand("get_settings", () => on);
    let release: () => void = () => {};
    mockCommand("asset_text", () => new Promise((r) => (release = () => r(lines))));
    const { result } = renderHook(() => useAssetText(7, true), { wrapper });
    await waitFor(() => expect(result.current.reading).toBe(true));
    release();
    await waitFor(() => expect(result.current.lines).toEqual(lines));
    expect(result.current.reading).toBe(false);
    expect(invokeMock).toHaveBeenCalledWith("asset_text", { id: 7 });
  });

  it("asks for nothing for a video", async () => {
    mockCommand("get_settings", () => on);
    mockCommand("asset_text", () => lines);
    const { result } = renderHook(() => useAssetText(7, false), { wrapper });
    await waitFor(() => expect(result.current.on).toBe(true));
    expect(result.current.lines).toEqual([]);
    expect(invokeMock.mock.calls.some(([c]) => c === "asset_text")).toBe(false);
  });
});
