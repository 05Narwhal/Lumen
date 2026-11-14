import { listen } from "@tauri-apps/api/event";
import { invoke } from "@/lib/ipc";
import { createStore } from "@/lib/store";
import { toast } from "@/components/toast";
import type { RestartStep } from "@/types/generated/RestartStep";
import type { Stall } from "@/types/generated/Stall";
import type { StallReport } from "@/types/generated/StallReport";

/**
 * Freeze detection in both directions (Round 15). The page beats (`ui_heartbeat`) so the
 * watchdog can reload it when its script stops yielding; the page pings so it can tell the
 * person when the backend stops answering. A ping that fails still proves the backend answers;
 * only silence counts.
 */
export const STALL = "app://stall";
/** *Restart Lumen* stayed in place: `Recovered`, or `Relaunch` when a new Lumen could not start. */
export const RESTART = "app://restart";
const BEAT_MS = 2_000;
const PING_MS = 3_000;
const PING_TIMEOUT_MS = 5_000;
/** A window or database stall this long is no longer "busy" but "not responding". */
const SLOW_TO_STUCK_MS = 20_000;

/** After waking from sleep, silence means nothing for this long (the watchdog's `WAKE_GRACE`). */
const WAKE_GRACE_MS = 30_000;

/** The clock jumped far past one interval: the PC slept, so every timer's age includes the sleep. */
export function woke(prev: number, now: number, intervalMs: number): boolean {
  return now - prev > 5 * intervalMs;
}

export type Backend = "ok" | "slow" | "stuck";
export type Health = {
  backend: Backend;
  /** The watchdog's last report: everything past its limit. */
  stalls: Stall[];
  /** When the window or database stall began (ms since the epoch), for *busy (40 s)*. */
  busySince: number | null;
};

export const healthStore = createStore<Health>({ backend: "ok", stalls: [], busySince: null });
export const useHealth = () => healthStore.use();

/** Stalls that freeze what the person is doing; a long job alone is work. */
const blocking = (s: Stall[]) => s.filter((x) => x.kind === "MainThread" || x.kind === "Database");

export function startHealth(): () => void {
  let missed = 0;
  let stalls: Stall[] = [];
  let busySince: number | null = null;
  // When the page first heard of the stall: "slow" turns "stuck" 20 s after that.
  let slowAt: number | null = null;
  let upgrade: ReturnType<typeof setTimeout> | undefined;
  // Wall-clock time of the last timer callback, and when waking from sleep stops counting misses.
  let lastTick = Date.now();
  let graceUntil = 0;
  const checkWake = () => {
    const now = Date.now();
    if (woke(lastTick, now, BEAT_MS)) {
      graceUntil = now + WAKE_GRACE_MS;
      missed = 0;
    }
    lastTick = now;
  };

  const publish = () => {
    const busy = blocking(stalls).length > 0;
    const backend: Backend =
      missed >= 2 || (busy && slowAt !== null && Date.now() - slowAt >= SLOW_TO_STUCK_MS) ? "stuck" : busy ? "slow" : "ok";
    const s = healthStore.get();
    if (s.backend !== backend || s.stalls !== stalls || s.busySince !== busySince) healthStore.set({ backend, stalls, busySince });
  };

  const beat = () => {
    checkWake();
    void invoke("ui_heartbeat", { hidden: document.hidden }).catch(() => {});
  };
  const ping = () => {
    checkWake();
    let settled = false;
    const timer = setTimeout(() => {
      checkWake();
      if (settled) return;
      settled = true;
      if (Date.now() >= graceUntil) missed += 1;
      publish();
    }, PING_TIMEOUT_MS);
    const answered = () => {
      clearTimeout(timer);
      settled = true;
      missed = 0;
      publish();
    };
    invoke("ping").then(answered, answered);
  };

  beat();
  const beats = setInterval(beat, BEAT_MS);
  const pings = setInterval(ping, PING_MS);
  document.addEventListener("visibilitychange", beat);

  let alive = true;
  let unlisten: (() => void) | undefined;
  void listen<StallReport>(STALL, (e) => {
    stalls = e.payload.stalls;
    const busy = blocking(stalls);
    if (busy.length === 0) {
      busySince = null;
      slowAt = null;
      clearTimeout(upgrade);
    } else if (busySince === null) {
      slowAt = Date.now();
      busySince = slowAt - Math.max(...busy.map((x) => x.forMs));
      clearTimeout(upgrade);
      upgrade = setTimeout(publish, SLOW_TO_STUCK_MS);
    }
    publish();
  }).then((u) => (alive ? (unlisten = u) : u()));

  return () => {
    alive = false;
    clearInterval(beats);
    clearInterval(pings);
    clearTimeout(upgrade);
    document.removeEventListener("visibilitychange", beat);
    unlisten?.();
    healthStore.set({ backend: "ok", stalls: [], busySince: null });
  };
}

/** *Reload window*: the page only, at once; the backend and its work are untouched. */
export function reloadWindow() {
  window.location.reload();
}

let restarting = false;

/**
 * *Restart Lumen*, on the screen the person is on. Rust first tries to recover in place; only
 * if that fails does a new Lumen open here, so the page hears back only when Lumen stays.
 */
export async function restartLumen() {
  if (restarting) return;
  restarting = true;
  const un = await listen<RestartStep>(RESTART, (e) => {
    restarting = false;
    un();
    toast({
      message: e.payload === "Recovered" ? "Lumen is answering again." : "Lumen could not restart. Close it and open it again.",
    });
  });
  await invoke("restart_lumen", { location: window.location.pathname + window.location.search }).catch(() => {
    restarting = false;
    un();
  });
}
