import { Button } from "@mantine/core";
import { useEffect, useState } from "react";
import { restartLumen, useHealth } from "@/lib/health";
import type { Stall } from "@/types/generated/Stall";
import "./styles.scss";

/** What a person calls the thing that is stuck. */
const busyName = (s: Stall) => (s.kind === "MainThread" ? "the window" : s.kind === "Database" ? "the library database" : s.what);

/**
 * Says so when the backend stops answering (Round 15): *busy* while the watchdog sees a window
 * or database stall, *not responding* when pings go unanswered or a stall passes 20 s. Sits
 * in the shell above the content, never as a modal: the page itself still works.
 */
export function HealthBanner() {
  const { backend, stalls, busySince } = useHealth();
  const [waited, setWaited] = useState(false);
  const [, tick] = useState(0);

  // *Wait* lasts until the backend answers again; the next freeze shows the banner afresh.
  useEffect(() => {
    if (backend === "ok") setWaited(false);
  }, [backend]);
  useEffect(() => {
    if (backend !== "slow") return;
    const t = setInterval(() => tick((n) => n + 1), 1_000);
    return () => clearInterval(t);
  }, [backend]);

  if (backend === "ok" || waited) return null;
  if (backend === "slow") {
    const first = stalls.find((s) => s.kind !== "Job") ?? stalls[0];
    const secs = busySince === null ? 0 : Math.round((Date.now() - busySince) / 1_000);
    return (
      <div className="health-banner" data-state="slow" role="status">
        <span className="health-banner__text">
          Lumen is busy: {first ? busyName(first) : "working"} ({secs} s).
        </span>
      </div>
    );
  }
  return (
    <div className="health-banner" data-state="stuck" role="alert">
      <span className="health-banner__text">Lumen is not responding.</span>
      <Button size="compact-sm" variant="default" onClick={() => setWaited(true)}>
        Wait
      </Button>
      <Button size="compact-sm" onClick={() => void restartLumen()}>
        Restart Lumen
      </Button>
    </div>
  );
}
