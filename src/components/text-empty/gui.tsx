import { Button } from "@mantine/core";
import { IconTextScan2 } from "@tabler/icons-react";
import { useNavigate } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { useSettings } from "@/hooks/use-settings";
import { jobsStore } from "@/lib/jobs-store";
import { readTextOn } from "@/lib/photo-text";
import "./styles.scss";

/**
 * A grid filtered by *Has text* with nothing in it (Round 13-3, spec §3). Reading text off:
 * the way to turn it on. The scan still running: how many photos it has left. Otherwise
 * nothing in these photos has words Lumen could read.
 */
export function TextEmptyState() {
  const { settings } = useSettings();
  const navigate = useNavigate();
  const job = jobsStore.use().jobs.find((j) => j.id === "ocr" && j.state === "running");
  const on = readTextOn(settings);
  const left = job ? Math.max(0, (job.total ?? 0) - (job.done ?? 0)) : 0;
  return (
    <div className="text-empty">
      <EmptyState
        icon={<IconTextScan2 />}
        title="No text found in these photos yet"
        description={
          !on
            ? "Lumen can read the words on signs, screenshots and documents, on this computer."
            : job
              ? `Still reading text · ${left.toLocaleString("en-US")} left`
              : "Lumen found no words in them."
        }
        action={!on ? <Button onClick={() => navigate("/settings?section=experimental")}>Turn on reading text</Button> : undefined}
      />
    </div>
  );
}
