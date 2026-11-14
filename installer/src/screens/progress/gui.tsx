import { Button, Meter } from "@/components/controls";
import { ClosingLine } from "@/components/closing";
import { Actions, Card, Heading, Small, Spacer } from "@/components/layout";
import { formatBytes, formatTimeLeft } from "@/lib/format";
import type { Closing, TaskState } from "@/types";
import "./styles.scss";

export interface TaskView {
  state: TaskState;
  done: number;
  total: number;
  secondsLeft?: number;
}

interface Props {
  heading: string;
  lumen: TaskView;
  /** null when smart features are off. */
  models: TaskView | null;
  error: string | null;
  onOpen: () => void;
  onSkip: () => void;
  onRetry: () => void;
  /** A running Lumen closing before the installer runs. */
  closing?: Closing | null;
  onCancel?: () => void;
}

const fraction = (t: TaskView) => (t.total > 0 ? Math.min(1, t.done / t.total) : undefined);

function Dot({ state }: { state: TaskState }) {
  return (
    <i className={`progress__dot progress__dot--${state}`} aria-hidden>
      {state === "done" ? "✓" : ""}
    </i>
  );
}

function modelsText(t: TaskView): string {
  switch (t.state) {
    case "waiting": return "waiting";
    case "done": return "done";
    case "later": return "later";
    case "failed": return "later";
    case "running": {
      const MB = 1024 ** 2;
      const total = formatBytes(t.total);
      const done = t.total >= 1024 ** 3 ? (t.done / 1024 ** 3).toFixed(1) : String(Math.round(t.done / MB));
      return `${done} of ${total}${t.secondsLeft !== undefined ? ` · ${formatTimeLeft(t.secondsLeft)}` : ""}`;
    }
  }
}

export function Progress({ heading, lumen, models, error, onOpen, onSkip, onRetry, closing = null, onCancel = () => {} }: Props) {
  const lumenDone = lumen.state === "done";
  // Nothing is installed while the open Lumen is still closing (or asking about an import).
  const waitingForLumen = closing === "closing" || closing === "asking-import";
  const downloading = models?.state === "running";
  return (
    <>
      <Heading>{heading}</Heading>
      <Card>
        <div className="progress__task">
          <Dot state={lumen.state} />
          <b>Lumen</b>
          <span className="progress__n">
            {lumen.state === "done" ? "done" : lumen.state === "failed" ? "stopped" : waitingForLumen ? "waiting for Lumen to close" : "installing"}
          </span>
          {lumen.state === "running" && !waitingForLumen && <Meter value={fraction(lumen)} label="Installing Lumen" />}
        </div>
        {models && (
          <div className="progress__task">
            <Dot state={models.state} />
            <b>Smart features</b>
            <span className="progress__n">{modelsText(models)}</span>
            {downloading && <Meter value={fraction(models)} label="Downloading smart features" />}
          </div>
        )}
      </Card>
      <ClosingLine closing={closing} onCancel={onCancel} />
      {error && <p role="alert" className="progress__error">{error}</p>}
      {lumenDone && downloading && (
        <Small>You can open Lumen now. Smart features keep downloading here, or Lumen finishes them.</Small>
      )}
      <Actions>
        {downloading && <Button variant="link" onClick={onSkip}>Skip smart features</Button>}
        <Spacer />
        {lumen.state === "failed" && <Button onClick={onRetry}>Try again</Button>}
        <Button variant="primary" disabled={!lumenDone} onClick={onOpen}>Open Lumen</Button>
      </Actions>
    </>
  );
}
