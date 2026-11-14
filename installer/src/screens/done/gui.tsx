import { Button, CheckRow } from "@/components/controls";
import { Heading, Lead } from "@/components/layout";
import type { ModelsOutcome } from "@/types";
import "./styles.scss";

interface Props {
  models: ModelsOutcome;
  openNow: boolean;
  onToggleOpen: (v: boolean) => void;
  onFinish: () => void;
}

export function Done({ models, openNow, onToggleOpen, onFinish }: Props) {
  return (
    <>
      <Heading>Lumen is ready</Heading>
      <Lead>
        {models === "complete"
          ? "Everything is installed."
          : "Smart features will finish downloading the next time Lumen is open. Nothing else is needed."}
      </Lead>
      <div className="done__check">
        <CheckRow label="Open Lumen now" checked={openNow} onChange={onToggleOpen} />
      </div>
      <Button variant="primary" onClick={onFinish}>Finish</Button>
    </>
  );
}
