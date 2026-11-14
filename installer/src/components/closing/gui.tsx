import { Button } from "@/components/controls";
import { Small } from "@/components/layout";
import type { Closing } from "@/types";
import "./styles.scss";

interface Props {
  /** null or `closed`: nothing to say. */
  closing: Closing | null;
  /** Stops waiting and ends Setup; nothing is changed. */
  onCancel: () => void;
}

const TEXT: Record<Exclude<Closing, "closed">, string> = {
  closing: "Closing Lumen…",
  "asking-import": "Lumen is asking about the import that's running. Answer there and Setup continues.",
};

/** While Setup waits for Lumen to close (no time limit): what is happening, and Cancel. */
export function ClosingLine({ closing, onCancel }: Props) {
  if (!closing || closing === "closed") return null;
  return (
    <div className="closing">
      <div role="status" className="closing__text">
        <Small>{TEXT[closing]}</Small>
      </div>
      <Button variant="link" onClick={onCancel}>Cancel</Button>
    </div>
  );
}
