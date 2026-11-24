import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ThemeChoice } from "@/types/generated/ThemeChoice";
import { createThemeTry, TRY_MS, type TryDeps } from "./theme-try";

function setup(current: ThemeChoice = "gallery") {
  const log: string[] = [];
  let choice: ThemeChoice = current;
  const deps: TryDeps = {
    tryTheme: vi.fn(async (id: string) => {
      log.push(`try:${id}`);
      return { id: `studio.${id}`, name: id };
    }),
    apply: vi.fn(async (id: string) => {
      log.push(`apply:${id}`);
      choice = { plugin: id, theme: "theme" };
    }),
    end: vi.fn(async () => void log.push("end")),
    keep: vi.fn(async () => void log.push("keep")),
    current: () => choice,
    restore: vi.fn((c: ThemeChoice) => {
      log.push("restore");
      choice = c;
    }),
  };
  const seen: (string | null)[] = [];
  const ctl = createThemeTry(deps, (s) => seen.push(s?.storeId ?? null));
  return { ctl, deps, log, seen, choice: () => choice };
}

describe("theme try", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("reverts to the previous theme after 30 seconds, not before", async () => {
    const t = setup("paper");
    await t.ctl.start("nocturne");
    await vi.advanceTimersByTimeAsync(TRY_MS - 1);
    expect(t.deps.end).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2);
    expect(t.deps.restore).toHaveBeenCalledWith("paper");
    expect(t.deps.end).toHaveBeenCalledTimes(1);
    expect(t.choice()).toBe("paper");
    expect(t.ctl.state()).toBeNull();
  });

  it("restores the theme before ending the try, so nothing flashes the default", async () => {
    const t = setup();
    await t.ctl.start("nocturne");
    await t.ctl.end();
    expect(t.log).toEqual(["try:nocturne", "apply:studio.nocturne", "restore", "end"]);
  });

  it("keep installs the theme, stops the timer and never reverts", async () => {
    const t = setup("paper");
    await t.ctl.start("nocturne");
    await t.ctl.keep();
    await vi.advanceTimersByTimeAsync(TRY_MS * 2);
    expect(t.deps.keep).toHaveBeenCalledTimes(1);
    expect(t.deps.end).not.toHaveBeenCalled();
    expect(t.deps.restore).not.toHaveBeenCalled();
    expect(t.choice()).toEqual({ plugin: "studio.nocturne", theme: "theme" });
    expect(t.ctl.state()).toBeNull();
  });

  it("trying a second theme keeps the first one's previous theme and restarts the 30 seconds", async () => {
    const t = setup("paper");
    await t.ctl.start("nocturne");
    await vi.advanceTimersByTimeAsync(20_000);
    await t.ctl.start("coastline");
    await vi.advanceTimersByTimeAsync(20_000);
    expect(t.deps.end).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(10_001);
    expect(t.deps.restore).toHaveBeenCalledWith("paper");
    expect(t.deps.restore).toHaveBeenCalledTimes(1);
  });

  it("a try that fails leaves the theme alone and starts no timer", async () => {
    const t = setup("paper");
    (t.deps.tryTheme as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("offline"));
    await expect(t.ctl.start("nocturne")).rejects.toThrow("offline");
    await vi.advanceTimersByTimeAsync(TRY_MS * 2);
    expect(t.deps.restore).not.toHaveBeenCalled();
    expect(t.deps.end).not.toHaveBeenCalled();
    expect(t.ctl.state()).toBeNull();
  });

  it("end with nothing being tried does nothing", async () => {
    const t = setup();
    await t.ctl.end();
    expect(t.deps.end).not.toHaveBeenCalled();
  });
});
