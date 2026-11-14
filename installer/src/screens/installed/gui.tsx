import { Button } from "@/components/controls";
import { Card, Heading, Lead, Small } from "@/components/layout";
import type { Installed as InstalledState } from "@/types";
import "./styles.scss";

type Present = Exclude<InstalledState, { kind: "none" }>;

interface Props {
  installed: Present;
  ours: string;
  lumenRunning: boolean;
  /** Update or Repair. */
  onPrimary: () => void;
  onOpen: () => void;
  onRemove: () => void;
}

export function Installed({ installed, ours, lumenRunning, onPrimary, onOpen, onRemove }: Props) {
  const newer = installed.kind === "newer";
  const verb = installed.kind === "older" ? "Update" : "Repair";
  return (
    <>
      <Heading>{newer ? "This PC already has a newer Lumen" : "Lumen is already installed"}</Heading>
      <Lead>
        This PC has <span className="installed__ver">{installed.version}</span>. This setup has <span className="installed__ver">{ours}</span>.
        {newer && " Setup never installs an older version over a newer one."}
      </Lead>
      <Card>
        {newer ? (
          <div className="s-row">
            <b>Open Lumen</b>
            <Button variant="primary" onClick={onOpen}>Open Lumen</Button>
            <span className="s-row__detail">Your newer Lumen is untouched.</span>
          </div>
        ) : (
          <div className="s-row">
            <b>{installed.kind === "older" ? `Update to ${ours}` : "Repair this install"}</b>
            <Button variant="primary" onClick={onPrimary}>{verb}</Button>
            <span className="s-row__detail">
              {installed.kind === "older" ? "Keeps your settings, libraries and shortcuts." : "Puts Lumen's files back. Keeps your settings, libraries and shortcuts."}
            </span>
          </div>
        )}
        <div className="s-row">
          <b>Remove Lumen</b>
          <Button onClick={onRemove}>Remove…</Button>
          <span className="s-row__detail">Your photos and libraries stay.</span>
        </div>
      </Card>
      {lumenRunning && !newer && (
        <Small>Lumen is open. It closes itself before the {installed.kind === "older" ? "update" : "repair"} and reopens after.</Small>
      )}
    </>
  );
}
