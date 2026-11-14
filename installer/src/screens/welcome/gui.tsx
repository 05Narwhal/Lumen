import { Button } from "@/components/controls";
import { Lead, Small } from "@/components/layout";
import { formatBytes } from "@/lib/format";
import "./styles.scss";

interface Props {
  version: string;
  sizeOnDisk: number;
  onInstall: () => void;
  onOptions: () => void;
}

export function Welcome({ version, sizeOnDisk, onInstall, onOptions }: Props) {
  return (
    <>
      <div className="welcome__wordmark">Lumen</div>
      <Lead>Your photos, on your machine, fast.</Lead>
      <div className="welcome__actions">
        <Button variant="primary" onClick={onInstall}>Install</Button>
        <Button onClick={onOptions}>Options…</Button>
      </div>
      <Small>
        Version <span>{version}</span> · {formatBytes(sizeOnDisk)} on this PC · no account, nothing leaves your computer
      </Small>
    </>
  );
}
