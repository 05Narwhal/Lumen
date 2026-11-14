import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { toastStore } from "@/components/toast/store";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { clearFavoriteOverrides, favoriteOverrides, useFavorites } from "./use-favorites";

function wrapper({ children }: { children: ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

let api: ReturnType<typeof useFavorites>;
function Probe() {
  api = useFavorites();
  return null;
}

const runUndo = () => {
  const t = toastStore.get().toasts.at(-1);
  t?.action?.run();
};

describe("useFavorites", () => {
  beforeEach(() => {
    mockLibraryDefaults();
    mockCommand("set_favorite", () => null);
    clearFavoriteOverrides();
    toastStore.set({ toasts: [] });
  });

  it("undo puts the tile's star back, not just the database", async () => {
    render(<Probe />, { wrapper });
    await act(async () => {
      await api.setFavorite([7], true);
    });
    expect(favoriteOverrides.get().map.has(7)).toBe(false); // handed over to the catalogue

    // The optimistic override is what the tile's star reads. Undo that only wrote to the
    // backend left the override saying "favorite", so the badge stayed lit.
    await act(async () => {
      runUndo();
      // Synchronously, before the round trip: the star must go out at once.
      expect(favoriteOverrides.get().map.get(7)).toBe(false);
      await Promise.resolve();
    });
    // And once the catalogue has caught up the override hands back to it entirely,
    // rather than pinning the tile for the rest of the session.
    expect(favoriteOverrides.get().map.has(7)).toBe(false);
  });

  it("rolls the override back when the command fails", async () => {
    mockCommand("set_favorite", () => {
      throw { code: "io", message: "nope" };
    });
    render(<Probe />, { wrapper });
    await act(async () => {
      await api.setFavorite([9], true);
    });
    expect(favoriteOverrides.get().map.has(9)).toBe(false);
  });
});
