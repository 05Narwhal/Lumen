import { Popover } from "@mantine/core";
import { jobsStore, ringProgress, selectRunning, type Job } from "@/lib/jobs-store";
import "./styles.scss";

/** *41 of 322*, or the job's own detail (a file, the time left). */
function jobDetail(j: Job): string | undefined {
  if (j.detail) return j.detail;
  if (j.total && j.total > 1) return `${Math.round(j.done ?? 0).toLocaleString("en-US")} of ${Math.round(j.total).toLocaleString("en-US")}`;
  return undefined;
}

/**
 * The only place background work is surfaced. One arc for everything (all that is done over
 * all there is, Round 9B), easing toward each new value and never starting over while work
 * runs; a finished line holds at full a moment before it goes.
 */
export function ActivityRing({ compact = false, placement = "sidebar" }: { compact?: boolean; placement?: "sidebar" | "titlebar" }) {
  const shown = selectRunning(jobsStore.use());
  const active = shown.length > 0;
  const progress = ringProgress(shown);
  const r = 8;
  const c = 2 * Math.PI * r;
  const lead = shown.find((j) => j.state === "running") ?? shown[0];
  return (
    <Popover position={placement === "titlebar" ? "bottom-end" : "right-end"} withArrow shadow="md" width={280}>
      <Popover.Target>
        <button
          type="button"
          className="activity-ring"
          data-placement={placement}
          aria-label={active ? `${lead.label}, ${Math.round(progress * 100)}% done` : "Nothing running"}
          data-active={active}
        >
          <svg width="20" height="20" viewBox="0 0 20 20" aria-hidden>
            <circle cx="10" cy="10" r={r} className="activity-ring__track" />
            <circle
              cx="10"
              cy="10"
              r={r}
              className="activity-ring__fill"
              strokeDasharray={c}
              strokeDashoffset={c * (1 - (active ? progress : 0))}
              data-testid="activity-ring-fill"
            />
          </svg>
          {!compact && (
            // Name the work rather than counting it: "Looking for people" tells you what the
            // app is doing, "2 running" does not.
            <span className="activity-ring__label">{active ? lead.label : "Idle"}</span>
          )}
        </button>
      </Popover.Target>
      <Popover.Dropdown className="activity-ring__dropdown">
        {!active ? (
          <p className="activity-ring__empty">Nothing running</p>
        ) : (
          shown.map((j) => {
            const detail = jobDetail(j);
            return (
              <div key={j.id} className="activity-ring__job" data-state={j.state}>
                <span className="activity-ring__job-label">{j.label}</span>
                <span className="activity-ring__bar" aria-hidden>
                  <span className="activity-ring__bar-fill" style={{ transform: `scaleX(${Math.min(1, Math.max(0, j.progress))})` }} />
                </span>
                {detail && <span className="activity-ring__job-detail">{detail}</span>}
              </div>
            );
          })
        )}
      </Popover.Dropdown>
    </Popover>
  );
}
