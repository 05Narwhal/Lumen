import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore, lastDevelopPhoto } from "@/lib/develop-source";
import { hydrateDevelopSource, rememberDevelopSource } from "@/lib/develop-source-persist";
import { FLUSH_REQUESTED } from "@/lib/events";
import { emitMock, mockCommand } from "@/test/tauri-mock";

const SELECTION = { ...DEFAULT_DEVELOP_SOURCE, only: [4, 2, 9], label: "Selection", sort: "rating" as const };

describe("the remembered Develop filmstrip", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    developSourceStore.set(DEFAULT_DEVELOP_SOURCE);
    lastDevelopPhoto.set(null);
  });
  afterEach(() => vi.useRealTimers());

  it("hydrates both stores from what was stored", async () => {
    mockCommand("get_develop_source", () => ({ source: SELECTION, last: 2 }));
    await hydrateDevelopSource();
    expect(developSourceStore.get()).toEqual(SELECTION);
    expect(lastDevelopPhoto.get()).toBe(2);
  });

  it("leaves Develop empty when nothing is stored or the read fails", async () => {
    mockCommand("get_develop_source", () => ({ source: null, last: null }));
    await hydrateDevelopSource();
    expect(developSourceStore.get()).toEqual(DEFAULT_DEVELOP_SOURCE);
    mockCommand("get_develop_source", () => {
      throw new Error("no library");
    });
    await expect(hydrateDevelopSource()).resolves.toBeUndefined();
    expect(developSourceStore.get()).toEqual(DEFAULT_DEVELOP_SOURCE);
  });

  it("writes a change once, after the debounce, and at once when a flush is asked for", async () => {
    const writes: unknown[] = [];
    mockCommand("set_develop_source", (args) => {
      writes.push(args);
    });
    const stop = rememberDevelopSource();
    developSourceStore.set(SELECTION);
    lastDevelopPhoto.set(4);
    await vi.advanceTimersByTimeAsync(400);
    expect(writes).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(200);
    expect(writes).toEqual([{ source: SELECTION, last: 4 }]);

    lastDevelopPhoto.set(9);
    emitMock(FLUSH_REQUESTED, false);
    await vi.advanceTimersByTimeAsync(0);
    expect(writes).toHaveLength(2);
    expect(writes[1]).toEqual({ source: SELECTION, last: 9 });
    await vi.advanceTimersByTimeAsync(1000);
    expect(writes, "nothing changed since the flush").toHaveLength(2);
    stop();
  });
});
