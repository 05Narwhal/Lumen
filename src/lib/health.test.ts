import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitMock, invokeMock, mockCommand } from "@/test/tauri-mock";
import { toastStore } from "@/components/toast/store";
import { healthStore, RESTART, restartLumen, startHealth, STALL, woke } from "./health";

describe("health", () => {
  let stop: () => void = () => {};
  beforeEach(() => {
    vi.useFakeTimers();
    mockCommand("ui_heartbeat", () => null);
  });
  afterEach(() => {
    stop();
    vi.useRealTimers();
  });

  it("beats every 2 s and at once when the page is hidden or shown", async () => {
    stop = startHealth();
    await vi.advanceTimersByTimeAsync(4_000);
    const beats = () => invokeMock.mock.calls.filter(([c]) => c === "ui_heartbeat").length;
    expect(beats()).toBe(3);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(beats()).toBe(4);
  });

  it("two pings that never answer mean stuck, and one answer brings back ok", async () => {
    let answer = false;
    mockCommand("ping", () => (answer ? 1 : new Promise(() => {})));
    stop = startHealth();
    await vi.advanceTimersByTimeAsync(9_000); // pings at 3 s and 6 s, the first times out at 8 s
    expect(healthStore.get().backend).toBe("ok");
    await vi.advanceTimersByTimeAsync(2_000); // the second times out at 11 s
    expect(healthStore.get().backend).toBe("stuck");
    answer = true;
    await vi.advanceTimersByTimeAsync(3_000);
    expect(healthStore.get().backend).toBe("ok");
  });

  it("a clock gap far past the interval is a wake", () => {
    expect(woke(1_000, 2_000, 2_000)).toBe(false);
    expect(woke(1_000, 9_000, 2_000)).toBe(false); // a busy page, not a sleep
    expect(woke(1_000, 12_000, 2_000)).toBe(true);
    expect(woke(1_000, 1_000 + 2 * 3_600_000, 2_000)).toBe(true);
  });

  it("waking from sleep does not flash the stuck banner, a real stall after the grace still does", async () => {
    mockCommand("ping", () => new Promise(() => {}));
    stop = startHealth();
    await vi.advanceTimersByTimeAsync(7_000); // two pings in flight, none timed out yet
    vi.setSystemTime(Date.now() + 2 * 3_600_000); // the PC slept for two hours
    await vi.advanceTimersByTimeAsync(5_000); // their timeouts fire, each "5 s" long over the sleep
    expect(healthStore.get().backend).toBe("ok");
    await vi.advanceTimersByTimeAsync(20_000); // still inside the 30 s grace
    expect(healthStore.get().backend).toBe("ok");
    await vi.advanceTimersByTimeAsync(20_000); // the grace is over; unanswered pings count again
    expect(healthStore.get().backend).toBe("stuck");
  });

  it("a ping that fails still proves the backend answers", async () => {
    mockCommand("ping", () => Promise.reject({ code: "x", message: "x" }));
    stop = startHealth();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(healthStore.get().backend).toBe("ok");
  });

  it("a main-thread stall is slow, then stuck 20 s later", async () => {
    mockCommand("ping", () => 1);
    stop = startHealth();
    await vi.advanceTimersByTimeAsync(0);
    emitMock(STALL, { stalls: [{ what: "main thread", kind: "MainThread", forMs: 4_000 }] });
    expect(healthStore.get().backend).toBe("slow");
    await vi.advanceTimersByTimeAsync(19_000);
    expect(healthStore.get().backend).toBe("slow");
    await vi.advanceTimersByTimeAsync(1_000);
    expect(healthStore.get().backend).toBe("stuck");
    emitMock(STALL, { stalls: [] });
    expect(healthStore.get().backend).toBe("ok");
  });

  it("a long job alone is not slow", async () => {
    mockCommand("ping", () => 1);
    stop = startHealth();
    await vi.advanceTimersByTimeAsync(0);
    emitMock(STALL, { stalls: [{ what: "job thumb:1", kind: "Job", forMs: 16_000 }] });
    expect(healthStore.get().backend).toBe("ok");
    expect(healthStore.get().stalls).toHaveLength(1);
  });

  it("Restart Lumen asks for the screen in view and says so when Lumen recovered in place", async () => {
    mockCommand("restart_lumen", () => null);
    window.history.replaceState(null, "", "/develop/12?panel=detail");
    await restartLumen();
    expect(invokeMock).toHaveBeenCalledWith("restart_lumen", { location: "/develop/12?panel=detail" });
    emitMock(RESTART, "Recovered");
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Lumen is answering again.");
    emitMock(RESTART, "Relaunch");
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Lumen is answering again.");
  });

  it("says so when a new Lumen could not be started", async () => {
    mockCommand("restart_lumen", () => null);
    await restartLumen();
    emitMock(RESTART, "Relaunch");
    expect(toastStore.get().toasts.at(-1)?.message).toBe("Lumen could not restart. Close it and open it again.");
  });
});
